//! Owner endpoints: registration, login and store management.
//!
//! The owner is the Ark address that registered the store. It proves
//! control by signing with that address's key.

use std::str::FromStr;
use std::sync::Arc;

use anyhow::Context;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use bitcoin::secp256k1::schnorr;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::App;
use crate::api::terminal::{CsvQuery, ListQuery, SalesPage, csv, list};
use crate::api::{Owner, OwnerStoreView};
use crate::brand::BrandKey;
use crate::database::{Device, STORE_COLUMNS, Store};
use crate::error::{ApiError, ApiResult, ErrorBody, bad_request, conflict, not_found};
use crate::money::MAX_SAT;
use crate::names;
use crate::rails::onchain::StoreDescriptor;
use crate::registration::Registration;
use crate::{silent_payment, token};


/// How far a registration timestamp may be from the server clock.
const REGISTRATION_CLOCK_SKEW_SECS: u64 = 600;
const CHALLENGE_LIFETIME_MINS: i64 = 5;
const SESSION_LIFETIME_HOURS: i64 = 24;
const INVITE_LIFETIME_HOURS: i64 = 24;

fn parse_signature(hex: &str) -> ApiResult<schnorr::Signature> {
	schnorr::Signature::from_str(hex).map_err(|_| bad_request("the signature is not valid hex"))
}

#[derive(Deserialize, ToSchema)]
pub struct RegisterRequest {
	registration: Registration,
	/// The hex BIP 340 signature of the registration message by the key of
	/// `ark_address`, with bark's message signing.
	signature: String,
}

#[derive(Serialize, ToSchema)]
pub struct StoreResponse {
	store: OwnerStoreView,
}

/// Registers a store, signed by the owner's Ark key.
///
/// The signed message is these lines, joined by `\n`, with no trailing
/// newline: `posd registration v1`, then `name:`, `ark_address:`,
/// `onchain_descriptor:`, `silent_payment_address:`, `currency:`, `brand:`,
/// `lightning_address_name:` and `timestamp:`, each followed by its field.
#[utoipa::path(
	post, path = "/stores", tag = "owner", security(()),
	request_body = RegisterRequest,
	responses(
		(status = 200, description = "The new store", body = StoreResponse),
		(status = 400, description = "The signature, timestamp, address, descriptor, silent \
			payment address, currency, brand or Lightning address name is not valid", body = ErrorBody),
		(status = 409, description = "A store is registered for the address, or the Lightning \
			address name is taken, reserved or held", body = ErrorBody),
	),
)]
pub async fn register(
	State(app): State<Arc<App>>,
	Json(req): Json<RegisterRequest>,
) -> ApiResult<Json<StoreResponse>> {
	let reg = req.registration;
	let signature = parse_signature(&req.signature)?;
	let info = app.ark.ark_info().await;
	let ark_address = reg.verify(&signature, info.server_pubkey)
		.map_err(|e| bad_request(e.to_string()))?;

	let now = Utc::now().timestamp().max(0) as u64;
	if now.abs_diff(reg.timestamp) > REGISTRATION_CLOCK_SKEW_SECS {
		return Err(bad_request("the registration timestamp is too far from the current time"));
	}
	let name = store_name(&reg.name)?;
	check_currency(&app, &reg.currency)?;
	check_brand(&app, &reg.brand)?;
	let ln_name = lightning_name(&reg.lightning_address_name)?;
	StoreDescriptor::parse_for(&reg.onchain_descriptor, app.cfg.network)
		.map_err(|e| bad_request(format!("{:#}", e)))?;
	let sp_address = match reg.silent_payment_address.as_str() {
		"" => None,
		a => Some(silent_payment::parse(a, app.cfg.network)
			.map_err(|e| bad_request(format!("silent_payment_address: {}", e)))?),
	};

	let mut conn = app.db.conn().await?;
	let tx = conn.transaction().await?;
	if let Some(n) = &ln_name {
		names::lock(&tx, &[(&reg.brand, n)]).await?;
		names::refuse_reserved(&tx, &reg.brand, n).await?;
		names::take_held(&tx, &reg.brand, n, None).await?;
	}
	let row = tx.query_opt(&format!("
		INSERT INTO store AS s (name, ark_address, onchain_descriptor, silent_payment_address,
			currency, brand, onchain_min_sat, lightning_address_name, registration_signature)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
		ON CONFLICT (ark_address) DO NOTHING
		RETURNING {}
	", STORE_COLUMNS), &[
		&name, &ark_address.to_string(), &reg.onchain_descriptor, &sp_address, &reg.currency,
		&reg.brand.as_str(), &(app.cfg.sales.default_onchain_min_sat as i64), &ln_name,
		&signature.to_string(),
	]).await.map_err(lightning_name_taken)?;
	let store = row.map(|r| Store::from_row(&r))
		.ok_or_else(|| conflict("a store is already registered for this Ark address"))?;
	tx.commit().await?;
	Ok(Json(StoreResponse { store: OwnerStoreView::new(&app, &store) }))
}

#[derive(Deserialize, ToSchema)]
pub struct ChallengeRequest {
	#[schema(example = "ark1…")]
	ark_address: String,
}

#[derive(Serialize, ToSchema)]
pub struct ChallengeResponse {
	/// The text to sign. It works once, within 5 minutes.
	challenge: String,
}

/// Starts an owner login: a challenge for the owner to sign.
///
/// posd keeps nothing for a challenge: it is authenticated by its last
/// line, `mac:`, which the owner signs with the rest.
#[utoipa::path(
	post, path = "/owner/challenge", tag = "owner", security(()),
	request_body = ChallengeRequest,
	responses(
		(status = 200, description = "The challenge to sign", body = ChallengeResponse),
		(status = 400, description = "The address is not a valid Ark address", body = ErrorBody),
	),
)]
pub async fn challenge(
	State(app): State<Arc<App>>,
	Json(req): Json<ChallengeRequest>,
) -> ApiResult<Json<ChallengeResponse>> {
	// The address as posd stores it, whatever case the client wrote it in.
	let address = ark::Address::from_str(&req.ark_address)
		.map_err(|_| bad_request("not a valid Ark address"))?.to_string();
	let expires_at = Utc::now() + Duration::minutes(CHALLENGE_LIFETIME_MINS);
	Ok(Json(ChallengeResponse { challenge: app.login_key.challenge(&address, expires_at) }))
}

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
	#[schema(example = "ark1…")]
	ark_address: String,
	/// The challenge, as returned.
	challenge: String,
	/// The hex BIP 340 signature of the challenge with bark's message signing.
	signature: String,
}

#[derive(Serialize, ToSchema)]
pub struct LoginResponse {
	/// The owner token. posd keeps only its hash, so it is shown once.
	token: String,
	expires_at: DateTime<Utc>,
}

/// Finishes an owner login with the signed challenge. A challenge works once.
#[utoipa::path(
	post, path = "/owner/login", tag = "owner", security(()),
	request_body = LoginRequest,
	responses(
		(status = 200, description = "The owner token", body = LoginResponse),
		(status = 400, description = "The signature does not verify, or the challenge is \
			not posd's, used or expired", body = ErrorBody),
		(status = 404, description = "No store is registered for the address", body = ErrorBody),
	),
)]
pub async fn login(
	State(app): State<Arc<App>>,
	Json(req): Json<LoginRequest>,
) -> ApiResult<Json<LoginResponse>> {
	let address = ark::Address::from_str(&req.ark_address)
		.map_err(|_| bad_request("not a valid Ark address"))?;
	let verified = app.login_key.verify(&req.challenge, &address.to_string(), Utc::now())
		.map_err(|e| bad_request(e.to_string()))?;
	let signature = parse_signature(&req.signature)?;
	if !ark::message::verify_for_address(&address, req.challenge.as_bytes(), &signature) {
		return Err(bad_request("the signature does not verify for this address"));
	}

	let mut conn = app.db.conn().await?;
	let tx = conn.transaction().await?;
	let store_id: i64 = tx.query_opt("SELECT id FROM store WHERE ark_address = $1", &[&address.to_string()])
		.await?.ok_or_else(|| not_found("no store is registered for this Ark address"))?
		.get("id");
	tx.execute("DELETE FROM used_login_nonce WHERE expires_at <= NOW()", &[]).await?;
	let fresh = tx.execute(
		"INSERT INTO used_login_nonce (nonce, expires_at) VALUES ($1, $2) ON CONFLICT DO NOTHING",
		&[&verified.nonce, &verified.expires_at],
	).await?;
	if fresh == 0 {
		return Err(bad_request("the challenge was used"));
	}
	let session_token = token::new_token();
	let expires_at = Utc::now() + Duration::hours(SESSION_LIFETIME_HOURS);
	tx.execute(
		"INSERT INTO owner_session (store_id, token_hash, expires_at) VALUES ($1, $2, $3)",
		&[&store_id, &token::hash(&session_token), &expires_at],
	).await?;
	tx.commit().await?;
	Ok(Json(LoginResponse { token: session_token, expires_at }))
}

/// The owner's store.
#[utoipa::path(
	get, path = "/owner/store", tag = "owner", security(("owner_token" = [])),
	responses((status = 200, description = "The store", body = OwnerStoreView), (status = 401, description = "The token is missing, not valid, or expired", body = ErrorBody),),
)]
pub async fn get_store(State(app): State<Arc<App>>, owner: Owner) -> Json<OwnerStoreView> {
	Json(OwnerStoreView::new(&app, &owner.store))
}

/// The fields to change. Fields left out stay as they are.
#[derive(Deserialize, ToSchema)]
pub struct UpdateStore {
	/// 1 to 64 characters.
	name: Option<String>,
	currency: Option<String>,
	brand: Option<BrandKey>,
	/// At most 21 million bitcoin.
	onchain_min_sat: Option<u64>,
	/// 1 to 64 lowercase letters, digits, `.`, `-` and `_`. An empty name
	/// removes the Lightning address. A name the store gives up, by a new
	/// name, an empty one or a move to another brand, is held for it for 90
	/// days: no other store can take it, and the store can take it back.
	#[schema(pattern = "^[a-z0-9._-]{0,64}$")]
	lightning_address_name: Option<String>,
}

pub(super) const LIGHTNING_NAME_RULE: &str =
	"a Lightning address name has 1 to 64 lowercase letters, digits, '.', '-' and '_'";

/// A taken Lightning address name is a conflict; other errors pass on.
pub(super) fn lightning_name_taken(e: tokio_postgres::Error) -> ApiError {
	if e.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION) {
		conflict("that Lightning address is taken")
	} else {
		e.into()
	}
}

/// A store name, trimmed: 1 to 64 characters.
fn store_name(name: &str) -> ApiResult<String> {
	let name = name.trim();
	if name.is_empty() || name.chars().count() > 64 {
		return Err(bad_request("the store name must be 1 to 64 characters"));
	}
	Ok(name.to_string())
}

/// Refuses a currency the price source does not quote.
fn check_currency(app: &App, currency: &str) -> ApiResult<()> {
	if app.price.decimals(currency).is_none() {
		let quoted = app.price.currencies().collect::<Vec<_>>().join(", ");
		return Err(bad_request(format!("unsupported currency {}: posd quotes {}", currency, quoted)));
	}
	Ok(())
}

fn check_brand(app: &App, brand: &BrandKey) -> ApiResult<()> {
	if app.cfg.brand(brand).is_none() {
		return Err(bad_request(format!("this service does not serve the brand {}", brand)));
	}
	Ok(())
}

/// A Lightning address name, or `None` for an empty one.
fn lightning_name(name: &str) -> ApiResult<Option<String>> {
	match name {
		"" => Ok(None),
		n if valid_lightning_name(n) => Ok(Some(n.to_string())),
		_ => Err(bad_request(LIGHTNING_NAME_RULE)),
	}
}

/// A Lightning address name: lowercase letters, digits, `.`, `-` and `_`.
pub(super) fn valid_lightning_name(name: &str) -> bool {
	!name.is_empty() && name.len() <= 64
		&& name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}

/// Changes the owner's store.
#[utoipa::path(
	patch, path = "/owner/store", tag = "owner", security(("owner_token" = [])),
	request_body = UpdateStore,
	responses(
		(status = 200, description = "The store", body = OwnerStoreView),
		(status = 400, description = "A field is not valid, or posd does not serve the brand", body = ErrorBody),
		(status = 401, description = "The token is missing, not valid, or expired", body = ErrorBody),
		(status = 409, description = "The Lightning address name is taken, reserved or held on \
			the store's domain", body = ErrorBody),
	),
)]
pub async fn update_store(
	State(app): State<Arc<App>>,
	owner: Owner,
	Json(req): Json<UpdateStore>,
) -> ApiResult<Json<OwnerStoreView>> {
	let name = req.name.as_deref().map(store_name).transpose()?;
	if let Some(c) = &req.currency {
		check_currency(&app, c)?;
	}
	if let Some(brand) = &req.brand {
		check_brand(&app, brand)?;
	}
	let ln_name = req.lightning_address_name.as_deref().map(lightning_name).transpose()?;
	let onchain_min_sat = req.onchain_min_sat
		.map(|v| if v <= MAX_SAT { Ok(v as i64) } else { Err(bad_request("onchain_min_sat is more than 21 million bitcoin")) })
		.transpose()?;

	let mut conn = app.db.conn().await?;
	let tx = conn.transaction().await?;
	// Lock the store, so two changes cannot each pass the name checks
	// against the store as it was.
	let current = tx.query_one(
		&format!("SELECT {} FROM store s WHERE s.id = $1 FOR UPDATE", STORE_COLUMNS),
		&[&owner.store.id],
	).await?;
	let current = Store::from_row(&current);
	// The address the store has, and the one it ends up with: a rename and
	// a move to another brand both change it.
	let brand = req.brand.clone().unwrap_or_else(|| current.brand.clone());
	let final_name = match &ln_name { Some(n) => n.clone(), None => current.lightning_address_name.clone() };
	let old = current.lightning_address_name.as_deref().map(|n| (&current.brand, n));
	let new = final_name.as_deref().map(|n| (&brand, n));
	names::lock(&tx, &old.into_iter().chain(new).collect::<Vec<_>>()).await?;
	names::change(&tx, current.id, old, new, false).await?;
	let row = tx.query_one(&format!("
		UPDATE store s SET
			name = coalesce($2, name),
			currency = coalesce($3, currency),
			onchain_min_sat = coalesce($4, onchain_min_sat),
			lightning_address_name = CASE WHEN $5 THEN $6 ELSE lightning_address_name END,
			brand = $7,
			updated_at = NOW()
		WHERE id = $1
		RETURNING {}
	", STORE_COLUMNS), &[
		&owner.store.id, &name, &req.currency, &onchain_min_sat, &ln_name.is_some(), &ln_name.flatten(),
		&brand.as_str(),
	]).await.map_err(lightning_name_taken)?;
	tx.commit().await?;
	Ok(Json(OwnerStoreView::new(&app, &Store::from_row(&row))))
}

#[derive(Serialize, ToSchema)]
pub struct Invite {
	code: String,
	/// The pairing link, on the store's brand's domain.
	#[schema(example = "https://pay.example.com/pair/ad13dcd6ba79d527ef933317")]
	url: String,
	/// 24 hours after it was made.
	expires_at: DateTime<Utc>,
}

/// Makes a single-use pairing link for a terminal.
#[utoipa::path(
	post, path = "/owner/invites", tag = "owner", security(("owner_token" = [])),
	responses((status = 200, description = "The pairing link", body = Invite), (status = 401, description = "The token is missing, not valid, or expired", body = ErrorBody),),
)]
pub async fn create_invite(State(app): State<Arc<App>>, owner: Owner) -> ApiResult<Json<Invite>> {
	let code = token::new_code();
	let expires_at = Utc::now() + Duration::hours(INVITE_LIFETIME_HOURS);
	let conn = app.db.conn().await?;
	conn.execute(
		"INSERT INTO invite (store_id, code_hash, expires_at) VALUES ($1, $2, $3)",
		&[&owner.store.id, &token::hash(&code), &expires_at],
	).await?;
	let brand = app.cfg.brand(&owner.store.brand)
		.ok_or_else(|| anyhow::anyhow!("the store's brand {} is not configured", owner.store.brand))?;
	let url = brand.url(&format!("/pair/{}", code));
	Ok(Json(Invite { code, url, expires_at }))
}

#[derive(Serialize, ToSchema)]
pub struct DeviceView {
	id: i64,
	#[schema(example = "Front till")]
	name: String,
	created_at: DateTime<Utc>,
	/// `null` while the device works.
	revoked_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, ToSchema)]
pub struct Devices {
	devices: Vec<DeviceView>,
}

/// The store's terminals, revoked ones too.
#[utoipa::path(
	get, path = "/owner/devices", tag = "owner", security(("owner_token" = [])),
	responses((status = 200, description = "The devices", body = Devices), (status = 401, description = "The token is missing, not valid, or expired", body = ErrorBody),),
)]
pub async fn list_devices(State(app): State<Arc<App>>, owner: Owner) -> ApiResult<Json<Devices>> {
	let conn = app.db.conn().await?;
	let rows = conn.query("
		SELECT id, name, created_at, revoked_at FROM device
		WHERE store_id = $1 ORDER BY id
	", &[&owner.store.id]).await?;
	let devices = rows.iter().map(Device::from_row).map(|d| DeviceView {
		id: d.id, name: d.name, created_at: d.created_at, revoked_at: d.revoked_at,
	}).collect();
	Ok(Json(Devices { devices }))
}

/// Revokes a terminal. Its next request is `401`.
#[utoipa::path(
	delete, path = "/owner/devices/{id}", tag = "owner", security(("owner_token" = [])),
	params(("id" = i64, Path, description = "The device id")),
	responses(
		(status = 204, description = "Revoked, now or before"),
		(status = 401, description = "The token is missing, not valid, or expired", body = ErrorBody),
		(status = 404, description = "No such device in this store", body = ErrorBody),
	),
)]
pub async fn revoke_device(
	State(app): State<Arc<App>>,
	owner: Owner,
	Path(id): Path<i64>,
) -> ApiResult<impl IntoResponse> {
	let conn = app.db.conn().await?;
	let n = conn.execute("
		UPDATE device SET revoked_at = coalesce(revoked_at, NOW())
		WHERE id = $1 AND store_id = $2
	", &[&id, &owner.store.id]).await.context("revoke failed")?;
	if n == 0 {
		return Err(not_found("no such device"));
	}
	Ok(StatusCode::NO_CONTENT)
}

/// The store's sales from every terminal and its Lightning address, newest first.
#[utoipa::path(
	get, path = "/owner/sales", operation_id = "owner_list_sales", tag = "owner", security(("owner_token" = [])),
	params(ListQuery),
	responses((status = 200, description = "A page of sales", body = SalesPage), (status = 401, description = "The token is missing, not valid, or expired", body = ErrorBody),),
)]
pub async fn list_sales(
	State(app): State<Arc<App>>,
	owner: Owner,
	Query(q): Query<ListQuery>,
) -> ApiResult<Json<SalesPage>> {
	list(&app, &owner.store, q).await
}

/// The store's sales as CSV, with the columns of the terminal's export.
#[utoipa::path(
	get, path = "/owner/sales.csv", operation_id = "owner_sales_csv", tag = "owner", security(("owner_token" = [])),
	params(CsvQuery),
	responses(
		(status = 200, description = "The CSV", content_type = "text/csv", body = String),
		(status = 400, description = "A date is not valid", body = ErrorBody),
		(status = 401, description = "The token is missing, not valid, or expired", body = ErrorBody),
	),
)]
pub async fn sales_csv(
	State(app): State<Arc<App>>,
	owner: Owner,
	Query(q): Query<CsvQuery>,
) -> ApiResult<axum::response::Response> {
	csv(&app, &owner.store, q).await
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn lightning_names() {
		assert!(valid_lightning_name("corner.cafe_1-2"));
		assert!(!valid_lightning_name("Cafe"));
		assert!(!valid_lightning_name(""));
		assert!(!valid_lightning_name("a b"));
		assert!(!valid_lightning_name("a@b"));
	}
}
