//! The operator API of a brand, authenticated by the brand's `admin_token`.
//!
//! An operator reserves Lightning address names on its brand's domain, so
//! nobody else registers them, and assigns a name to a store, for example
//! the old username of a user it has verified.

use std::str::FromStr;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::App;
use crate::api::{Operator, StoreView};
use crate::api::owner::{LIGHTNING_NAME_RULE, lightning_name_taken, valid_lightning_name};
use crate::database::{STORE_COLUMNS, Store};
use crate::error::{ApiResult, ErrorBody, bad_request, not_found};
use crate::names;


/// The most names one request reserves.
const MAX_RESERVE_BATCH: usize = 10_000;

#[derive(Deserialize, ToSchema)]
pub struct ReserveNames {
	/// At most 10,000 Lightning address names.
	#[schema(example = json!(["cornercafe"]), max_items = 10000)]
	names: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct Reserved {
	/// How many of the names were newly reserved.
	reserved: u64,
}

/// Reserves names on the operator's domain.
///
/// A reserved name cannot be taken at registration or by an owner's
/// change, on that domain only. Reserving a name again, or one a store
/// already has, changes nothing.
#[utoipa::path(
	post, path = "/admin/reserved-names", tag = "operator", security(("admin_token" = [])),
	request_body = ReserveNames,
	responses(
		(status = 200, description = "How many names were reserved", body = Reserved),
		(status = 400, description = "Too many names, or a name is not valid", body = ErrorBody),
		(status = 401, description = "The token is not a brand's admin_token", body = ErrorBody),
	),
)]
pub async fn reserve_names(
	State(app): State<Arc<App>>,
	op: Operator,
	Json(req): Json<ReserveNames>,
) -> ApiResult<Json<Reserved>> {
	if req.names.len() > MAX_RESERVE_BATCH {
		return Err(bad_request(format!("at most {} names per request", MAX_RESERVE_BATCH)));
	}
	if let Some(bad) = req.names.iter().find(|n| !valid_lightning_name(n)) {
		return Err(bad_request(format!("{}: {}", bad, LIGHTNING_NAME_RULE)));
	}
	let conn = app.db.conn().await?;
	let added = conn.execute("
		INSERT INTO reserved_name (brand, name) SELECT $1, unnest($2::TEXT[])
		ON CONFLICT DO NOTHING
	", &[&op.brand.as_str(), &req.names]).await?;
	Ok(Json(Reserved { reserved: added }))
}

#[derive(Deserialize, ToSchema)]
pub struct AssignLightningAddress {
	/// The store's Ark address, as registered.
	#[schema(example = "ark1…")]
	ark_address: String,
	#[schema(example = "cornercafe", pattern = "^[a-z0-9._-]{1,64}$")]
	name: String,
}

/// Gives a store of the operator's brand a Lightning address, reserved or not.
///
/// A name another store gave up is held for it like any other, and the name
/// the store had is held for it.
#[utoipa::path(
	post, path = "/admin/lightning-addresses", tag = "operator", security(("admin_token" = [])),
	request_body = AssignLightningAddress,
	responses(
		(status = 200, description = "The store", body = StoreView),
		(status = 400, description = "The name or the Ark address is not valid", body = ErrorBody),
		(status = 401, description = "The token is not a brand's admin_token", body = ErrorBody),
		(status = 404, description = "No store of the brand has the address", body = ErrorBody),
		(status = 409, description = "Another store has or holds the name", body = ErrorBody),
	),
)]
pub async fn assign_lightning_address(
	State(app): State<Arc<App>>,
	op: Operator,
	Json(req): Json<AssignLightningAddress>,
) -> ApiResult<Json<StoreView>> {
	if !valid_lightning_name(&req.name) {
		return Err(bad_request(LIGHTNING_NAME_RULE));
	}
	// The address as posd stores it, whatever case the caller wrote it in.
	let ark_address = ark::Address::from_str(&req.ark_address)
		.map_err(|_| bad_request("not a valid Ark address"))?.to_string();
	let mut conn = app.db.conn().await?;
	let tx = conn.transaction().await?;
	let current = tx.query_opt(&format!("
		SELECT {} FROM store s WHERE s.ark_address = $1 AND s.brand = $2 FOR UPDATE
	", STORE_COLUMNS), &[&ark_address, &op.brand.as_str()]).await?
		.map(|r| Store::from_row(&r))
		.ok_or_else(|| not_found("no store of this brand has that Ark address"))?;
	let old = current.lightning_address_name.as_deref().map(|n| (&op.brand, n));
	let new = Some((&op.brand, req.name.as_str()));
	names::lock(&tx, &old.into_iter().chain(new).collect::<Vec<_>>()).await?;
	names::change(&tx, current.id, old, new, true).await?;
	let row = tx.query_one(&format!("
		UPDATE store s SET lightning_address_name = $2, updated_at = NOW()
		WHERE s.id = $1
		RETURNING {}
	", STORE_COLUMNS), &[&current.id, &req.name]).await.map_err(lightning_name_taken)?;
	tx.commit().await?;
	Ok(Json(StoreView::new(&app, &Store::from_row(&row)).await))
}
