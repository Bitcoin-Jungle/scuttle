//! The bitcoin price, from the source the config names.
//!
//! - `bullbitcoin`: the Bull Bitcoin public price API.
//! - `json`: any HTTP API that answers JSON, with the price at a JSON pointer.
//! - `fixed`: one price in every currency, for test networks.
//!
//! The source also decides the currencies a store can use: the ones it
//! quotes, with the decimal places a sale may have.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::config::{self, PriceSource};
use crate::money::Decimal;


/// What the Bull Bitcoin API quotes (its `listRates` pairs from BTC), with
/// the decimal places a sale may have.
const BULL_BITCOIN_CURRENCIES: &[(&str, u32)] = &[
	("USD", 2),
	("CAD", 2),
	("MXN", 2),
	("CRC", 2),
	("EUR", 2),
	("ARS", 2),
	("AUD", 2),
	("COP", 0),
];

const BULL_BITCOIN_URL: &str = "https://api.bullbitcoin.com/public/price";

#[derive(Debug, Clone)]
pub struct Quote {
	pub currency: String,
	/// The price of one bitcoin in `currency`.
	pub btc_price: Decimal,
	pub fetched_at: DateTime<Utc>,
}

pub struct PriceFeed {
	cfg: config::Price,
	currencies: BTreeMap<String, u32>,
	http: reqwest::Client,
	cache: Mutex<HashMap<String, (Instant, Quote)>>,
}

impl PriceFeed {
	pub fn new(cfg: config::Price) -> anyhow::Result<PriceFeed> {
		let http = reqwest::Client::builder()
			.timeout(Duration::from_secs(10))
			.build()?;
		let currencies = cfg.currencies.clone().unwrap_or_else(|| {
			BULL_BITCOIN_CURRENCIES.iter().map(|(c, d)| (c.to_string(), *d)).collect()
		});
		Ok(PriceFeed { cfg, currencies, http, cache: Mutex::new(HashMap::new()) })
	}

	/// The decimal places of a currency the source quotes, or `None` if it
	/// does not quote it.
	pub fn decimals(&self, currency: &str) -> Option<u32> {
		self.currencies.get(currency).copied()
	}

	/// The currencies a store can use.
	pub fn currencies(&self) -> impl Iterator<Item = &str> {
		self.currencies.keys().map(String::as_str)
	}

	/// The current price of one bitcoin in `currency`, at most `cache_secs`
	/// old.
	pub async fn quote(&self, currency: &str) -> anyhow::Result<Quote> {
		if self.decimals(currency).is_none() {
			bail!("the price source does not quote {}", currency);
		}
		if self.cfg.source == PriceSource::Fixed {
			let fixed = self.cfg.fixed_btc_price.as_deref().context("no fixed_btc_price")?;
			return Ok(Quote {
				currency: currency.to_string(),
				btc_price: fixed.parse().context("fixed_btc_price is not a decimal")?,
				fetched_at: Utc::now(),
			});
		}

		let max_age = Duration::from_secs(self.cfg.cache_secs);
		let mut cache = self.cache.lock().await;
		if let Some((at, quote)) = cache.get(currency) {
			if at.elapsed() < max_age {
				return Ok(quote.clone());
			}
		}

		let btc_price = match self.cfg.source {
			PriceSource::BullBitcoin => self.bull_bitcoin(currency).await?,
			PriceSource::Json => self.json(currency).await?,
			PriceSource::Fixed => unreachable!("answered above"),
		};
		let quote = Quote { currency: currency.to_string(), btc_price, fetched_at: Utc::now() };
		cache.insert(currency.to_string(), (Instant::now(), quote.clone()));
		Ok(quote)
	}

	async fn bull_bitcoin(&self, currency: &str) -> anyhow::Result<Decimal> {
		let body = json!({
			"id": 1,
			"jsonrpc": "2.0",
			"method": "getRate",
			"params": { "element": { "fromCurrency": "BTC", "toCurrency": currency } },
		});
		let url = self.cfg.url.as_deref().unwrap_or(BULL_BITCOIN_URL);
		let resp = self.http.post(url).json(&body).send().await
			.context("price API request failed")?
			.error_for_status().context("price API returned an error status")?
			.json::<Value>().await
			.context("price API returned no JSON")?;
		parse_get_rate(&resp)
	}

	async fn json(&self, currency: &str) -> anyhow::Result<Decimal> {
		let url = self.cfg.url.as_deref().context("no price.url")?.replace("{currency}", currency);
		let pointer = self.cfg.pointer.as_deref().context("no price.pointer")?
			.replace("{currency}", currency);
		let resp = self.http.get(url).send().await
			.context("price API request failed")?
			.error_for_status().context("price API returned an error status")?
			.json::<Value>().await
			.context("price API returned no JSON")?;
		parse_pointer(&resp, &pointer)
	}
}

/// Reads the price from a Bull Bitcoin `getRate` answer:
/// `indexPrice / 10^precision`.
fn parse_get_rate(resp: &Value) -> anyhow::Result<Decimal> {
	if let Some(err) = resp.get("error") {
		bail!("price API error: {}", err);
	}
	let element = resp.pointer("/result/element").context("price API answer has no element")?;
	let index = element.get("indexPrice").and_then(|v| v.as_u64())
		.context("price API answer has no integer indexPrice")?;
	let precision = element.get("precision").and_then(|v| v.as_u64()).unwrap_or(2);
	if index == 0 || precision > 18 {
		bail!("price API answer is not usable: {}", element);
	}
	Ok(Decimal::new(index as u128, precision as u32))
}

/// Reads the price at `pointer`: a decimal string, or a JSON number written
/// as a plain decimal.
fn parse_pointer(resp: &Value, pointer: &str) -> anyhow::Result<Decimal> {
	let value = resp.pointer(pointer)
		.with_context(|| format!("the price API answer has nothing at {}", pointer))?;
	let text = match value {
		Value::String(s) => s.clone(),
		Value::Number(n) => n.to_string(),
		other => bail!("the price at {} is not a number: {}", pointer, other),
	};
	let price = text.parse::<Decimal>()
		.with_context(|| format!("the price at {} is not a plain decimal: {}", pointer, text))?;
	if price.is_zero() {
		bail!("the price API answered zero");
	}
	Ok(price)
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn parses_get_rate() {
		let resp = json!({"result": {"element": {"indexPrice": 6250000000u64, "precision": 2}}});
		assert_eq!(parse_get_rate(&resp).unwrap().to_string(), "62500000.00");
		let no_precision = json!({"result": {"element": {"indexPrice": 100000}}});
		assert_eq!(parse_get_rate(&no_precision).unwrap().to_string(), "1000.00");
		assert!(parse_get_rate(&json!({"error": {"code": 1}})).is_err());
		assert!(parse_get_rate(&json!({"result": {"element": {"indexPrice": 0}}})).is_err());
		assert!(parse_get_rate(&json!({"result": {}})).is_err());
	}

	#[test]
	fn parses_pointers() {
		// Coinbase's spot price.
		let coinbase = json!({"data": {"amount": "98765.43", "base": "BTC", "currency": "USD"}});
		assert_eq!(parse_pointer(&coinbase, "/data/amount").unwrap().to_string(), "98765.43");
		// mempool.space's prices.
		let mempool = json!({"time": 1, "USD": 98765, "EUR": 91234});
		assert_eq!(parse_pointer(&mempool, "/EUR").unwrap().to_string(), "91234");
		assert!(parse_pointer(&mempool, "/CRC").is_err());
		assert!(parse_pointer(&json!({"p": 0}), "/p").is_err());
		assert!(parse_pointer(&json!({"p": 1e21}), "/p").is_err());
		assert!(parse_pointer(&json!({"p": true}), "/p").is_err());
	}

	#[tokio::test]
	async fn fixed_quotes_only_its_currencies() {
		let feed = PriceFeed::new(config::Price {
			source: PriceSource::Fixed,
			fixed_btc_price: Some("100000.00".into()),
			currencies: Some([("USD".to_string(), 2), ("JPY".to_string(), 0)].into()),
			..config::Price::default()
		}).unwrap();
		assert_eq!(feed.quote("USD").await.unwrap().btc_price.to_string(), "100000.00");
		assert_eq!(feed.decimals("JPY"), Some(0));
		assert!(feed.quote("CRC").await.is_err());
		assert_eq!(feed.currencies().collect::<Vec<_>>(), ["JPY", "USD"]);
	}
}
