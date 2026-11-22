//! The posd HTTP API.
//!
//! The OpenAPI document is generated from the handlers: every documented
//! route is registered with [routes!] from its `#[utoipa::path]`, so the
//! spec and the router cannot disagree. `pos/openapi.json` is the committed
//! copy; a test fails when it is stale. The prose guide is `pos/docs/api.md`.

mod admin;
pub mod lnurl;
mod owner;
mod terminal;
mod web;

use std::sync::Arc;

use axum::Router;
use axum::extract::FromRequestParts;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::routing::{any, get};
use utoipa::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::App;
use crate::brand::BrandKey;
use crate::database::{Device, STORE_COLUMNS, Store};
use crate::error::ApiError;
use crate::token;


#[derive(OpenApi)]
#[openapi(
	info(
		title = "posd",
		description = "The point-of-sale and Lightning address service. The guide, with the \
			signed message formats and how the rails work, is pos/docs/api.md.",
		license(name = "MIT", identifier = "MIT"),
	),
	servers(
		(url = "/", description = "A brand's public_url, on the posd that serves it. Each \
			brand's domain has its own Lightning addresses."),
	),
	modifiers(&SecuritySchemes),
	tags(
		(name = "terminal", description = "A paired terminal, with its device token."),
		(name = "owner", description = "A store's owner wallet, which signs with its Ark key."),
		(name = "operator", description = "A brand's own services, with the brand's admin_token."),
		(name = "lnurl", description = "LUD-06 and LUD-16 Lightning addresses, for any payer."),
	),
)]
struct ApiDoc;

/// One bearer scheme per caller. Handlers name them in `security(...)`.
struct SecuritySchemes;

impl utoipa::Modify for SecuritySchemes {
	fn modify(&self, api: &mut utoipa::openapi::OpenApi) {
		let components = api.components.get_or_insert_with(Default::default);
		for (name, what) in [
			("device_token", "The device token a terminal gets from pairing."),
			("owner_token", "The owner token from a signed login. It lasts 24 hours."),
			("admin_token", "A brand's admin_token from the posd config."),
		] {
			components.add_security_scheme(name, SecurityScheme::Http(
				HttpBuilder::new().scheme(HttpAuthScheme::Bearer).description(Some(what)).build(),
			));
		}
	}
}

/// The documented routes, with the OpenAPI document they make.
fn documented() -> OpenApiRouter<Arc<App>> {
	let api = OpenApiRouter::new()
		.routes(routes!(terminal::pair))
		.routes(routes!(terminal::terminal))
		.routes(routes!(terminal::rate))
		.routes(routes!(terminal::create_sale, terminal::list_sales))
		.routes(routes!(terminal::sales_csv))
		.routes(routes!(terminal::get_sale))
		.routes(routes!(terminal::cancel_sale))
		.routes(routes!(owner::register))
		.routes(routes!(owner::challenge))
		.routes(routes!(owner::login))
		.routes(routes!(owner::get_store, owner::update_store))
		.routes(routes!(owner::create_invite))
		.routes(routes!(owner::list_devices))
		.routes(routes!(owner::revoke_device))
		.routes(routes!(owner::list_sales))
		.routes(routes!(owner::sales_csv))
		.routes(routes!(lnurl::callback))
		.routes(routes!(admin::reserve_names))
		.routes(routes!(admin::assign_lightning_address));
	OpenApiRouter::with_openapi(ApiDoc::openapi())
		.nest("/api/v1", api)
		.routes(routes!(lnurl::pay_request))
}

/// The OpenAPI document of the API.
pub fn openapi() -> utoipa::openapi::OpenApi {
	documented().into_openapi()
}

/// The OpenAPI document as `pos/openapi.json` holds it.
pub fn openapi_json() -> String {
	let mut api = openapi();
	// The version is the crate's, which the info macro cannot see.
	api.info.version = env!("CARGO_PKG_VERSION").to_string();
	serde_json::to_string_pretty(&api).expect("serializes")
}

pub fn router(app: Arc<App>) -> Router {
	let (router, _) = documented().split_for_parts();
	let spec = openapi_json();
	router
		.route("/api/v1/openapi.json", get(move || async move {
			([(axum::http::header::CONTENT_TYPE, "application/json")], spec)
		}))
		.route("/api/v1/{*rest}", any(|| async { ApiError::NotFound("no such endpoint".into()) }))
		.route("/pair/{code}/manifest.webmanifest", get(web::pair_manifest))
		.route("/pair/{code}", get(web::pair_index))
		.route("/manifest.webmanifest", get(web::manifest))
		.route("/brands/{brand}/{file}", get(web::brand_asset))
		.fallback(web::asset)
		.with_state(app)
}

fn bearer(parts: &Parts) -> Result<&str, ApiError> {
	parts.headers.get(AUTHORIZATION)
		.and_then(|v| v.to_str().ok())
		.and_then(|v| v.strip_prefix("Bearer "))
		.map(str::trim)
		.filter(|t| !t.is_empty())
		.ok_or(ApiError::Unauthorized)
}

/// A brand's operator, from the brand's `admin_token`.
pub struct Operator {
	pub brand: BrandKey,
}

impl FromRequestParts<Arc<App>> for Operator {
	type Rejection = ApiError;

	async fn from_request_parts(parts: &mut Parts, app: &Arc<App>) -> Result<Self, Self::Rejection> {
		// Compare hashes, so the comparison takes no longer for a closer guess.
		let hash = token::hash(bearer(parts)?);
		app.cfg.brands.iter()
			.find(|(_, b)| b.admin_token.as_deref().is_some_and(|t| token::hash(t) == hash))
			.map(|(key, _)| Operator { brand: key.clone() })
			.ok_or(ApiError::Unauthorized)
	}
}

/// A paired terminal, from its bearer token.
pub struct Terminal {
	pub device: Device,
	pub store: Store,
}

impl FromRequestParts<Arc<App>> for Terminal {
	type Rejection = ApiError;

	async fn from_request_parts(parts: &mut Parts, app: &Arc<App>) -> Result<Self, Self::Rejection> {
		let hash = token::hash(bearer(parts)?);
		let conn = app.db.conn().await?;
		let row = conn.query_opt(&format!("
			SELECT {}, d.id AS device_id, d.name AS device_name,
				d.created_at AS device_created_at, d.revoked_at AS device_revoked_at
			FROM device d JOIN store s ON s.id = d.store_id
			WHERE d.token_hash = $1 AND d.revoked_at IS NULL
		", STORE_COLUMNS), &[&hash]).await?.ok_or(ApiError::Unauthorized)?;
		Ok(Terminal {
			device: Device {
				id: row.get("device_id"),
				name: row.get("device_name"),
				created_at: row.get("device_created_at"),
				revoked_at: row.get("device_revoked_at"),
			},
			store: Store::from_row(&row),
		})
	}
}

/// The owner of a store, from an owner session token.
pub struct Owner {
	pub store: Store,
}

impl FromRequestParts<Arc<App>> for Owner {
	type Rejection = ApiError;

	async fn from_request_parts(parts: &mut Parts, app: &Arc<App>) -> Result<Self, Self::Rejection> {
		let hash = token::hash(bearer(parts)?);
		let conn = app.db.conn().await?;
		let row = conn.query_opt(&format!("
			SELECT {} FROM owner_session o JOIN store s ON s.id = o.store_id
			WHERE o.token_hash = $1 AND o.expires_at > NOW()
		", STORE_COLUMNS), &[&hash]).await?.ok_or(ApiError::Unauthorized)?;
		Ok(Owner { store: Store::from_row(&row) })
	}
}

/// How a store looks to the API.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
#[schema(as = Store)]
pub struct StoreView {
	#[schema(example = "Corner Café")]
	pub name: String,
	#[schema(example = "CRC")]
	pub currency: String,
	/// The decimal places a sale in `currency` may have.
	#[schema(example = 2)]
	pub currency_decimals: u32,
	/// Sales below this many sats offer no on-chain address.
	#[schema(example = 50000)]
	pub onchain_min_sat: i64,
	/// `null` when the store has none. Its domain is the host of the
	/// brand's public URL.
	#[schema(example = "cornercafe@pay.example.com")]
	pub lightning_address: Option<String>,
	/// The key of the store's brand in posd's config.
	pub brand: BrandKey,
}

impl StoreView {
	pub fn new(app: &App, store: &Store) -> StoreView {
		StoreView {
			name: store.name.clone(),
			currency: store.currency.clone(),
			// A currency the price source stopped quoting: sales in it fail,
			// and amounts show with the usual 2.
			currency_decimals: app.price.decimals(&store.currency).unwrap_or(2),
			onchain_min_sat: store.onchain_min_sat,
			lightning_address: store.lightning_address_name.as_ref()
				.zip(app.cfg.brand(&store.brand))
				.map(|(n, brand)| format!("{}@{}", n, brand.host())),
			brand: store.brand.clone(),
		}
	}
}

/// The host a request was made to: `Host`, or the `X-Forwarded-Host` a
/// trusted proxy sets. It picks the brand of a page or a Lightning address.
pub fn request_host(app: &App, headers: &axum::http::HeaderMap) -> String {
	let forwarded = app.cfg.http.trust_forwarded_host
		.then(|| headers.get("x-forwarded-host")).flatten();
	forwarded.or_else(|| headers.get(axum::http::header::HOST))
		.and_then(|h| h.to_str().ok())
		.map(|h| h.split(',').next().unwrap_or("").trim().to_string())
		.unwrap_or_default()
}

#[cfg(test)]
mod test {
	/// `pos/openapi.json` is generated: `just dump-posd-openapi` rewrites
	/// it, and this fails while it is stale.
	#[test]
	fn openapi_is_current() {
		let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("openapi.json");
		let generated = super::openapi_json() + "\n";
		if std::env::var_os("UPDATE_OPENAPI").is_some() {
			std::fs::write(&path, &generated).unwrap();
			return;
		}
		let committed = std::fs::read_to_string(&path).unwrap_or_default();
		assert!(committed == generated,
			"pos/openapi.json is stale: run `just dump-posd-openapi`");
	}
}
