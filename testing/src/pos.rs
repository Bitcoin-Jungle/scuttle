//! Runs posd, the point-of-sale service, inside the test process.

use std::sync::Arc;
use std::time::Duration;

use bitcoin::Network;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use pos::brand::BrandKey;
use pos::config::{self, Config};
use pos::rails::onchain::BitcoindConfig;

use crate::{Barkd, Bitcoind, Captaind, TestContext};
use crate::ports::pick_port;


/// The bitcoin price posd uses in tests: 62,500,000.00 per bitcoin in every
/// currency, so 12,500.00 is 20,000 sats.
pub const TEST_BTC_PRICE: &str = "62500000.00";

/// The test posd's two brands, told apart by host as in production. `ALPHA`
/// is on 127.0.0.1, `BETA` on localhost, with assets of its own.
pub const ALPHA: &str = "alpha";
pub const BETA: &str = "beta";

/// The brand stylesheet of `BETA`.
pub const BETA_CSS: &str = ":root { --pos-accent: #123456; }\n";

pub fn brand(key: &str) -> BrandKey {
	key.parse().expect("a valid brand key")
}

/// posd's config for a test database.
pub fn postgres_config(cfg: &server::config::Postgres) -> config::Postgres {
	config::Postgres {
		host: cfg.host.clone(),
		port: cfg.port,
		name: cfg.name.clone(),
		user: cfg.user.clone(),
		password: cfg.password.as_ref().map(|p| p.leak_ref().clone()),
		max_connections: 5,
	}
}

pub struct Posd {
	pub app: Arc<pos::App>,
	/// posd's own bitcoind, so its watch-only wallet does not disturb the
	/// wallet the tests fund from. None in the esplora lane.
	pub bitcoind: Option<Bitcoind>,
	pub url: String,
	http: reqwest::Client,
	server: tokio::task::JoinHandle<()>,
	watcher: tokio::task::JoinHandle<()>,
}

/// The operator token of `brand` on the test posd.
pub fn admin_token(brand: &str) -> String {
	format!("test-operator-token-for-{}-0123456789", brand)
}

impl Drop for Posd {
	fn drop(&mut self) {
		self.server.abort();
		self.watcher.abort();
	}
}

impl Posd {
	/// The base URL of `key`'s domain on this posd.
	pub fn brand_url(&self, key: &str) -> String {
		self.app.cfg.brand(&brand(key)).expect("the test posd serves both brands").public_url.clone()
	}

	/// Starts posd against `srv`, with `forwarder` as its barkd.
	pub async fn start(ctx: &TestContext, srv: &Captaind, forwarder: &Barkd) -> Posd {
		Self::start_with(ctx, srv, forwarder, |_| {}).await
	}

	/// Starts posd with a config `change` makes.
	pub async fn start_with(
		ctx: &TestContext,
		srv: &Captaind,
		forwarder: &Barkd,
		change: impl FnOnce(&mut Config),
	) -> Posd {
		let port = pick_port();
		let beta_assets = ctx.datadir.join("posd-brand-beta");
		std::fs::create_dir_all(&beta_assets).unwrap();
		std::fs::write(beta_assets.join("brand.css"), BETA_CSS).unwrap();
		let url = format!("http://127.0.0.1:{}", port);
		let bitcoind = match ctx.electrs {
			Some(_) => None,
			None => {
				let mut btc_cfg = ctx.bitcoind_default_cfg("posd_bitcoind");
				btc_cfg.wallet = true;
				Some(ctx.new_bitcoind_with_cfg("posd_bitcoind", btc_cfg).await)
			},
		};
		let mut cfg = Config {
			network: Network::Regtest,
			http: config::Http {
				listen: format!("127.0.0.1:{}", port).parse().unwrap(),
				trust_forwarded_host: false,
			},
			brands: [
				(brand(ALPHA), config::Brand {
					name: "Alpha".into(),
					app_name: None,
					short_name: None,
					public_url: url.clone(),
					theme_color: None,
					theme_color_dark: None,
					assets: None,
					admin_token: Some(admin_token(ALPHA)),
				}),
				(brand(BETA), config::Brand {
					name: "Beta".into(),
					app_name: Some("Beta Till".into()),
					short_name: None,
					public_url: format!("http://localhost:{}", port),
					theme_color: Some("#123456".into()),
					theme_color_dark: None,
					assets: Some(beta_assets),
					admin_token: Some(admin_token(BETA)),
				}),
			].into_iter().collect(),
			postgres: postgres_config(&ctx.new_postgres(&format!("{}_posd", ctx.test_name.replace('/', "_"))).await),
			ark_server_url: srv.ark_url(),
			barkd: config::Barkd {
				url: forwarder.base_url(),
				token: Some(forwarder.auth_token()),
			},
			// The esplora test lane covers the Esplora chain source.
			bitcoind: bitcoind.as_ref().map(|bitcoind| BitcoindConfig {
				url: bitcoind.rpc_url(),
				cookie: Some(bitcoind.rpc_cookie()),
				rpc_user: None,
				rpc_password: None,
				wallet: "posd".into(),
			}),
			esplora_url: ctx.electrs.as_ref().map(|e| e.rest_url()),
			price: config::Price {
				source: config::PriceSource::Fixed,
				fixed_btc_price: Some(TEST_BTC_PRICE.into()),
				currencies: Some([("CRC".to_string(), 2), ("USD".to_string(), 2)].into()),
				..Default::default()
			},
			sales: config::Sales {
				poll_interval_ms: 200,
				..Default::default()
			},
		};
		change(&mut cfg);
		let listen = cfg.http.listen;
		let app = pos::App::new(cfg).await.map_err(|e| format!("{:#}", e)).expect("posd failed to start");
		let watcher = tokio::spawn(pos::watcher::run(app.clone()));
		let listener = tokio::net::TcpListener::bind(listen).await.unwrap();
		let router = pos::api::router(app.clone());
		let server = tokio::spawn(async move {
			axum::serve(listener, router).await.unwrap();
		});
		Posd { app, bitcoind, url, http: reqwest::Client::new(), server, watcher }
	}

	/// Calls the API. Returns the status and the JSON body (`Null` if
	/// there is none).
	pub async fn call(
		&self,
		method: Method,
		path: &str,
		token: Option<&str>,
		body: Option<Value>,
	) -> (StatusCode, Value) {
		let mut req = self.http.request(method, format!("{}{}", self.url, path));
		if let Some(token) = token {
			req = req.bearer_auth(token);
		}
		if let Some(body) = body {
			req = req.json(&body);
		}
		let resp = req.send().await.expect("posd request failed");
		let status = resp.status();
		let text = resp.text().await.unwrap();
		let json = serde_json::from_str(&text).unwrap_or(Value::String(text));
		(status, json)
	}

	/// Calls the API and expects a 2xx answer.
	pub async fn ok(&self, method: Method, path: &str, token: Option<&str>, body: Option<Value>) -> Value {
		let (status, json) = self.call(method, path, token, body).await;
		assert!(status.is_success(), "{} {} failed with {}: {}", "posd", path, status, json);
		json
	}

	/// Registers a store.
	pub async fn register(&self, registration: Value, signature: String) {
		self.ok(Method::POST, "/api/v1/stores", None, Some(json!({
			"registration": registration, "signature": signature,
		}))).await;
	}

	/// The login challenge for the owner of `ark_address`.
	pub async fn challenge(&self, ark_address: &str) -> String {
		self.ok(Method::POST, "/api/v1/owner/challenge", None, Some(json!({
			"ark_address": ark_address,
		}))).await["challenge"].as_str().unwrap().to_string()
	}

	/// Logs in with a signed challenge and returns the owner token.
	pub async fn login(&self, ark_address: &str, challenge: &str, signature: String) -> String {
		let login = self.ok(Method::POST, "/api/v1/owner/login", None, Some(json!({
			"ark_address": ark_address, "challenge": challenge, "signature": signature,
		}))).await;
		login["token"].as_str().unwrap().to_string()
	}

	/// Pairs a terminal through a new invite and returns its token.
	pub async fn pair(&self, owner_token: &str, device_name: &str) -> String {
		let invite = self.ok(Method::POST, "/api/v1/owner/invites", Some(owner_token), None).await;
		let paired = self.ok(Method::POST, "/api/v1/pair", None, Some(json!({
			"code": invite["code"], "device_name": device_name,
		}))).await;
		paired["token"].as_str().unwrap().to_string()
	}

	/// Waits until the sale reaches `state`, or panics after `timeout`. It
	/// follows the sale with long-polls, as a terminal does.
	pub async fn wait_sale_state(&self, token: &str, id: &str, state: &str, timeout: Duration) -> Value {
		let deadline = tokio::time::Instant::now() + timeout;
		let mut sale = self.ok(Method::GET, &format!("/api/v1/sales/{}", id), Some(token), None).await;
		loop {
			if sale["state"] == state {
				return sale;
			}
			if tokio::time::Instant::now() > deadline {
				panic!("sale {} is {} after {:?}, expected {}", id, sale["state"], timeout, state);
			}
			let path = format!("/api/v1/sales/{}?wait={}", id, sale["state"].as_str().unwrap());
			sale = self.ok(Method::GET, &path, Some(token), None).await;
		}
	}
}
