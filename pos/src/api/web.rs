//! Serves the terminal PWA, embedded at build time (see `build.rs`), in the
//! look of a brand.
//!
//! posd writes the brand into each page before it is sent: the brand's
//! stylesheet, icons and browser colours, and what the app needs to know
//! about every brand, so the first paint is already in the brand. A page
//! is in the brand of the domain it was asked for, and a pairing page in
//! the brand of the code's store.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use serde_json::json;

use crate::App;
use crate::api::request_host;
use crate::brand::{self, BrandInfo, BrandKey, DEFAULT_KEY};
use crate::token;


#[derive(RustEmbed)]
#[folder = "$OUT_DIR/web-dist/"]
struct WebDist;

/// The neutral brand's files, from the source tree, so posd has them even
/// when it is built without the terminal.
#[derive(RustEmbed)]
#[folder = "web/public/brands/default/"]
struct NeutralBrand;

/// The brand of the domain a request was made to, or the neutral brand.
fn host_brand<'a>(app: &'a App, headers: &HeaderMap) -> &'a BrandInfo {
	let key = app.cfg.brand_for_host(&request_host(app, headers)).map(|k| k.as_str()).unwrap_or(DEFAULT_KEY);
	&app.brands[key]
}

/// `GET /pair/<code>`: the PWA, which reads the code from the path, in the
/// brand of the code's store.
pub async fn pair_index(
	State(app): State<Arc<App>>,
	headers: HeaderMap,
	Path(code): Path<String>,
) -> Response {
	let brand = pairing_brand(&app, &code).await.unwrap_or_else(|| host_brand(&app, &headers));
	serve_index(&app, brand)
}

/// The PWA, with `brand` written into its head.
fn serve_index(app: &App, brand: &BrandInfo) -> Response {
	let Some(file) = WebDist::get("index.html") else {
		return (StatusCode::NOT_FOUND, "posd was built without the terminal").into_response();
	};
	let html = branded_page(&String::from_utf8_lossy(&file.data), brand, &app.brands);
	(
		[(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")],
		html,
	).into_response()
}

/// `html` in `brand`, with what the app knows of every brand in `brands`.
fn branded_page(html: &str, brand: &BrandInfo, brands: &BTreeMap<String, BrandInfo>) -> String {
	// The JSON sits in a script element, which "</" or "<!--" would end or
	// change: JSON allows "<" as \u003c anywhere.
	let data = json!({ "current": brand.key, "brands": brands }).to_string()
		.replace('<', "\\u003c");
	let head = format!(
		concat!(
			"<link rel=\"stylesheet\" href=\"{css}\" data-brand-css />\n",
			"<link rel=\"icon\" type=\"image/png\" href=\"{icon}\" data-brand-icon />\n",
			"<link rel=\"apple-touch-icon\" href=\"{icon}\" data-brand-icon />\n",
			"<meta name=\"theme-color\" content=\"{light}\" media=\"(prefers-color-scheme: light)\" />\n",
			"<meta name=\"theme-color\" content=\"{dark}\" media=\"(prefers-color-scheme: dark)\" />\n",
			"<script id=\"pos-brands\" type=\"application/json\">{data}</script>\n",
			"</head>",
		),
		css = brand::asset_url(&brand.key, "brand.css"),
		icon = brand::asset_url(&brand.key, "icon-192.png"),
		light = html_attr(&brand.theme_color),
		dark = html_attr(&brand.theme_color_dark),
		data = data,
	);
	html.replacen("<html lang=\"en\">", &format!("<html lang=\"en\" data-brand=\"{}\">", brand.key), 1)
		.replacen("</head>", &head, 1)
}

/// A config value in an HTML attribute.
fn html_attr(s: &str) -> String {
	s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;")
}

fn valid_code(code: &str) -> bool {
	!code.is_empty() && code.len() <= 64 && code.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The brand of the store a pairing code belongs to, if it is configured.
/// A redeemed or expired code still gives its brand: an installed terminal
/// reopens its start URL, which carries the code.
async fn pairing_brand<'a>(app: &'a App, code: &str) -> Option<&'a BrandInfo> {
	if !valid_code(code) {
		return None;
	}
	let key = async {
		let conn = app.db.conn().await?;
		let row = conn.query_opt(
			"SELECT s.brand FROM invite i JOIN store s ON s.id = i.store_id WHERE i.code_hash = $1",
			&[&token::hash(code)],
		).await?;
		anyhow::Ok(row.and_then(|r| r.get::<_, &str>("brand").parse::<BrandKey>().ok()))
	}.await;
	match key {
		Ok(key) => {
			let key = key?;
			app.cfg.brand(&key)?;
			app.brands.get(key.as_str())
		},
		Err(e) => {
			tracing::warn!("failed to look up the brand of a pairing code: {:#}", e);
			None
		},
	}
}

fn manifest_response(brand: &BrandInfo, start_url: &str) -> Response {
	let manifest = json!({
		"name": brand.app_name,
		"short_name": brand.short_name,
		"start_url": start_url,
		"scope": "/",
		"display": "standalone",
		"orientation": "portrait",
		"background_color": brand.theme_color,
		"theme_color": brand.theme_color,
		"icons": [
			{ "src": brand::asset_url(&brand.key, "icon-192.png"), "sizes": "192x192", "type": "image/png" },
			{ "src": brand::asset_url(&brand.key, "icon-512.png"), "sizes": "512x512", "type": "image/png" },
		],
	});
	(
		[(header::CONTENT_TYPE, "application/manifest+json"), (header::CACHE_CONTROL, "no-cache")],
		manifest.to_string(),
	).into_response()
}

/// The manifest of a terminal installed from the site root, in the brand of
/// the domain.
pub async fn manifest(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
	manifest_response(host_brand(&app, &headers), "/")
}

/// The manifest of an install from a pairing link. Its `start_url` carries
/// the code, so the installed app opens with it: on iOS the installed app
/// does not share storage with the browser that opened the link.
pub async fn pair_manifest(
	State(app): State<Arc<App>>,
	headers: HeaderMap,
	Path(code): Path<String>,
) -> Response {
	if !valid_code(&code) {
		return StatusCode::NOT_FOUND.into_response();
	}
	let brand = pairing_brand(&app, &code).await.unwrap_or_else(|| host_brand(&app, &headers));
	manifest_response(brand, &format!("/pair/{}", code))
}

fn content_type(file: &str) -> &'static str {
	match file.rsplit_once('.').map(|(_, ext)| ext) {
		Some("css") => "text/css; charset=utf-8",
		Some("svg") => "image/svg+xml",
		Some("png") => "image/png",
		Some("woff2") => "font/woff2",
		Some("woff") => "font/woff",
		_ => "application/octet-stream",
	}
}

/// `GET /brands/<key>/<file>`: a file of a brand's asset directory, or the
/// neutral brand's file of that name.
pub async fn brand_asset(
	State(app): State<Arc<App>>,
	Path((key, file)): Path<(String, String)>,
) -> Response {
	// The neutral brand's key, or one posd serves.
	let brand = match key.parse::<BrandKey>() {
		Ok(key) if key.as_str() == DEFAULT_KEY => None,
		Ok(key) => match app.cfg.brand(&key) {
			Some(brand) => Some(brand),
			None => return StatusCode::NOT_FOUND.into_response(),
		},
		Err(_) => return StatusCode::NOT_FOUND.into_response(),
	};
	if !brand::servable(&file) {
		return StatusCode::NOT_FOUND.into_response();
	}
	let configured = brand.and_then(|b| b.asset(&file));
	let data = match configured {
		Some(path) => match tokio::fs::read(&path).await {
			Ok(data) => data,
			Err(e) => {
				tracing::warn!("failed to read brand asset {}: {}", path.display(), e);
				return StatusCode::NOT_FOUND.into_response();
			},
		},
		None => match NeutralBrand::get(&file) {
			Some(f) => f.data.into_owned(),
			None => return StatusCode::NOT_FOUND.into_response(),
		},
	};
	let mut resp = (
		[
			(header::CONTENT_TYPE, content_type(&file)),
			// An operator can change a brand's files, so they are not
			// cached for long.
			(header::CACHE_CONTROL, "public, max-age=300"),
		],
		data,
	).into_response();
	if file.ends_with(".svg") {
		// An SVG opened on its own runs as a page of this origin: no scripts.
		resp.headers_mut().insert(
			header::CONTENT_SECURITY_POLICY,
			"default-src 'none'; style-src 'unsafe-inline'".parse().expect("valid header"),
		);
	}
	resp
}

/// Static assets. Unknown paths outside the API fall back to the PWA, so
/// its client-side routes work on reload.
pub async fn asset(State(app): State<Arc<App>>, headers: HeaderMap, uri: Uri) -> Response {
	let path = uri.path().trim_start_matches('/');
	if path.starts_with("api/") || path.starts_with(".well-known/") {
		return StatusCode::NOT_FOUND.into_response();
	}
	if path.is_empty() || path == "index.html" {
		return serve_index(&app, host_brand(&app, &headers));
	}
	match WebDist::get(path) {
		Some(file) => {
			// Vite hashes the names of bundled assets, so they never change.
			let cache = if path.starts_with("assets/") {
				"public, max-age=31536000, immutable"
			} else {
				"no-cache"
			};
			(
				[
					(header::CONTENT_TYPE, file.metadata.mimetype().to_string()),
					(header::CACHE_CONTROL, cache.to_string()),
				],
				file.data,
			).into_response()
		},
		None if path.starts_with("assets/") || path.contains('.') => StatusCode::NOT_FOUND.into_response(),
		None => serve_index(&app, host_brand(&app, &headers)),
	}
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn writes_the_brand_into_the_page() {
		let mut brand = BrandInfo::neutral();
		brand.key = "shop".into();
		brand.name = "</script><!-- \"Shop\"".into();
		brand.theme_color = "#fff\"><script>".into();
		let brands = [("shop".to_string(), brand.clone())].into_iter().collect();
		let page = branded_page("<html lang=\"en\"><head><title>x</title></head><body></body></html>", &brand, &brands);

		assert!(page.starts_with("<html lang=\"en\" data-brand=\"shop\"><head>"), "{}", page);
		assert!(page.contains("href=\"/brands/shop/brand.css\" data-brand-css"));
		assert!(page.contains("content=\"#fff&quot;>&lt;script>\""), "attributes are escaped: {}", page);
		// Nothing in the data can end or change the script element.
		let data = page.split("type=\"application/json\">").nth(1).unwrap().split("</script>").next().unwrap();
		assert!(!data.contains('<'), "{}", data);
		let parsed: serde_json::Value = serde_json::from_str(data).unwrap();
		assert_eq!(parsed["current"], "shop");
		assert_eq!(parsed["brands"]["shop"]["name"], "</script><!-- \"Shop\"");
		assert!(page.ends_with("</head><body></body></html>"));
	}
}
