use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use bitcoin::Network;
use serde::{Deserialize, Serialize};

use crate::brand::{BrandKey, DEFAULT_KEY};
use crate::money::Decimal;


/// A Postgres connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Postgres {
	pub host: String,
	#[serde(default = "default_postgres_port")]
	pub port: u16,
	pub name: String,
	pub user: Option<String>,
	#[serde(skip_serializing)]
	pub password: Option<String>,
	#[serde(default = "default_max_connections")]
	pub max_connections: u32,
}

fn default_postgres_port() -> u16 { 5432 }
fn default_max_connections() -> u32 { 10 }

impl Postgres {
	pub fn to_pg_config(&self) -> tokio_postgres::Config {
		let mut cfg = tokio_postgres::Config::new();
		cfg.host(&self.host).port(self.port).dbname(&self.name);
		if let Some(user) = &self.user {
			cfg.user(user);
		}
		if let Some(password) = &self.password {
			cfg.password(password);
		}
		cfg
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Http {
	/// The address posd listens on.
	pub listen: SocketAddr,
	/// Take the request's domain from `X-Forwarded-Host` rather than
	/// `Host`. Set it only behind a proxy that sets the header itself:
	/// otherwise a client picks the domain.
	#[serde(default)]
	pub trust_forwarded_host: bool,
}

/// A brand this posd serves. A store's Lightning address is
/// `<name>@<host of public_url>`, and its pairing links and Lightning address
/// callbacks are under `public_url`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Brand {
	/// The brand's name, as the terminal shows it: "Open <name> on your
	/// phone". Not translated.
	pub name: String,
	/// The name of an installed terminal. The default is "<name> POS".
	pub app_name: Option<String>,
	/// The name under its home screen icon. The default is `app_name`.
	pub short_name: Option<String>,
	/// The public base URL, for example `https://pay.example.com`.
	pub public_url: String,
	/// The page background in light and dark mode, for the browser chrome
	/// and the install splash screen. They must match `brand.css`.
	pub theme_color: Option<String>,
	pub theme_color_dark: Option<String>,
	/// A directory with the brand's `brand.css`, `mark.svg` or `mark.png`,
	/// `spinner.svg`, `icon-192.png`, `icon-512.png` and fonts. Each file is
	/// optional; a missing one is the neutral brand's.
	pub assets: Option<PathBuf>,
	/// The token of the brand's operator API, which reserves Lightning
	/// address names and assigns them, for example to migrated users. No
	/// operator API without it.
	#[serde(default, skip_serializing)]
	pub admin_token: Option<String>,
}

impl Brand {
	/// `public_url` followed by `path`, which starts with `/`.
	pub fn url(&self, path: &str) -> String {
		format!("{}{}", self.public_url.trim_end_matches('/'), path)
	}

	/// The host of `public_url`, with its port if it has one: the domain of
	/// the Lightning addresses.
	pub fn host(&self) -> String {
		url::Url::parse(&self.public_url).ok()
			.and_then(|u| u.host_str().map(|h| match u.port() {
				Some(p) => format!("{}:{}", h, p),
				None => h.to_string(),
			}))
			.unwrap_or_default()
	}

	pub fn app_name(&self) -> String {
		self.app_name.clone().unwrap_or_else(|| format!("{} POS", self.name))
	}

	/// The path of an asset file, if the brand has it.
	pub fn asset(&self, file: &str) -> Option<PathBuf> {
		crate::brand::asset_in(self.assets.as_deref()?, file)
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Barkd {
	/// The base URL of the private barkd that issues forwarding invoices.
	pub url: String,
	/// The barkd bearer token.
	#[serde(skip_serializing)]
	pub token: Option<String>,
}

/// Where the bitcoin price comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PriceSource {
	/// The Bull Bitcoin public price API.
	#[default]
	BullBitcoin,
	/// Any HTTP API that answers JSON: `url` and `pointer` name the price,
	/// with `{currency}` for the currency code.
	Json,
	/// `fixed_btc_price` in every currency. For test networks only.
	Fixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Price {
	#[serde(default)]
	pub source: PriceSource,
	/// The API URL. For `bullbitcoin`, the default is its public endpoint.
	/// For `json`, for example
	/// `https://api.coinbase.com/v2/prices/BTC-{currency}/spot`.
	pub url: Option<String>,
	/// For `json`: the JSON pointer to the price of one bitcoin in the
	/// answer, a string or a number, for example `/data/amount`.
	pub pointer: Option<String>,
	/// For `fixed`: the price of one bitcoin in every currency.
	pub fixed_btc_price: Option<String>,
	#[serde(default = "default_price_cache_secs")]
	pub cache_secs: u64,
	/// The currencies a store can use, with the decimal places a sale may
	/// have. The default is what the Bull Bitcoin API quotes; other sources
	/// must list theirs.
	pub currencies: Option<BTreeMap<String, u32>>,
}

fn default_price_cache_secs() -> u64 { 60 }

impl Default for Price {
	fn default() -> Self {
		Price {
			source: PriceSource::default(),
			url: None,
			pointer: None,
			fixed_btc_price: None,
			cache_secs: default_price_cache_secs(),
			currencies: None,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sales {
	/// How long an open sale and its invoices stay valid.
	#[serde(default = "default_sale_expiry_secs")]
	pub expiry_secs: u64,
	/// The on-chain minimum for a new store, in sats.
	#[serde(default = "default_onchain_min_sat")]
	pub default_onchain_min_sat: u64,
	/// The largest Lightning address payment, in sats.
	#[serde(default = "default_lnurl_max_sat")]
	pub lnurl_max_sat: u64,
	/// How often the watcher checks open sales.
	#[serde(default = "default_poll_interval_ms")]
	pub poll_interval_ms: u64,
	/// How long after a sale expires the watcher keeps checking its
	/// payments: an on-chain address for late or extra payments, and a
	/// Lightning payment barkd has not reported delivered or gone.
	#[serde(default = "default_watch_secs")]
	pub watch_secs: u64,
}

fn default_sale_expiry_secs() -> u64 { 600 }
fn default_onchain_min_sat() -> u64 { 50_000 }
fn default_lnurl_max_sat() -> u64 { 10_000_000 }
fn default_poll_interval_ms() -> u64 { 2_000 }
fn default_watch_secs() -> u64 { 7 * 24 * 3600 }

impl Default for Sales {
	fn default() -> Self {
		Sales {
			expiry_secs: default_sale_expiry_secs(),
			default_onchain_min_sat: default_onchain_min_sat(),
			lnurl_max_sat: default_lnurl_max_sat(),
			poll_interval_ms: default_poll_interval_ms(),
			watch_secs: default_watch_secs(),
		}
	}
}

impl Sales {
	pub fn expiry(&self) -> Duration { Duration::from_secs(self.expiry_secs) }
	pub fn poll_interval(&self) -> Duration { Duration::from_millis(self.poll_interval_ms) }
}

/// Bolt Card payments, where posd asks a card's service to pay a sale.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BoltCard {
	/// Let a card's service be on a private or local address, and on plain
	/// http. For test networks only: otherwise a terminal could point posd
	/// at its own network.
	#[serde(default)]
	pub allow_private_hosts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
	pub network: Network,
	pub http: Http,
	pub postgres: Postgres,
	/// The Ark server's public gRPC address.
	pub ark_server_url: String,
	pub barkd: Barkd,
	/// The chain source for on-chain payments: a bitcoind, where posd keeps
	/// a watch-only wallet, or an Esplora API. Set exactly one.
	pub bitcoind: Option<crate::rails::onchain::BitcoindConfig>,
	pub esplora_url: Option<String>,
	#[serde(default)]
	pub price: Price,
	#[serde(default)]
	pub sales: Sales,
	#[serde(default)]
	pub bolt_card: BoltCard,
	/// The brands this posd serves, by key. A store of another brand cannot
	/// register.
	pub brands: BTreeMap<BrandKey, Brand>,
}

/// Whether `url` is an http(s) URL with a host and nothing after it but a
/// `/`: posd appends paths to it.
fn plain_base_url(url: &str) -> bool {
	url::Url::parse(url).is_ok_and(|u| {
		matches!(u.scheme(), "http" | "https") && u.host_str().is_some()
			&& u.path() == "/" && u.query().is_none() && u.fragment().is_none()
			&& u.username().is_empty() && u.password().is_none()
	})
}

/// The shortest operator token posd accepts.
const MIN_ADMIN_TOKEN_LEN: usize = 32;

impl Config {
	/// Loads a config file, then applies `POSD__` environment variables, for
	/// example `POSD__POSTGRES__HOST`, and checks it.
	pub fn load(path: Option<&Path>) -> anyhow::Result<Config> {
		let mut builder = config::Config::builder();
		if let Some(path) = path {
			builder = builder.add_source(config::File::from(path));
		}
		let cfg: Config = builder.add_source(config::Environment::with_prefix("POSD").separator("__"))
			.build().context("failed to read the config")?
			.try_deserialize().context("the config is not valid")?;
		cfg.validate()?;
		Ok(cfg)
	}

	/// Checks what deserializing cannot.
	pub fn validate(&self) -> anyhow::Result<()> {
		if self.brands.is_empty() {
			bail!("the config has no [brands]");
		}
		let mut hosts = std::collections::HashSet::new();
		let mut tokens = std::collections::HashSet::new();
		for (key, brand) in &self.brands {
			if key.as_str() == DEFAULT_KEY {
				bail!("brands.{} is the name of the built-in brand", DEFAULT_KEY);
			}
			if brand.name.trim().is_empty() {
				bail!("brands.{}.name is empty", key);
			}
			let host = brand.host();
			if host.is_empty() || !plain_base_url(&brand.public_url) {
				bail!("brands.{}.public_url is not an http(s) URL of a host, with no path or query", key);
			}
			if !hosts.insert(host.clone()) {
				bail!("two brands have the public host {}", host);
			}
			if brand.admin_token.as_ref().is_some_and(|t| t.len() < MIN_ADMIN_TOKEN_LEN) {
				bail!("brands.{}.admin_token must be at least {} characters", key, MIN_ADMIN_TOKEN_LEN);
			}
			if let Some(token) = &brand.admin_token {
				if !tokens.insert(token.as_str()) {
					bail!("two brands have the same admin_token");
				}
			}
			if let Some(dir) = &brand.assets {
				if !dir.is_dir() {
					bail!("brands.{}.assets is not a directory: {}", key, dir.display());
				}
			}
		}

		if self.postgres.max_connections == 0 {
			bail!("postgres.max_connections must not be zero");
		}
		if self.ark_server_url.trim().is_empty() || self.barkd.url.trim().is_empty() {
			bail!("ark_server_url and barkd.url must be set");
		}
		match (&self.bitcoind, &self.esplora_url) {
			(Some(b), None) => {
				if b.url.trim().is_empty() {
					bail!("bitcoind.url must be set");
				}
				if b.rpc_user.is_none() && b.cookie.is_none() {
					bail!("[bitcoind] needs rpc_user or cookie");
				}
			},
			(None, Some(url)) => if url.trim().is_empty() {
				bail!("esplora_url is empty");
			},
			_ => bail!("set exactly one of [bitcoind] and esplora_url"),
		}

		let price = &self.price;
		match price.source {
			PriceSource::BullBitcoin => {},
			PriceSource::Json => {
				let url = price.url.as_deref().unwrap_or("");
				if url.is_empty() || price.pointer.as_deref().unwrap_or("").is_empty() {
					bail!("the json price source needs price.url and price.pointer");
				}
				if !price.pointer.as_deref().unwrap_or("").starts_with('/') {
					bail!("price.pointer is a JSON pointer, which starts with /");
				}
				if price.currencies.is_none() {
					bail!("the json price source needs price.currencies");
				}
			},
			PriceSource::Fixed => {
				if self.network == Network::Bitcoin {
					bail!("the fixed price source is for test networks only");
				}
				let fixed = price.fixed_btc_price.as_deref()
					.context("the fixed price source needs price.fixed_btc_price")?;
				let fixed = fixed.parse::<Decimal>().context("price.fixed_btc_price is not a decimal")?;
				if fixed.is_zero() {
					bail!("price.fixed_btc_price is zero");
				}
				if price.currencies.is_none() {
					bail!("the fixed price source needs price.currencies");
				}
			},
		}
		if price.source != PriceSource::Fixed && price.fixed_btc_price.is_some() {
			bail!("price.fixed_btc_price needs source = \"fixed\"");
		}
		if let Some(currencies) = &price.currencies {
			if currencies.is_empty() {
				bail!("price.currencies is empty");
			}
			for (code, decimals) in currencies {
				if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_uppercase()) {
					bail!("price.currencies: {} is not a currency code", code);
				}
				if *decimals > 4 {
					bail!("price.currencies: {} has more than 4 decimal places", code);
				}
			}
		}

		let sales = &self.sales;
		if sales.expiry_secs == 0 || sales.poll_interval_ms == 0 || sales.lnurl_max_sat == 0 {
			bail!("sales.expiry_secs, sales.poll_interval_ms and sales.lnurl_max_sat must not be zero");
		}
		if sales.default_onchain_min_sat > crate::money::MAX_SAT || sales.lnurl_max_sat > crate::money::MAX_SAT {
			bail!("sales.default_onchain_min_sat and sales.lnurl_max_sat must be at most 21 million bitcoin");
		}
		if self.bolt_card.allow_private_hosts && self.network == Network::Bitcoin {
			bail!("bolt_card.allow_private_hosts is for test networks only");
		}
		Ok(())
	}

	/// The brand of `key`, if this posd serves it.
	pub fn brand(&self, key: &BrandKey) -> Option<&Brand> {
		self.brands.get(key)
	}

	/// The brand whose public host is `host`. With a single brand, that
	/// brand answers for every host, which keeps a one-brand deployment
	/// reachable under any name.
	pub fn brand_for_host(&self, host: &str) -> Option<&BrandKey> {
		self.brands.iter().find(|(_, b)| b.host().eq_ignore_ascii_case(host)).map(|(k, _)| k)
			.or_else(|| (self.brands.len() == 1).then(|| self.brands.keys().next().unwrap()))
	}
}

#[cfg(test)]
mod test {
	use super::*;

	fn key(k: &str) -> BrandKey {
		k.parse().unwrap()
	}

	fn brands(urls: &[(&str, &str)]) -> BTreeMap<BrandKey, Brand> {
		urls.iter().map(|(k, u)| (key(k), Brand {
			name: k.to_string(),
			app_name: None,
			short_name: None,
			public_url: u.to_string(),
			theme_color: None,
			theme_color_dark: None,
			assets: None,
			admin_token: None,
		})).collect()
	}

	fn config(brands: BTreeMap<BrandKey, Brand>) -> Config {
		Config {
			network: Network::Regtest,
			http: Http { listen: "127.0.0.1:0".parse().unwrap(), trust_forwarded_host: false },
			postgres: Postgres {
				host: String::new(), port: 5432, name: String::new(), user: None, password: None,
				max_connections: 1,
			},
			ark_server_url: "http://127.0.0.1:3535".into(),
			barkd: Barkd { url: "http://127.0.0.1:3000".into(), token: None },
			bitcoind: None,
			esplora_url: Some("http://127.0.0.1:3002".into()),
			price: Price::default(),
			sales: Sales::default(),
			bolt_card: BoltCard::default(),
			brands,
		}
	}

	#[test]
	fn picks_the_brand_by_host() {
		let cfg = config(brands(&[
			("alpha", "https://alpha.example.com"),
			("beta", "https://beta.example.com/"),
		]));
		cfg.validate().unwrap();
		assert_eq!(cfg.brand_for_host("alpha.example.com"), Some(&key("alpha")));
		assert_eq!(cfg.brand_for_host("BETA.EXAMPLE.COM"), Some(&key("beta")));
		assert_eq!(cfg.brand_for_host("other.example"), None);
		let beta = cfg.brand(&key("beta")).unwrap();
		assert_eq!(beta.url("/pair/abc"), "https://beta.example.com/pair/abc");
		assert_eq!(beta.host(), "beta.example.com");
		assert_eq!(beta.app_name(), "beta POS");
	}

	#[test]
	fn one_brand_answers_any_host() {
		let cfg = config(brands(&[("alpha", "http://127.0.0.1:3080")]));
		assert_eq!(cfg.brand(&key("alpha")).unwrap().host(), "127.0.0.1:3080");
		assert_eq!(cfg.brand_for_host("192.168.1.4:3080"), Some(&key("alpha")));
	}

	#[test]
	fn refuses_brands_without_their_own_host() {
		assert!(config(BTreeMap::new()).validate().is_err());
		assert!(config(brands(&[("alpha", "not a url")])).validate().is_err());
		assert!(config(brands(&[("default", "https://pay.example.com")])).validate().is_err());
		assert!(config(brands(&[
			("alpha", "https://pay.example.com"),
			("beta", "https://pay.example.com/beta"),
		])).validate().is_err());
		assert!(config(brands(&[("a", "https://pay.example.com/shop")])).validate().is_err(), "a path");
		assert!(config(brands(&[("a", "https://pay.example.com?x=1")])).validate().is_err(), "a query");
		assert!(config(brands(&[("a", "ftp://pay.example.com")])).validate().is_err(), "not http");
		let mut same = brands(&[("a", "https://a.example.com"), ("b", "https://b.example.com")]);
		for b in same.values_mut() {
			b.admin_token = Some("x".repeat(MIN_ADMIN_TOKEN_LEN));
		}
		assert!(config(same).validate().is_err(), "a shared admin token");
		let mut short = brands(&[("alpha", "https://pay.example.com")]);
		short.get_mut(&key("alpha")).unwrap().admin_token = Some("short".into());
		assert!(config(short).validate().is_err());
	}

	#[test]
	fn checks_chain_and_price() {
		let ok = || config(brands(&[("shop", "https://pay.example.com")]));
		ok().validate().unwrap();

		let mut none = ok();
		none.esplora_url = None;
		assert!(none.validate().is_err(), "no chain source");

		let mut fixed = ok();
		fixed.price.source = PriceSource::Fixed;
		fixed.price.fixed_btc_price = Some("100000".into());
		assert!(fixed.validate().is_err(), "fixed needs currencies");
		fixed.price.currencies = Some([("USD".to_string(), 2)].into());
		fixed.validate().unwrap();
		fixed.network = Network::Bitcoin;
		assert!(fixed.validate().is_err(), "no fixed price on mainnet");

		let mut stray = ok();
		stray.price.fixed_btc_price = Some("100000".into());
		assert!(stray.validate().is_err(), "a fixed price needs the fixed source");

		let mut json = ok();
		json.price.source = PriceSource::Json;
		json.price.url = Some("https://example.com/{currency}".into());
		json.price.currencies = Some([("usd".to_string(), 2)].into());
		assert!(json.validate().is_err(), "no pointer");
		json.price.pointer = Some("price".into());
		json.price.currencies = Some([("USD".to_string(), 2)].into());
		assert!(json.validate().is_err(), "a pointer without /");
		json.price.currencies = Some([("usd".to_string(), 2)].into());
		json.price.pointer = Some("/price".into());
		assert!(json.validate().is_err(), "lowercase code");
		json.price.currencies = Some([("USD".to_string(), 2)].into());
		json.validate().unwrap();

		let mut zero = ok();
		zero.sales.expiry_secs = 0;
		assert!(zero.validate().is_err());
		let mut big = ok();
		big.sales.lnurl_max_sat = u64::MAX / 100;
		assert!(big.validate().is_err(), "lnurl_max_sat over 21 million bitcoin");
		let mut pool = ok();
		pool.postgres.max_connections = 0;
		assert!(pool.validate().is_err());
	}
}
