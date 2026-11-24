//! Lightning address (LUD-06 and LUD-16), with LUD-22 payment methods.
//!
//! The Ark server cannot put a description hash in an invoice, so the
//! invoice carries the metadata's text description, not its hash. Wallets
//! that enforce the LUD-06 hash check refuse these invoices.
//!
//! A wallet that supports LUD-22 can ask for the owner's Ark address or
//! silent payment address instead of an invoice, and pay it directly. Those
//! payments never pass posd, so they are not in the store's sales.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use bitcoin::Amount;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::{IntoParams, ToSchema};

use crate::App;
use crate::api::request_host;
use crate::config::Brand;
use crate::database::{STORE_COLUMNS, Store};
use crate::error::{ApiError, ApiResult, LnurlError, LnurlErrorBody, bad_request, not_found};
use crate::sales::{self, SaleAmount};


const MIN_SENDABLE_MSAT: u64 = 1_000;

/// The LUD-22 method for the owner's Ark address.
pub const METHOD_ARK: &str = "ARK";
/// The LUD-22 method for the owner's silent payment address.
pub const METHOD_SILENT_PAYMENT: &str = "SP";

/// The LUD-22 methods a store offers besides Lightning.
fn payment_methods(store: &Store) -> Vec<&'static str> {
	let mut methods = vec![METHOD_ARK];
	if store.silent_payment_address.is_some() {
		methods.push(METHOD_SILENT_PAYMENT);
	}
	methods
}

/// The address of a LUD-22 method, if the store offers it.
fn method_address<'a>(store: &'a Store, method: &str) -> Option<&'a str> {
	match method {
		METHOD_ARK => Some(&store.ark_address),
		METHOD_SILENT_PAYMENT => store.silent_payment_address.as_deref(),
		_ => None,
	}
}

/// The most unpaid Lightning address invoices a store has open at once.
/// Asking for invoices needs no credentials, and the watcher checks every
/// open one, so the caps keep a flood from slowing every terminal.
pub const MAX_OPEN_PER_STORE: i64 = 25;
/// The most across all stores.
const MAX_OPEN_TOTAL: i64 = 500;

/// Lightning address invoices being made: asked of barkd but not yet saved
/// as sales. They count toward the caps, so a burst of requests cannot all
/// pass the check before any is saved.
#[derive(Default)]
pub struct Pending {
	/// Held while a request checks the caps, so checks run one at a time.
	check: tokio::sync::Mutex<()>,
	/// Per store, and in all.
	count: std::sync::Mutex<(HashMap<i64, i64>, i64)>,
}

/// A place under the caps, given back when the request ends.
pub struct Slot<'a> {
	pending: &'a Pending,
	store_id: i64,
}

impl Pending {
	/// A place for one more invoice of `store_id`, or `429` past a cap.
	async fn reserve<'a>(&'a self, app: &App, store_id: i64) -> ApiResult<Slot<'a>> {
		let _check = self.check.lock().await;
		let open = app.db.conn().await?.query_one("
			SELECT count(*) FILTER (WHERE store_id = $1) AS store, count(*) AS total
			FROM sale WHERE device_id IS NULL AND state = 'open' AND expires_at > NOW()
		", &[&store_id]).await?;
		let mut count = self.count.lock().expect("not poisoned");
		let store = open.get::<_, i64>("store") + count.0.get(&store_id).copied().unwrap_or(0);
		let total = open.get::<_, i64>("total") + count.1;
		if store >= MAX_OPEN_PER_STORE || total >= MAX_OPEN_TOTAL {
			return Err(ApiError::TooManyRequests("too many unpaid invoices are open, try again later".into()));
		}
		*count.0.entry(store_id).or_default() += 1;
		count.1 += 1;
		Ok(Slot { pending: self, store_id })
	}
}

impl Drop for Slot<'_> {
	fn drop(&mut self) {
		let mut count = self.pending.count.lock().expect("not poisoned");
		if let Some(n) = count.0.get_mut(&self.store_id) {
			*n -= 1;
			if *n == 0 {
				count.0.remove(&self.store_id);
			}
		}
		count.1 -= 1;
	}
}

/// The store whose Lightning address is `name` on the domain the request
/// was made to. Each brand's domain has its own names.
async fn store_by_name(app: &App, headers: &HeaderMap, name: &str) -> ApiResult<(Store, Brand)> {
	let key = app.cfg.brand_for_host(&request_host(app, headers))
		.ok_or_else(|| not_found("no Lightning addresses on this domain"))?;
	let brand = app.cfg.brand(key).expect("brand_for_host returns a configured brand").clone();
	let conn = app.db.conn().await?;
	let row = conn.query_opt(
		&format!("SELECT {} FROM store s WHERE s.brand = $1 AND s.lightning_address_name = $2", STORE_COLUMNS),
		&[&key.as_str(), &name],
	).await?.ok_or_else(|| not_found("no such Lightning address"))?;
	Ok((Store::from_row(&row), brand))
}

fn metadata(store: &Store, brand: &Brand, name: &str) -> String {
	json!([
		["text/plain", format!("Payment to {}", store.name)],
		["text/identifier", format!("{}@{}", name, brand.host())],
	]).to_string()
}

/// A LUD-06 pay request.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PayRequest {
	#[schema(example = "payRequest")]
	tag: &'static str,
	callback: String,
	/// In msat.
	#[schema(example = 1000)]
	min_sendable: u64,
	/// In msat, set in the posd config.
	max_sendable: u64,
	/// The LUD-06 metadata, a JSON array as a string.
	metadata: String,
	/// LUD-22: the methods besides Lightning that the callback can return
	/// an address for. `SP` only when the owner registered a silent payment
	/// address.
	/// Named as LUD-22 names it, not in the camelCase of LUD-06.
	#[serde(rename = "payment_methods")]
	#[schema(example = json!(["ARK", "SP"]))]
	payment_methods: Vec<&'static str>,
}

/// The pay request of a Lightning address (LUD-16).
///
/// Each brand's domain has its own names: posd tells them apart by the
/// request's host (see [request_host]).
#[utoipa::path(
	get, path = "/.well-known/lnurlp/{name}", tag = "lnurl", security(()),
	params(("name" = String, Path, description = "The name before the @")),
	responses(
		(status = 200, description = "The pay request", body = PayRequest),
		(status = 404, description = "No such Lightning address on this domain", body = LnurlErrorBody),
	),
)]
pub async fn pay_request(
	State(app): State<Arc<App>>,
	headers: HeaderMap,
	Path(name): Path<String>,
) -> Result<Json<PayRequest>, LnurlError> {
	let (store, brand) = store_by_name(&app, &headers, &name).await?;
	Ok(Json(PayRequest {
		tag: "payRequest",
		callback: brand.url(&format!("/api/v1/lnurlp/{}/callback", name)),
		min_sendable: MIN_SENDABLE_MSAT,
		max_sendable: app.cfg.sales.lnurl_max_sat * 1000,
		metadata: metadata(&store, &brand, &name),
		payment_methods: payment_methods(&store),
	}))
}

#[derive(Deserialize, IntoParams)]
pub struct CallbackQuery {
	/// In msat: whole sats, within the pay request's bounds.
	amount: u64,
	/// LUD-22: methods from `payment_methods`, comma-separated, most
	/// preferred first. The first one the store offers is answered with its
	/// address; with none, or without the field, the answer is an invoice.
	#[param(example = "ARK,SP")]
	payment_method: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct CallbackResponse {
	/// The forwarding invoice. It carries the metadata's text description,
	/// not its hash: the Ark server cannot put a hash in an invoice.
	pr: String,
	/// Always empty.
	#[schema(value_type = Vec<Object>)]
	routes: Vec<serde_json::Value>,
}

/// A LUD-22 address, for the amount of the callback.
#[derive(Serialize, ToSchema)]
pub struct MethodAddress {
	#[schema(example = "ark1…")]
	address: String,
}

/// An invoice, or with a LUD-22 method an address, keyed by the method:
/// `{"ARK": {"address": "ark1…"}}`. Tell them apart by their shape.
#[derive(Serialize, ToSchema)]
#[serde(untagged)]
pub enum CallbackReply {
	Invoice(CallbackResponse),
	Method(BTreeMap<String, MethodAddress>),
}

/// Makes the invoice for a Lightning address payment (LUD-06 callback), or
/// gives the address of a LUD-22 method.
#[utoipa::path(
	get, path = "/lnurlp/{name}/callback", tag = "lnurl", security(()),
	params(("name" = String, Path, description = "The name before the @"), CallbackQuery),
	responses(
		(status = 200, description = "The invoice, or the address of a LUD-22 method", body = CallbackReply),
		(status = 400, description = "The amount is out of range or not whole sats", body = LnurlErrorBody),
		(status = 404, description = "No such Lightning address on this domain", body = LnurlErrorBody),
		(status = 429, description = "Too many unpaid invoices are open; try again in a few \
			minutes", body = LnurlErrorBody),
		(status = 502, description = "The Ark server failed", body = LnurlErrorBody),
	),
)]
pub async fn callback(
	State(app): State<Arc<App>>,
	headers: HeaderMap,
	Path(name): Path<String>,
	Query(q): Query<CallbackQuery>,
) -> Result<Json<CallbackReply>, LnurlError> {
	let (store, _) = store_by_name(&app, &headers, &name).await?;
	if q.amount < MIN_SENDABLE_MSAT || q.amount > app.cfg.sales.lnurl_max_sat * 1000 {
		return Err(bad_request("the amount is out of range").into());
	}
	if q.amount % 1000 != 0 {
		return Err(bad_request("the amount must be whole sats").into());
	}
	let method = q.payment_method.as_deref().unwrap_or("").split(',').map(str::trim)
		.find_map(|m| method_address(&store, m).map(|a| (m, a)));
	if let Some((method, address)) = method {
		let reply = BTreeMap::from([(method.to_string(), MethodAddress { address: address.to_string() })]);
		return Ok(Json(CallbackReply::Method(reply)));
	}
	let _slot = app.lnurl_pending.reserve(&app, store.id).await?;
	let amount = SaleAmount::Sat(Amount::from_sat(q.amount / 1000));
	let sale = sales::create_sale(&app, &store, None, amount, None, None, None).await?;
	Ok(Json(CallbackReply::Invoice(CallbackResponse { pr: sale.lightning_invoice, routes: vec![] })))
}
