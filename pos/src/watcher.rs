//! Watches the payment rails of sales and moves sales to their next state.
//!
//! A sale is paid by the first rail that delivers its full amount. The
//! watcher keeps checking the other rails, so a second payment is recorded
//! as excess instead of being missed:
//!
//! - A Lightning payment is checked until barkd reports it delivered or
//!   gone, however long posd was down. Its invoice expires with the sale, so
//!   a delivered payment was made in time and pays an expired sale too.
//! - An on-chain address is checked often while its sale is live, and
//!   rarely after, until the watch window ends, for late and extra payments.
//!   An `in-progress` sale stays live until its payment confirms or leaves
//!   the mempool, however long that takes.

use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context;
use bitcoin::{Address, Amount};
use futures::StreamExt;

use crate::App;
use crate::database::{Rail, SaleState, WatchedPayment};
use crate::rails::forward::ForwardStatus;


/// Live sales' on-chain addresses are checked every this many ticks.
const LIVE_ONCHAIN_EVERY_TICKS: u64 = 5;
/// Final sales' on-chain addresses are checked every this many ticks.
const FINAL_ONCHAIN_EVERY_TICKS: u64 = 150;
/// How many payments are checked at once.
const CONCURRENT_CHECKS: usize = 8;

/// What a check found for one payment rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
	/// Delivered to the owner. For on-chain, confirmed.
	pub received: Amount,
	/// Seen but not delivered. Only on-chain reports this.
	pub pending: Amount,
	/// Whether the rail is done: delivered, or certain never to pay.
	pub settled: bool,
	pub txids: Option<Vec<String>>,
}

/// Which payments a pass checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pass {
	pub live_onchain: bool,
	pub final_onchain: bool,
}

impl Pass {
	fn at(tick: u64) -> Pass {
		Pass {
			live_onchain: tick % LIVE_ONCHAIN_EVERY_TICKS == 0,
			final_onchain: tick % FINAL_ONCHAIN_EVERY_TICKS == 0,
		}
	}
}

pub async fn run(app: Arc<App>) {
	let mut tick = 0u64;
	loop {
		if let Err(e) = run_once(&app, Pass::at(tick)).await {
			tracing::warn!("watcher pass failed: {:#}", e);
		}
		tick = tick.wrapping_add(1);
		tokio::time::sleep(app.cfg.sales.poll_interval()).await;
	}
}

/// One pass: Lightning payments always, on-chain ones as `pass` says, then
/// expiry.
///
/// A sale expires only on a pass that checked its on-chain address, and
/// only if that check worked: a payment made just before expiry must be
/// seen before the sale ends.
pub async fn run_once(app: &App, pass: Pass) -> anyhow::Result<()> {
	let payments = watched_payments(app, pass).await?;
	// Terminal sales come first in the list, so a backlog of Lightning
	// address invoices does not hold them up.
	let results = futures::stream::iter(payments)
		.map(|p| async move {
			let result = match observe(app, &p).await {
				Ok(Some(obs)) => record(app, &p, &obs).await,
				Ok(None) => Ok(false),
				Err(e) => Err(e),
			};
			(p, result)
		})
		.buffered(CONCURRENT_CHECKS)
		.collect::<Vec<_>>().await;
	let mut changed = false;
	let mut unchecked = Vec::new();
	for (p, result) in results {
		match result {
			Ok(c) => changed |= c,
			Err(e) => {
				tracing::warn!("check of sale {} ({}) failed: {:#}", p.sale_public_id, p.rail.as_str(), e);
				if p.rail == Rail::Onchain {
					unchecked.push(p.sale_id);
				}
			},
		}
	}
	if pass.live_onchain {
		changed |= expire(app, &unchecked).await?;
	}
	if changed {
		app.notify_sale_change();
	}
	Ok(())
}

async fn watched_payments(app: &App, pass: Pass) -> anyhow::Result<Vec<WatchedPayment>> {
	let conn = app.db.conn().await?;
	let rows = conn.query("
		SELECT p.id AS payment_id, s.id AS sale_id, s.public_id, p.rail, p.payment_hash,
			p.onchain_address, p.received_sat, p.pending_sat
		FROM sale_payment p
		JOIN sale s ON s.id = p.sale_id
		WHERE (p.rail = 'lightning' AND p.settled_at IS NULL
				AND s.expires_at > NOW() - make_interval(secs => $1))
			OR (p.rail = 'onchain' AND $2 AND s.state IN ('open', 'in-progress'))
			OR (p.rail = 'onchain' AND $3 AND s.state IN ('paid', 'expired', 'canceled')
				AND s.expires_at > NOW() - make_interval(secs => $1))
		ORDER BY s.device_id IS NULL, p.id
	", &[&(app.cfg.sales.watch_secs as f64), &pass.live_onchain, &pass.final_onchain]).await?;
	Ok(rows.iter().map(WatchedPayment::from_row).collect())
}

/// Checks one rail. `None` means nothing new.
async fn observe(app: &App, p: &WatchedPayment) -> anyhow::Result<Option<Observation>> {
	match p.rail {
		Rail::Lightning => {
			let hash = p.payment_hash.as_deref().context("Lightning payment without hash")?;
			Ok(match app.forwarder.status(hash).await? {
				ForwardStatus::AwaitingPayment | ForwardStatus::Arriving => None,
				ForwardStatus::Delivered(amount) => Some(Observation {
					received: amount, pending: Amount::ZERO, settled: true, txids: None,
				}),
				// Canceled or expired unpaid: stop checking it.
				ForwardStatus::Gone => Some(Observation {
					received: Amount::ZERO, pending: Amount::ZERO, settled: true, txids: None,
				}),
			})
		},
		Rail::Onchain => {
			let address = p.onchain_address.as_deref().context("on-chain payment without address")?;
			let address = Address::from_str(address)?.require_network(app.cfg.network)?;
			let funds = app.chain.funds(&address).await?;
			let unchanged = funds.confirmed.to_sat() as i64 == p.received_sat
				&& funds.pending.to_sat() as i64 == p.pending_sat;
			if unchanged {
				return Ok(None);
			}
			Ok(Some(Observation {
				received: funds.confirmed,
				pending: funds.pending,
				// An address stays watched for late or extra payments until
				// its watch window ends.
				settled: false,
				txids: Some(app.chain.txids(&address).await?),
			}))
		},
	}
}

/// The state a sale moves to after an observation, if it changes.
pub fn next_state(
	state: SaleState,
	amount_sat: i64,
	rail: Rail,
	obs: &Observation,
	expired: bool,
) -> Option<SaleState> {
	let received = obs.received.to_sat() as i64;
	let pending = obs.pending.to_sat() as i64;
	match rail {
		Rail::Lightning => {
			// The invoice expired with the sale, so a payment it delivers
			// was made in time.
			let payable = state.is_live() || state == SaleState::Expired;
			(payable && obs.settled && received >= amount_sat).then_some(SaleState::Paid)
		},
		Rail::Onchain => {
			if !state.is_live() {
				return None;
			}
			if received >= amount_sat {
				Some(SaleState::Paid)
			} else if received + pending >= amount_sat {
				(state == SaleState::Open).then_some(SaleState::InProgress)
			} else if state == SaleState::InProgress {
				// The mempool payment went away.
				Some(if expired { SaleState::Expired } else { SaleState::Open })
			} else {
				None
			}
		},
	}
}

/// Stores an observation and moves the sale on. Returns whether a sale
/// changed state.
async fn record(app: &App, p: &WatchedPayment, obs: &Observation) -> anyhow::Result<bool> {
	let mut conn = app.db.conn().await?;
	let tx = conn.transaction().await?;

	// Lock the sale, so two payments cannot both mark it paid.
	let row = tx.query_one(
		"SELECT state, amount_sat, expires_at < NOW() AS expired FROM sale WHERE id = $1 FOR UPDATE",
		&[&p.sale_id],
	).await?;
	let state: SaleState = row.get("state");
	let amount_sat: i64 = row.get("amount_sat");
	let expired: bool = row.get("expired");

	tx.execute("
		UPDATE sale_payment SET received_sat = $2, pending_sat = $3,
			settled_at = CASE WHEN $4 THEN NOW() END,
			txids = coalesce($5, txids)
		WHERE id = $1
	", &[
		&p.payment_id, &(obs.received.to_sat() as i64), &(obs.pending.to_sat() as i64),
		&obs.settled, &obs.txids,
	]).await?;
	tx.execute("
		UPDATE sale SET received_sat = (
			SELECT coalesce(sum(received_sat), 0) FROM sale_payment WHERE sale_id = $1
		), updated_at = NOW() WHERE id = $1
	", &[&p.sale_id]).await?;

	let next = next_state(state, amount_sat, p.rail, obs, expired);
	if let Some(next) = next {
		let paid = next == SaleState::Paid;
		tx.execute("
			UPDATE sale SET state = $2,
				paid_rail = CASE WHEN $3 THEN $4 ELSE paid_rail END,
				paid_at = CASE WHEN $3 THEN NOW() ELSE paid_at END
			WHERE id = $1
		", &[&p.sale_id, &next, &paid, &p.rail]).await?;
	}
	tx.commit().await?;

	if next == Some(SaleState::Paid) {
		tracing::info!("sale {} paid by {}", p.sale_public_id, p.rail.as_str());
		if p.rail != Rail::Lightning {
			cancel_forward_invoice(app, p.sale_id).await;
		}
	}
	Ok(next.is_some())
}

/// Cancels the forwarding invoice of a sale paid by another rail, so a
/// second payment is less likely. It is best effort: a payment already in
/// flight still completes and counts as excess.
async fn cancel_forward_invoice(app: &App, sale_id: i64) {
	let payments = match unsettled_payments(app, sale_id).await {
		Ok(p) => p,
		Err(e) => {
			tracing::warn!("failed to load the invoice of sale {}: {:#}", sale_id, e);
			return;
		},
	};
	for p in payments.iter().filter(|p| p.rail == Rail::Lightning) {
		let Some(hash) = &p.payment_hash else { continue };
		match app.forwarder.cancel(hash).await {
			Ok(()) => if let Err(e) = settle_canceled(app, p.payment_id).await {
				tracing::warn!("failed to record the canceled invoice {}: {:#}", hash, e);
			},
			Err(e) => tracing::info!("forwarding invoice {} stays open: {:#}", hash, e),
		}
	}
}

/// The payments of a sale the watcher still checks.
pub async fn unsettled_payments(app: &App, sale_id: i64) -> anyhow::Result<Vec<WatchedPayment>> {
	let conn = app.db.conn().await?;
	let rows = conn.query("
		SELECT p.id AS payment_id, s.id AS sale_id, s.public_id, p.rail, p.payment_hash,
			p.onchain_address, p.received_sat, p.pending_sat
		FROM sale_payment p JOIN sale s ON s.id = p.sale_id
		WHERE p.sale_id = $1 AND p.settled_at IS NULL
		ORDER BY p.id
	", &[&sale_id]).await?;
	Ok(rows.iter().map(WatchedPayment::from_row).collect())
}

/// Records that barkd canceled an invoice: it can no longer pay, so the
/// watcher stops checking it.
pub async fn settle_canceled(app: &App, payment_id: i64) -> anyhow::Result<()> {
	let conn = app.db.conn().await?;
	conn.execute(
		"UPDATE sale_payment SET settled_at = NOW() WHERE id = $1 AND settled_at IS NULL", &[&payment_id],
	).await?;
	Ok(())
}

/// Checks one payment now and records what it finds. Returns whether the
/// sale changed state.
pub async fn check_now(app: &App, p: &WatchedPayment) -> anyhow::Result<bool> {
	match observe(app, p).await? {
		Some(obs) => record(app, p, &obs).await,
		None => Ok(false),
	}
}

/// Expires open sales past their expiry, except `unchecked` ones, whose
/// on-chain check failed. An `in-progress` sale stays open for its mempool
/// payment.
async fn expire(app: &App, unchecked: &[i64]) -> anyhow::Result<bool> {
	let conn = app.db.conn().await?;
	let n = conn.execute("
		UPDATE sale SET state = 'expired', updated_at = NOW()
		WHERE state = 'open' AND expires_at < NOW() AND NOT (id = ANY($1))
	", &[&unchecked]).await?;
	Ok(n > 0)
}

#[cfg(test)]
mod test {
	use super::*;

	fn obs(received: u64, pending: u64, settled: bool) -> Observation {
		Observation {
			received: Amount::from_sat(received),
			pending: Amount::from_sat(pending),
			settled,
			txids: None,
		}
	}

	#[test]
	fn state_transitions() {
		use SaleState::*;
		// Lightning pays on delivery of the full amount.
		assert_eq!(next_state(Open, 1000, Rail::Lightning, &obs(1000, 0, true), false), Some(Paid));
		assert_eq!(next_state(Open, 1000, Rail::Lightning, &obs(1200, 0, true), false), Some(Paid));
		assert_eq!(next_state(Open, 1000, Rail::Lightning, &obs(0, 0, true), false), None);
		// Its invoice expired with the sale, so a delivery seen after expiry,
		// as after a posd outage, still pays it.
		assert_eq!(next_state(Expired, 1000, Rail::Lightning, &obs(1000, 0, true), true), Some(Paid));
		// Paid and canceled sales stay so: the payment counts as excess.
		assert_eq!(next_state(Paid, 1000, Rail::Lightning, &obs(1000, 0, true), false), None);
		assert_eq!(next_state(Canceled, 1000, Rail::Lightning, &obs(1000, 0, true), false), None);
		// A late on-chain payment to an expired sale counts as excess.
		assert_eq!(next_state(Expired, 1000, Rail::Onchain, &obs(1000, 0, false), true), None);

		// On-chain: mempool, then 1 confirmation.
		assert_eq!(next_state(Open, 1000, Rail::Onchain, &obs(0, 1000, false), false), Some(InProgress));
		assert_eq!(next_state(InProgress, 1000, Rail::Onchain, &obs(0, 1000, false), true), None);
		assert_eq!(next_state(InProgress, 1000, Rail::Onchain, &obs(1000, 0, false), true), Some(Paid));
		// Underpaid on-chain stays open.
		assert_eq!(next_state(Open, 1000, Rail::Onchain, &obs(0, 500, false), false), None);
		// The mempool payment disappears.
		assert_eq!(next_state(InProgress, 1000, Rail::Onchain, &obs(0, 0, false), false), Some(Open));
		assert_eq!(next_state(InProgress, 1000, Rail::Onchain, &obs(0, 0, false), true), Some(Expired));
	}
}
