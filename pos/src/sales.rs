//! Creating sales, and how a sale looks to the API.

use std::str::FromStr;

use anyhow::Context;
use bitcoin::{Amount, Denomination};
use chrono::{DateTime, Utc};
use serde::Serialize;
use tokio_postgres::GenericClient;

use ark::fees::LightningReceiveFees;

use crate::App;
use crate::database::{Rail, SALE_SELECT, Sale, SaleState, Store};
use crate::error::{ApiError, ApiResult, bad_request, conflict, not_found};
use crate::money::{Decimal, MAX_SAT};
use crate::rails::onchain::StoreDescriptor;
use crate::token;


pub const MAX_MEMO_CHARS: usize = 200;

/// The most derivation indexes a sale skips past addresses that are in use,
/// before it goes without an on-chain address.
const MAX_ADDRESS_SKIPS: usize = 50;

/// The smallest amount barkd makes a Lightning invoice for under the Ark
/// server's Lightning receive fee: it refuses one that leaves nothing after
/// the fee. `None` if no amount does.
pub fn min_lightning_amount(fees: &LightningReceiveFees) -> Option<Amount> {
	// What is left after the fee never shrinks as the amount grows, so a
	// binary search finds the smallest amount above its own fee.
	let pays = |sat: u64| fees.calculate(Amount::from_sat(sat)).is_some_and(|fee| fee.to_sat() < sat);
	if !pays(MAX_SAT) {
		return None;
	}
	let (mut low, mut high) = (0, MAX_SAT);
	while high - low > 1 {
		let mid = low + (high - low) / 2;
		if pays(mid) { high = mid } else { low = mid }
	}
	Some(Amount::from_sat(high))
}

/// The smallest amount of a sale: every sale has a Lightning invoice.
pub async fn min_sale_amount(app: &App) -> ApiResult<Amount> {
	let fees = app.ark.ark_info().await.fees.lightning_receive;
	min_lightning_amount(&fees)
		.ok_or_else(|| ApiError::Upstream(anyhow::anyhow!("the Ark server's Lightning receive fee leaves nothing")))
}

/// The amount a sale is created for.
#[derive(Debug, Clone)]
pub enum SaleAmount {
	Fiat(Decimal),
	Sat(Amount),
}

/// A terminal's name for a sale it asks for, with the request as sent, so a
/// repeated request returns the same sale.
pub struct SaleRequest {
	pub key: String,
	pub fingerprint: String,
}

/// The sale a device already made for `request`, if any. A key reused for a
/// different request is refused.
pub async fn sale_for_request(
	conn: &impl GenericClient,
	device_id: i64,
	request: &SaleRequest,
) -> ApiResult<Option<Sale>> {
	let row = conn.query_opt(
		"SELECT id, request_fingerprint FROM sale WHERE device_id = $1 AND request_key = $2",
		&[&device_id, &request.key],
	).await?;
	let Some(row) = row else { return Ok(None) };
	if row.get::<_, Option<String>>("request_fingerprint").as_deref() != Some(&request.fingerprint) {
		return Err(conflict("this Idempotency-Key was used for a different sale"));
	}
	Ok(Some(get_sale_by_id(conn, row.get("id")).await?))
}

/// The sale as the API shows it.
#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
pub struct SaleView {
	/// Random, so that sales cannot be enumerated.
	#[schema(example = "f1a01eb4e3e74b15220ac2be24fc38e0")]
	pub id: String,
	pub created_at: DateTime<Utc>,
	/// When an `open` sale expires. An `in-progress` sale stays so past it
	/// until its payment confirms.
	pub expires_at: DateTime<Utc>,
	pub state: SaleState,
	#[schema(example = 35000)]
	pub amount_sat: i64,
	/// The amount keyed in, as a decimal in `currency`. `null` for a sale
	/// keyed in sats.
	#[schema(example = "12.50")]
	pub fiat_amount: Option<String>,
	#[schema(example = "CRC")]
	pub currency: Option<String>,
	/// The price of one bitcoin in `currency` the sale used.
	#[schema(example = "62500000.00")]
	pub btc_price: Option<String>,
	#[schema(example = "2 coffees")]
	pub memo: Option<String>,
	/// Both rails in one BIP 321 URI. Terminals show one rail at a time.
	#[schema(example = "bitcoin:bc1p…?amount=0.00035&lightning=lnbc…")]
	pub payment_request: String,
	/// The forwarding invoice.
	#[schema(example = "lnbc…")]
	pub lightning_invoice: String,
	/// `null` below the store's `onchain_min_sat`.
	#[schema(example = "bc1p…")]
	pub onchain_address: Option<String>,
	pub paid_at: Option<DateTime<Utc>>,
	pub paid_rail: Option<Rail>,
	/// Higher than `amount_sat` when the payer overpaid or paid both rails.
	pub received_sat: i64,
	/// `null` for a Lightning address payment.
	#[schema(example = "Front till")]
	pub device_name: Option<String>,
}

impl From<Sale> for SaleView {
	fn from(s: Sale) -> SaleView {
		let payment_request = payment_request(
			s.onchain_address.as_deref(),
			Amount::from_sat(s.amount_sat as u64),
			&s.lightning_invoice,
		);
		SaleView {
			id: s.public_id,
			created_at: s.created_at,
			expires_at: s.expires_at,
			state: s.state,
			amount_sat: s.amount_sat,
			fiat_amount: s.fiat_amount,
			currency: s.currency,
			btc_price: s.btc_price,
			memo: s.memo,
			payment_request,
			lightning_invoice: s.lightning_invoice,
			onchain_address: s.onchain_address,
			paid_at: s.paid_at,
			paid_rail: s.paid_rail,
			received_sat: s.received_sat,
			device_name: s.device_name,
		}
	}
}

/// The BIP 321 URI of a sale. The address and amount are only present when
/// the sale offers on-chain.
pub fn payment_request(onchain_address: Option<&str>, amount: Amount, lightning: &str) -> String {
	match onchain_address {
		Some(address) => format!("bitcoin:{}?amount={}&lightning={}", address, btc_amount(amount), lightning),
		None => format!("bitcoin:?lightning={}", lightning),
	}
}

/// A BIP 321 amount: bitcoin, without trailing zeros.
fn btc_amount(amount: Amount) -> String {
	let s = amount.to_string_in(Denomination::Bitcoin);
	if s.contains('.') {
		s.trim_end_matches('0').trim_end_matches('.').to_string()
	} else {
		s
	}
}

/// Creates a sale with all the rails it offers. With a `request` from a
/// device, a sale the device already made for it is returned instead. A
/// fiat amount is in `currency` when given, which must be the store's: a
/// terminal that shows another currency gets a conflict, not a sale in a
/// currency it did not show.
pub async fn create_sale(
	app: &App,
	store: &Store,
	device_id: Option<i64>,
	amount: SaleAmount,
	currency: Option<&str>,
	memo: Option<String>,
	lightning_only: bool,
	request: Option<&SaleRequest>,
) -> ApiResult<Sale> {
	let request = match (device_id, request) {
		(Some(device), Some(request)) => {
			let conn = app.db.conn().await?;
			if let Some(sale) = sale_for_request(&*conn, device, request).await? {
				return Ok(sale);
			}
			Some((device, request))
		},
		_ => None,
	};

	let memo = memo.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
	if memo.as_ref().is_some_and(|m| m.chars().count() > MAX_MEMO_CHARS) {
		return Err(bad_request(format!("the memo is longer than {} characters", MAX_MEMO_CHARS)));
	}

	let (amount_sat, fiat) = match amount {
		SaleAmount::Sat(sat) => (sat, None),
		SaleAmount::Fiat(fiat) => {
			if currency.is_some_and(|c| c != store.currency) {
				return Err(conflict(format!("the store's currency is {}", store.currency)));
			}
			let decimals = app.price.decimals(&store.currency)
				.ok_or_else(|| bad_request(format!("the price source no longer quotes {}", store.currency)))?;
			if fiat.scale() > decimals {
				return Err(bad_request(format!(
					"{} has {} decimal places", store.currency, decimals,
				)));
			}
			let fiat = fiat.round(decimals);
			let quote = app.price.quote(&store.currency).await.map_err(ApiError::Upstream)?;
			let sat = fiat.fiat_to_sat(&quote.btc_price)
				.ok_or_else(|| bad_request("the amount is too large"))?;
			(sat, Some((fiat, quote.btc_price)))
		},
	};
	let min = min_sale_amount(app).await?;
	if amount_sat < min {
		return Err(bad_request(format!("the amount must be at least {} sat", min.to_sat())));
	}
	if amount_sat.to_sat() > MAX_SAT {
		return Err(bad_request("the amount is more than 21 million bitcoin"));
	}

	let ark_address = ark::Address::from_str(&store.ark_address)
		.context("store has an invalid Ark address")?;
	let expiry = app.cfg.sales.expiry();
	let description = Some(memo.clone().unwrap_or_else(|| store.name.clone()));

	// The invoice is created first: it needs the network, and an invoice left
	// over from a failed insert just expires.
	let forward = app.forwarder.create_invoice(amount_sat, &ark_address, description, expiry)
		.await.map_err(ApiError::Upstream)?;

	let mut conn = app.db.conn().await?;
	let tx = conn.transaction().await?;

	let onchain = if !lightning_only && amount_sat.to_sat() >= store.onchain_min_sat as u64 {
		onchain_address(app, &tx, store).await?
	} else {
		None
	};

	let public_id = token::random_hex(16);
	// A repeat of the same request that raced this one may have inserted
	// first: then nothing is inserted, and its sale is the answer. The
	// invoice made above just expires.
	let row = tx.query_opt("
		INSERT INTO sale (public_id, store_id, device_id, amount_sat, fiat_amount, currency,
			btc_price, memo, expires_at, request_key, request_fingerprint)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW() + make_interval(secs => $9), $10, $11)
		ON CONFLICT (device_id, request_key) WHERE request_key IS NOT NULL DO NOTHING
		RETURNING id
	", &[
		&public_id, &store.id, &device_id, &(amount_sat.to_sat() as i64),
		&fiat.as_ref().map(|(f, _)| f.to_string()),
		&fiat.as_ref().map(|_| store.currency.clone()),
		&fiat.as_ref().map(|(_, p)| p.to_string()),
		&memo, &(expiry.as_secs() as f64),
		&request.map(|(_, r)| r.key.clone()), &request.map(|(_, r)| r.fingerprint.clone()),
	]).await?;
	let Some(row) = row else {
		// The insert waited for the other request to commit, so its sale is
		// visible here. Rolling back returns the index this one reserved.
		let (device, request) = request.context("a sale insert conflicted without a request key")?;
		let sale = sale_for_request(&tx, device, request).await?
			.context("the conflicting sale is gone")?;
		tx.rollback().await?;
		return Ok(sale);
	};
	let sale_id = row.get::<_, i64>("id");

	tx.execute("
		INSERT INTO sale_payment (sale_id, rail, payment_hash, invoice)
		VALUES ($1, 'lightning', $2, $3)
	", &[&sale_id, &forward.payment_hash().to_string(), &forward.to_string()]).await?;
	if let Some((index, address)) = &onchain {
		tx.execute("
			INSERT INTO sale_payment (sale_id, rail, onchain_index, onchain_address)
			VALUES ($1, 'onchain', $2, $3)
		", &[&sale_id, index, &address.to_string()]).await?;
	}

	let sale = get_sale_by_id(&tx, sale_id).await?;
	tx.commit().await?;
	Ok(sale)
}

/// Reserves the next derivation index of the store's descriptor whose
/// address is unused: not on another sale, of any store, and without
/// history on chain, which a wallet's own receives would leave. `None`
/// when too many are in use.
async fn onchain_address(
	app: &App,
	tx: &tokio_postgres::Transaction<'_>,
	store: &Store,
) -> ApiResult<Option<(i64, bitcoin::Address)>> {
	let desc = StoreDescriptor::parse(&store.onchain_descriptor)
		.context("store has an invalid descriptor")?;
	// Stores that registered the same descriptor take turns until commit,
	// so each sees the addresses the other took.
	tx.execute("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))", &[&store.onchain_descriptor]).await?;
	for _ in 0..MAX_ADDRESS_SKIPS {
		// The update locks the store, so two of its sales never get the
		// same index.
		let row = tx.query_one("
			UPDATE store SET next_onchain_index = next_onchain_index + 1, updated_at = NOW()
			WHERE id = $1 RETURNING next_onchain_index - 1 AS idx
		", &[&store.id]).await?;
		let index = row.get::<_, i64>("idx");
		let index_u32 = u32::try_from(index).context("on-chain index overflow")?;
		let address = desc.address(index_u32, app.cfg.network)?;

		let on_sale = tx.query_opt(
			"SELECT 1 FROM sale_payment WHERE onchain_address = $1", &[&address.to_string()],
		).await?.is_some();
		if on_sale {
			tracing::info!("store {} skips index {}: the address is on another sale", store.id, index);
			continue;
		}
		if !app.chain.watch(&address).await.map_err(ApiError::Upstream)? {
			tracing::info!("store {} skips index {}: the address has history", store.id, index);
			continue;
		}
		return Ok(Some((index, address)));
	}
	tracing::warn!("store {} has no unused address in {} indexes: the sale is Lightning only",
		store.id, MAX_ADDRESS_SKIPS);
	Ok(None)
}

pub async fn get_sale_by_id(conn: &impl GenericClient, id: i64) -> anyhow::Result<Sale> {
	let row = conn.query_one(&format!("{} WHERE s.id = $1", SALE_SELECT), &[&id]).await?;
	Ok(Sale::from_row(&row))
}

/// A sale of `store` by its public id.
pub async fn get_sale(conn: &impl GenericClient, store_id: i64, public_id: &str) -> ApiResult<Sale> {
	let row = conn.query_opt(
		&format!("{} WHERE s.store_id = $1 AND s.public_id = $2", SALE_SELECT),
		&[&store_id, &public_id],
	).await?;
	row.map(|r| Sale::from_row(&r)).ok_or_else(|| not_found("no such sale"))
}

/// Sales of `store`, newest first, older than the sale `before`.
pub async fn list_sales(
	conn: &impl GenericClient,
	store_id: i64,
	before: Option<&str>,
	limit: i64,
) -> ApiResult<Vec<Sale>> {
	let limit = limit.clamp(1, 100);
	let rows = conn.query(&format!("{}
		WHERE s.store_id = $1 AND ($2::TEXT IS NULL OR s.id < (
			SELECT id FROM sale WHERE public_id = $2 AND store_id = $1))
		ORDER BY s.id DESC LIMIT $3
	", SALE_SELECT), &[&store_id, &before, &limit]).await?;
	Ok(rows.iter().map(Sale::from_row).collect())
}

/// Sales of `store` created in `[from, to)`, oldest first, as CSV.
pub async fn sales_csv(
	conn: &impl GenericClient,
	store_id: i64,
	from: Option<DateTime<Utc>>,
	to: Option<DateTime<Utc>>,
) -> ApiResult<String> {
	let rows = conn.query(&format!("{}
		WHERE s.store_id = $1 AND ($2::TIMESTAMPTZ IS NULL OR s.created_at >= $2)
			AND ($3::TIMESTAMPTZ IS NULL OR s.created_at < $3)
		ORDER BY s.id
	", SALE_SELECT), &[&store_id, &from, &to]).await?;
	let mut out = String::from(
		"id,created_at,paid_at,state,paid_rail,amount_sat,received_sat,fiat_amount,currency,btc_price,memo,device_name\n",
	);
	for row in rows {
		let s = Sale::from_row(&row);
		let fields = [
			s.public_id,
			s.created_at.to_rfc3339(),
			s.paid_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
			s.state.as_str().to_string(),
			s.paid_rail.map(|r| r.as_str().to_string()).unwrap_or_default(),
			s.amount_sat.to_string(),
			s.received_sat.to_string(),
			s.fiat_amount.unwrap_or_default(),
			s.currency.unwrap_or_default(),
			s.btc_price.unwrap_or_default(),
			s.memo.unwrap_or_default(),
			s.device_name.unwrap_or_default(),
		];
		out.push_str(&fields.iter().map(|f| csv_field(f)).collect::<Vec<_>>().join(","));
		out.push('\n');
	}
	Ok(out)
}

/// Quotes a CSV field when needed. A leading formula character is escaped,
/// so a memo cannot run as a spreadsheet formula.
fn csv_field(s: &str) -> String {
	let s = if s.starts_with(['=', '+', '-', '@']) { format!("'{}", s) } else { s.to_string() };
	if s.contains([',', '"', '\n', '\r']) {
		format!("\"{}\"", s.replace('"', "\"\""))
	} else {
		s
	}
}

#[cfg(test)]
mod test {
	use ark::fees::PpmFeeRate;

	#[test]
	fn min_lightning_amount_leaves_something_after_the_fee() {
		let fees = |base: u64, ppm: u64| LightningReceiveFees { base_fee: Amount::from_sat(base), ppm: PpmFeeRate(ppm) };
		assert_eq!(min_lightning_amount(&fees(0, 0)), Some(Amount::from_sat(1)));
		// 0.2% of 1 sat rounds up to 1 sat, which leaves nothing.
		assert_eq!(min_lightning_amount(&fees(0, 2_000)), Some(Amount::from_sat(2)));
		assert_eq!(min_lightning_amount(&fees(100, 0)), Some(Amount::from_sat(101)));
		assert_eq!(min_lightning_amount(&fees(100, 10_000)), Some(Amount::from_sat(103)));
		assert_eq!(min_lightning_amount(&fees(0, 1_000_000)), None);
		// At 99.9% the fee leaves a whole sat only from 8,000 sats on.
		assert_eq!(min_lightning_amount(&fees(7, 999_000)), Some(Amount::from_sat(8_000)));
		for (base, ppm) in [(0, 2_000), (100, 0), (100, 10_000), (7, 999_000)] {
			let fees = fees(base, ppm);
			let min = min_lightning_amount(&fees).unwrap();
			assert!(min > fees.calculate(min).unwrap());
			assert!(min - Amount::ONE_SAT <= fees.calculate(min - Amount::ONE_SAT).unwrap());
		}
	}
	use super::*;

	#[test]
	fn builds_payment_requests() {
		assert_eq!(
			payment_request(Some("bc1qaddr"), Amount::from_sat(35_000), "lnbc1a"),
			"bitcoin:bc1qaddr?amount=0.00035&lightning=lnbc1a",
		);
		assert_eq!(
			payment_request(None, Amount::from_sat(1_000), "lnbc1a"),
			"bitcoin:?lightning=lnbc1a",
		);
		assert_eq!(btc_amount(Amount::from_sat(100_000_000)), "1");
		assert_eq!(btc_amount(Amount::from_sat(1)), "0.00000001");
	}

	#[test]
	fn escapes_csv() {
		assert_eq!(csv_field("plain"), "plain");
		assert_eq!(csv_field("a,b"), "\"a,b\"");
		assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
		assert_eq!(csv_field("=SUM(A1)"), "'=SUM(A1)");
	}
}
