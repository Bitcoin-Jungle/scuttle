//! Terminal endpoints.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::{IntoParams, ToSchema};

use crate::App;
use crate::api::{StoreView, Terminal};
use crate::database::{Rail, STORE_COLUMNS, SaleState, Store};
use crate::error::{ApiError, ApiResult, ErrorBody, bad_request, conflict, not_found};
use crate::money::{Decimal, MAX_SAT};
use crate::rails::forward::ForwardStatus;
use crate::sales::{self, SaleAmount, SaleRequest, SaleView};
use crate::{token, watcher};


/// The longest a long-poll waits.
const LONG_POLL: Duration = Duration::from_secs(25);

#[derive(Deserialize, ToSchema)]
pub struct PairRequest {
	/// The code from the pairing link `/pair/<code>`.
	#[schema(example = "ad13dcd6ba79d527ef933317")]
	code: String,
	/// 1 to 64 characters, shown to the owner and on receipts.
	#[schema(example = "Front till")]
	device_name: String,
}

#[derive(Serialize, ToSchema)]
pub struct PairResponse {
	/// The device token. posd keeps only its hash, so it is shown once.
	token: String,
	store: StoreView,
}

/// Pairs a terminal with a pairing code. A code works once.
#[utoipa::path(
	post, path = "/pair", tag = "terminal", security(()),
	request_body = PairRequest,
	responses(
		(status = 200, description = "The device token and its store", body = PairResponse),
		(status = 400, description = "The device name is not valid", body = ErrorBody),
		(status = 404, description = "The code is unknown, used or expired", body = ErrorBody),
	),
)]
pub async fn pair(
	State(app): State<Arc<App>>,
	Json(req): Json<PairRequest>,
) -> ApiResult<Json<PairResponse>> {
	let name = req.device_name.trim();
	if name.is_empty() || name.chars().count() > 64 {
		return Err(bad_request("the device name must be 1 to 64 characters"));
	}

	let mut conn = app.db.conn().await?;
	let tx = conn.transaction().await?;
	// Locking the invite row makes a code work exactly once.
	let row = tx.query_opt(&format!("
		SELECT i.id AS invite_id, {} FROM invite i JOIN store s ON s.id = i.store_id
		WHERE i.code_hash = $1 AND i.redeemed_at IS NULL AND i.expires_at > NOW()
		FOR UPDATE OF i
	", STORE_COLUMNS), &[&token::hash(req.code.trim())]).await?
		.ok_or_else(|| not_found("the pairing code is unknown, used or expired"))?;
	let invite_id: i64 = row.get("invite_id");
	let store = Store::from_row(&row);

	let device_token = token::new_token();
	let device_id: i64 = tx.query_one(
		"INSERT INTO device (store_id, name, token_hash) VALUES ($1, $2, $3) RETURNING id",
		&[&store.id, &name, &token::hash(&device_token)],
	).await?.get("id");
	tx.execute(
		"UPDATE invite SET redeemed_at = NOW(), device_id = $2 WHERE id = $1",
		&[&invite_id, &device_id],
	).await?;
	tx.commit().await?;

	Ok(Json(PairResponse { token: device_token, store: StoreView::new(&app, &store).await }))
}

#[derive(Serialize, ToSchema)]
pub struct TerminalView {
	store: StoreView,
	#[schema(example = "Front till")]
	device_name: String,
}

/// The terminal's store and name.
#[utoipa::path(
	get, path = "/terminal", tag = "terminal", security(("device_token" = [])),
	responses(
		(status = 200, description = "The terminal", body = TerminalView),
		(status = 401, description = "The token is missing, not valid, or revoked", body = ErrorBody),
	),
)]
pub async fn terminal(State(app): State<Arc<App>>, t: Terminal) -> Json<TerminalView> {
	Json(TerminalView { store: StoreView::new(&app, &t.store).await, device_name: t.device.name })
}

#[derive(Serialize, ToSchema)]
pub struct Rate {
	#[schema(example = "CRC")]
	currency: String,
	/// The price of one bitcoin in `currency`.
	#[schema(example = "62500000.00")]
	btc_price: String,
	fetched_at: DateTime<Utc>,
}

/// The bitcoin price in the store's currency.
#[utoipa::path(
	get, path = "/rate", tag = "terminal", security(("device_token" = [])),
	responses(
		(status = 200, description = "The current price", body = Rate),
		(status = 401, description = "The token is missing, not valid, or revoked", body = ErrorBody),
		(status = 502, description = "The price feed failed", body = ErrorBody),
	),
)]
pub async fn rate(State(app): State<Arc<App>>, t: Terminal) -> ApiResult<Json<Rate>> {
	let quote = app.price.quote(&t.store.currency).await.map_err(ApiError::Upstream)?;
	Ok(Json(Rate {
		currency: quote.currency,
		btc_price: quote.btc_price.to_string(),
		fetched_at: quote.fetched_at,
	}))
}

/// A new sale. Give exactly one of `fiat_amount` and `amount_sat`.
#[derive(Deserialize, ToSchema)]
pub struct CreateSale {
	/// A decimal in the store's currency, with at most its decimal places.
	#[schema(example = "12.50")]
	fiat_amount: Option<String>,
	/// The currency the terminal showed `fiat_amount` in. When it is not the
	/// store's, as after the owner changed it, the sale is refused with
	/// `409` instead of charging in another currency.
	#[schema(example = "CRC")]
	currency: Option<String>,
	/// At most 21 million bitcoin.
	#[schema(example = 35000)]
	amount_sat: Option<u64>,
	/// At most 200 characters.
	#[schema(example = "2 coffees")]
	memo: Option<String>,
}

/// The longest `Idempotency-Key` posd keeps.
const MAX_REQUEST_KEY_LEN: usize = 64;

/// The sale request named by an `Idempotency-Key` header, if any.
fn sale_request(headers: &HeaderMap, req: &CreateSale) -> ApiResult<Option<SaleRequest>> {
	let Some(key) = headers.get("idempotency-key") else { return Ok(None) };
	let key = key.to_str().ok()
		.filter(|k| !k.is_empty() && k.len() <= MAX_REQUEST_KEY_LEN)
		.filter(|k| k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
		.ok_or_else(|| bad_request(format!(
			"Idempotency-Key must be 1 to {} letters, digits, - or _", MAX_REQUEST_KEY_LEN,
		)))?;
	let fingerprint = serde_json::to_string(&json!({
		"fiat_amount": req.fiat_amount, "currency": req.currency, "amount_sat": req.amount_sat,
		"memo": req.memo,
	})).expect("serializes");
	Ok(Some(SaleRequest { key: key.to_string(), fingerprint }))
}

/// Creates a sale.
///
/// With an `Idempotency-Key`, a request repeated with the same key and body,
/// as after a response lost on a poor network, returns the sale of the
/// first request instead of making another.
#[utoipa::path(
	post, path = "/sales", tag = "terminal", security(("device_token" = [])),
	params(
		("Idempotency-Key" = Option<String>, Header, pattern = "^[A-Za-z0-9_-]{1,64}$",
			description = "Unique per sale on the device."),
	),
	request_body = CreateSale,
	responses(
		(status = 200, description = "The sale", body = SaleView),
		(status = 400, description = "The amount, memo or key is not valid", body = ErrorBody),
		(status = 401, description = "The token is missing, not valid, or revoked", body = ErrorBody),
		(status = 409, description = "The key was used with a different body, or `currency` \
			is not the store's", body = ErrorBody),
		(status = 502, description = "The price feed or the Ark server failed", body = ErrorBody),
	),
)]
pub async fn create_sale(
	State(app): State<Arc<App>>,
	t: Terminal,
	headers: HeaderMap,
	Json(req): Json<CreateSale>,
) -> ApiResult<Json<SaleView>> {
	let request = sale_request(&headers, &req)?;
	let amount = match (req.fiat_amount, req.amount_sat) {
		(Some(fiat), None) => SaleAmount::Fiat(fiat.parse::<Decimal>()
			.map_err(|e| bad_request(e.to_string()))?),
		(None, Some(sat)) if sat <= MAX_SAT => SaleAmount::Sat(bitcoin::Amount::from_sat(sat)),
		(None, Some(_)) => return Err(bad_request("the amount is more than 21 million bitcoin")),
		_ => return Err(bad_request("give exactly one of fiat_amount and amount_sat")),
	};
	let sale = sales::create_sale(
		&app, &t.store, Some(t.device.id), amount, req.currency.as_deref(), req.memo,
		request.as_ref(),
	).await?;
	Ok(Json(sale.into()))
}

/// Gives a sale its on-chain address, when the cashier picks on-chain.
///
/// A sale gets one address, so repeating the request returns the same one.
/// A payer who already paid part of the amount to it sees the full amount
/// in `onchain_uri`; the terminal shows what arrived from
/// `onchain_received_sat` and `onchain_pending_sat`.
#[utoipa::path(
	post, path = "/sales/{id}/onchain", tag = "terminal", security(("device_token" = [])),
	params(("id" = String, Path, description = "The sale id")),
	responses(
		(status = 200, description = "The sale, with its on-chain address", body = SaleView),
		(status = 401, description = "The token is missing, not valid, or revoked", body = ErrorBody),
		(status = 404, description = "No such sale in this store", body = ErrorBody),
		(status = 409, description = "The sale is not open, is below the store's on-chain \
			minimum, or the store has no unused address", body = ErrorBody),
		(status = 502, description = "The chain source failed", body = ErrorBody),
	),
)]
pub async fn add_onchain(
	State(app): State<Arc<App>>,
	t: Terminal,
	Path(id): Path<String>,
) -> ApiResult<Json<SaleView>> {
	Ok(Json(sales::add_onchain(&app, &t.store, &id).await?.into()))
}

#[derive(Deserialize, IntoParams)]
pub struct WaitQuery {
	/// Hold the request until the sale is no longer in this state, for at
	/// most 25 seconds. Repeat it with the last state seen to follow a sale.
	#[param(value_type = Option<SaleState>)]
	wait: Option<String>,
	/// With `wait`, also return when `onchain_received_sat +
	/// onchain_pending_sat` is no longer this, so a terminal sees a partial
	/// payment arrive.
	#[param(minimum = 0)]
	onchain_sat: Option<i64>,
}

/// A sale of the terminal's store, at once or by long-poll.
#[utoipa::path(
	get, path = "/sales/{id}", tag = "terminal", security(("device_token" = [])),
	params(("id" = String, Path, description = "The sale id"), WaitQuery),
	responses(
		(status = 200, description = "The sale", body = SaleView),
		(status = 400, description = "`wait` is not a state", body = ErrorBody),
		(status = 401, description = "The token is missing, not valid, or revoked", body = ErrorBody),
		(status = 404, description = "No such sale in this store", body = ErrorBody),
	),
)]
pub async fn get_sale(
	State(app): State<Arc<App>>,
	t: Terminal,
	Path(id): Path<String>,
	Query(q): Query<WaitQuery>,
) -> ApiResult<Json<SaleView>> {
	let wait = q.wait.map(|w| w.parse::<SaleState>().map_err(|e| bad_request(e.to_string())))
		.transpose()?;
	let deadline = tokio::time::Instant::now() + LONG_POLL;
	loop {
		// Subscribe before reading, so a change between the read and the
		// wait is not missed.
		let mut changes = app.sale_changes.subscribe();
		let sale = {
			let conn = app.db.conn().await?;
			sales::get_sale(&*conn, t.store.id, &id).await?
		};
		let onchain_sat = sale.onchain_received_sat + sale.onchain_pending_sat;
		let done = match wait {
			Some(state) => sale.state != state || q.onchain_sat.is_some_and(|n| n != onchain_sat),
			None => true,
		};
		let now = tokio::time::Instant::now();
		if done || now >= deadline {
			return Ok(Json(sale.into()));
		}
		let _ = tokio::time::timeout(deadline - now, changes.changed()).await;
	}
}

/// Cancels an open sale.
///
/// posd first checks the sale's on-chain address, then asks barkd to cancel
/// the Lightning invoice. barkd forgets an unpaid invoice; the Ark server
/// keeps it payable until it expires. When a payment was made or is
/// arriving, the sale is not canceled: it is `409`, and the sale soon shows
/// its new state.
#[utoipa::path(
	post, path = "/sales/{id}/cancel", tag = "terminal", security(("device_token" = [])),
	params(("id" = String, Path, description = "The sale id")),
	responses(
		(status = 200, description = "The canceled sale", body = SaleView),
		(status = 401, description = "The token is missing, not valid, or revoked", body = ErrorBody),
		(status = 404, description = "No such sale in this store", body = ErrorBody),
		(status = 409, description = "The sale is not open, or a payment for it was made", body = ErrorBody),
		(status = 502, description = "The chain source or barkd failed; the sale stays open", body = ErrorBody),
	),
)]
pub async fn cancel_sale(
	State(app): State<Arc<App>>,
	t: Terminal,
	Path(id): Path<String>,
) -> ApiResult<Json<SaleView>> {
	let sale = {
		let conn = app.db.conn().await?;
		sales::get_sale(&*conn, t.store.id, &id).await?
	};
	if sale.state != SaleState::Open {
		return Err(conflict(format!("the sale is {}", sale.state)));
	}
	let payments = watcher::unsettled_payments(&app, sale.id).await?;

	// A payment to the address shown moves the sale on: in the mempool to
	// in-progress, confirmed to paid.
	for p in payments.iter().filter(|p| p.rail == Rail::Onchain) {
		if watcher::check_now(&app, p).await.map_err(ApiError::Upstream)? {
			app.notify_sale_change();
			let sale = sales::get_sale(&*app.db.conn().await?, t.store.id, &id).await?;
			return Err(conflict(format!("the sale is {}", sale.state)));
		}
	}
	// Part of the amount arrived on-chain: the customer paid something, so
	// the cashier takes the rest, or lets the sale expire with the part
	// recorded.
	let sale = sales::get_sale(&*app.db.conn().await?, t.store.id, &id).await?;
	if sale.onchain_received_sat + sale.onchain_pending_sat > 0 {
		return Err(conflict("part of the amount was paid on-chain"));
	}

	for p in payments.iter().filter(|p| p.rail == Rail::Lightning) {
		let hash = p.payment_hash.as_deref().context("Lightning payment without hash")?;
		if app.forwarder.cancel(hash).await.is_ok() {
			watcher::settle_canceled(&app, p.payment_id).await?;
			continue;
		}
		match app.forwarder.status(hash).await.map_err(ApiError::Upstream)? {
			ForwardStatus::Gone => watcher::settle_canceled(&app, p.payment_id).await?,
			// barkd refused for another reason, as when its own task holds
			// the receive for a moment: the sale stays open, try again.
			ForwardStatus::AwaitingPayment => {
				return Err(ApiError::Upstream(anyhow::anyhow!("barkd did not cancel invoice {}", hash)));
			},
			ForwardStatus::Arriving | ForwardStatus::Delivered(_) => {
				if watcher::check_now(&app, p).await? {
					app.notify_sale_change();
				}
				return Err(conflict("a payment for the sale was made"));
			},
		}
	}

	// An on-chain payment may have arrived meanwhile. One that arrives
	// later is still recorded, as received on a canceled sale.
	let conn = app.db.conn().await?;
	let n = conn.execute("
		UPDATE sale SET state = 'canceled', updated_at = NOW()
		WHERE id = $1 AND state = 'open'
	", &[&sale.id]).await?;
	let sale = sales::get_sale(&*conn, t.store.id, &id).await?;
	if n == 0 {
		return Err(conflict(format!("the sale is {}", sale.state)));
	}
	app.notify_sale_change();
	Ok(Json(sale.into()))
}

#[derive(Deserialize, IntoParams)]
pub struct ListQuery {
	/// The id of the last sale of the previous page.
	pub before: Option<String>,
	/// At most 100. The default is 50.
	#[param(maximum = 100, minimum = 1)]
	pub limit: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct SalesPage {
	/// Newest first.
	sales: Vec<SaleView>,
}

/// The store's sales, newest first.
#[utoipa::path(
	get, path = "/sales", tag = "terminal", security(("device_token" = [])),
	params(ListQuery),
	responses(
		(status = 200, description = "A page of sales", body = SalesPage),
		(status = 401, description = "The token is missing, not valid, or revoked", body = ErrorBody),
	),
)]
pub async fn list_sales(
	State(app): State<Arc<App>>,
	t: Terminal,
	Query(q): Query<ListQuery>,
) -> ApiResult<Json<SalesPage>> {
	list(&app, &t.store, q).await
}

pub async fn list(app: &App, store: &Store, q: ListQuery) -> ApiResult<Json<SalesPage>> {
	let conn = app.db.conn().await?;
	let sales = sales::list_sales(&*conn, store.id, q.before.as_deref(), q.limit.unwrap_or(50)).await?;
	Ok(Json(SalesPage { sales: sales.into_iter().map(SaleView::from).collect() }))
}

#[derive(Deserialize, IntoParams)]
pub struct CsvQuery {
	/// A date (`2026-11-20`, in UTC) or an RFC 3339 time.
	pub from: Option<String>,
	/// A date or an RFC 3339 time. A date includes that whole day.
	pub to: Option<String>,
}

/// The store's sales as CSV.
///
/// The columns are `id, created_at, paid_at, state, paid_rail, amount_sat,
/// received_sat, fiat_amount, currency, btc_price, memo, device_name`.
#[utoipa::path(
	get, path = "/sales.csv", tag = "terminal", security(("device_token" = [])),
	params(CsvQuery),
	responses(
		(status = 200, description = "The CSV", content_type = "text/csv", body = String),
		(status = 400, description = "A date is not valid", body = ErrorBody),
		(status = 401, description = "The token is missing, not valid, or revoked", body = ErrorBody),
	),
)]
pub async fn sales_csv(
	State(app): State<Arc<App>>,
	t: Terminal,
	Query(q): Query<CsvQuery>,
) -> ApiResult<Response> {
	csv(&app, &t.store, q).await
}

pub async fn csv(app: &App, store: &Store, q: CsvQuery) -> ApiResult<Response> {
	let from = q.from.as_deref().map(|d| parse_date(d, false)).transpose()?;
	let to = q.to.as_deref().map(|d| parse_date(d, true)).transpose()?;
	let conn = app.db.conn().await?;
	let body = sales::sales_csv(&*conn, store.id, from, to).await?;
	Ok((
		[
			(header::CONTENT_TYPE, "text/csv; charset=utf-8"),
			(header::CONTENT_DISPOSITION, "attachment; filename=\"sales.csv\""),
		],
		body,
	).into_response())
}

/// An RFC 3339 time, or a date in UTC. A date as the end of a range
/// includes that whole day.
fn parse_date(s: &str, end: bool) -> ApiResult<DateTime<Utc>> {
	if let Ok(t) = DateTime::parse_from_rfc3339(s) {
		return Ok(t.with_timezone(&Utc));
	}
	let day = NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
		.ok_or_else(|| bad_request(format!("not a date: {}", s)))?;
	let day = if end { day.succ_opt().ok_or_else(|| bad_request("date out of range"))? } else { day };
	Ok(day.and_hms_opt(0, 0, 0).expect("midnight exists").and_utc())
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn date_ranges() {
		assert_eq!(parse_date("2026-11-20", false).unwrap().to_rfc3339(), "2026-11-20T00:00:00+00:00");
		assert_eq!(parse_date("2026-11-20", true).unwrap().to_rfc3339(), "2026-11-21T00:00:00+00:00");
		assert_eq!(parse_date("2026-11-20T10:00:00Z", true).unwrap().to_rfc3339(), "2026-11-20T10:00:00+00:00");
		assert!(parse_date("20/11/2026", false).is_err());
	}
}
