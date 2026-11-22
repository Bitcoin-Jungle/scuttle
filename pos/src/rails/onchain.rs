//! The on-chain rail: one address per sale, derived from the owner's
//! public descriptor.
//!
//! posd reads the chain from one of two sources:
//! - bitcoind: a watch-only wallet of posd's own. Each sale address is
//!   imported when the sale is created. bitcoind rescans only the last two
//!   hours for it, so older payments never count.
//! - Esplora: address lookups over HTTP, with nothing to import.
//!
//! A sale skips an address that already received what posd can see, so an
//! earlier payment to the wallet cannot pay a new sale.

use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use anyhow::{Context, bail};
use bdk_wallet::miniscript::ForEachKey;
use bdk_wallet::miniscript::descriptor::{Descriptor, DescriptorPublicKey, Wildcard};
use bitcoin::{Address, Amount, Network, NetworkKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};


/// A public, single-path, ranged descriptor such as
/// `tr([fp/86'/0'/1']xpub…/0/*)`.
#[derive(Debug, Clone)]
pub struct StoreDescriptor(Descriptor<DescriptorPublicKey>);

impl StoreDescriptor {
	pub fn parse(s: &str) -> anyhow::Result<StoreDescriptor> {
		let desc = Descriptor::<DescriptorPublicKey>::from_str(s)
			.context("not a valid public descriptor")?;
		if !desc.has_wildcard() {
			bail!("the descriptor must be ranged (end in /*)");
		}
		if desc.is_multipath() {
			bail!("the descriptor must have a single derivation path");
		}
		// A hardened wildcard needs a private key to derive.
		let hardened = desc.for_any_key(|k| match k {
			DescriptorPublicKey::XPub(x) => x.wildcard == Wildcard::Hardened,
			DescriptorPublicKey::MultiXPub(x) => x.wildcard == Wildcard::Hardened,
			DescriptorPublicKey::Single(_) => false,
		});
		if hardened {
			bail!("the descriptor must derive without private keys (no hardened wildcard)");
		}
		Ok(StoreDescriptor(desc))
	}

	/// Parses a descriptor for a store on `network`: its keys must be for
	/// it, and it must have an address form there.
	pub fn parse_for(s: &str, network: Network) -> anyhow::Result<StoreDescriptor> {
		let desc = StoreDescriptor::parse(s)?;
		let other = desc.0.for_any_key(|k| match k {
			DescriptorPublicKey::XPub(x) => x.xkey.network != NetworkKind::from(network),
			DescriptorPublicKey::MultiXPub(x) => x.xkey.network != NetworkKind::from(network),
			DescriptorPublicKey::Single(_) => false,
		});
		if other {
			bail!("the descriptor's keys are not for {}", network);
		}
		desc.address(0, network)?;
		Ok(desc)
	}

	pub fn address(&self, index: u32, network: Network) -> anyhow::Result<Address> {
		let derived = self.0.at_derivation_index(index)
			.context("index out of range for the descriptor")?;
		derived.address(network).context("the descriptor has no address form")
	}
}

/// What the chain shows for an address.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AddressFunds {
	/// Paid to the address with at least 1 confirmation.
	pub confirmed: Amount,
	/// Paid to the address in mempool transactions.
	pub pending: Amount,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitcoindConfig {
	/// The RPC URL, for example `http://127.0.0.1:8332`.
	pub url: String,
	/// The RPC cookie file. Used when `rpc_user` is not set.
	pub cookie: Option<PathBuf>,
	pub rpc_user: Option<String>,
	#[serde(skip_serializing)]
	pub rpc_password: Option<String>,
	/// The watch-only wallet posd creates and uses.
	#[serde(default = "default_wallet")]
	pub wallet: String,
}

fn default_wallet() -> String { "posd".into() }

/// The chain source for the on-chain rail.
pub enum Chain {
	Bitcoind(Bitcoind),
	Esplora(Esplora),
}

impl Chain {
	/// Connects to the source the config names: exactly one of `bitcoind`
	/// and `esplora_url`.
	pub async fn connect(cfg: &crate::config::Config) -> anyhow::Result<Chain> {
		match (&cfg.bitcoind, &cfg.esplora_url) {
			(Some(b), None) => Ok(Chain::Bitcoind(Bitcoind::connect(b.clone()).await?)),
			(None, Some(url)) => Ok(Chain::Esplora(Esplora::new(url)?)),
			_ => bail!("set exactly one of [bitcoind] and esplora_url"),
		}
	}

	/// Starts watching an address for a new sale. Returns whether it is
	/// fresh: no payment to it that the watcher would count.
	pub async fn watch(&self, address: &Address) -> anyhow::Result<bool> {
		match self {
			Chain::Bitcoind(b) => {
				b.watch(address).await?;
				Ok(b.funds(address).await? == AddressFunds::default())
			},
			// Esplora indexes every address.
			Chain::Esplora(e) => Ok(!e.has_history(address).await?),
		}
	}

	/// The payments to an address. Spends from it do not count: the owner
	/// spending a payment does not undo the sale.
	pub async fn funds(&self, address: &Address) -> anyhow::Result<AddressFunds> {
		match self {
			Chain::Bitcoind(b) => b.funds(address).await,
			Chain::Esplora(e) => e.funds(address).await,
		}
	}

	/// The transactions that paid the address, for the record.
	pub async fn txids(&self, address: &Address) -> anyhow::Result<Vec<String>> {
		match self {
			Chain::Bitcoind(b) => b.txids(address).await,
			Chain::Esplora(e) => e.txids(address).await,
		}
	}
}

pub struct Esplora {
	client: esplora_client::AsyncClient,
}

impl Esplora {
	pub fn new(url: &str) -> anyhow::Result<Esplora> {
		let client = esplora_client::Builder::new(url.trim_end_matches('/'))
			.timeout(30)
			.build_async()
			.context("failed to create the Esplora client")?;
		Ok(Esplora { client })
	}

	async fn has_history(&self, address: &Address) -> anyhow::Result<bool> {
		let stats = self.client.get_address_stats(address).await
			.context("Esplora address lookup failed")?;
		Ok(stats.chain_stats.tx_count > 0 || stats.mempool_stats.tx_count > 0)
	}

	/// The totals Esplora keeps per address, so no payment falls off a page
	/// of history.
	async fn funds(&self, address: &Address) -> anyhow::Result<AddressFunds> {
		let stats = self.client.get_address_stats(address).await
			.context("Esplora address lookup failed")?;
		Ok(AddressFunds {
			confirmed: Amount::from_sat(stats.chain_stats.funded_txo_sum),
			pending: Amount::from_sat(stats.mempool_stats.funded_txo_sum),
		})
	}

	/// The paying transactions in the first page of history: up to 50 in
	/// the mempool and 25 confirmed. The totals above never depend on it.
	async fn txids(&self, address: &Address) -> anyhow::Result<Vec<String>> {
		let script = address.script_pubkey();
		let txs = self.client.scripthash_txs(&script, None).await
			.context("Esplora address lookup failed")?;
		Ok(txs.into_iter()
			.filter(|tx| tx.vout.iter().any(|o| o.scriptpubkey == script))
			.map(|tx| tx.txid.to_string())
			.collect())
	}
}

pub struct Bitcoind {
	cfg: BitcoindConfig,
	http: reqwest::Client,
}

impl Bitcoind {
	/// Connects, and creates or loads the watch-only wallet.
	pub async fn connect(cfg: BitcoindConfig) -> anyhow::Result<Bitcoind> {
		let http = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;
		let chain = Bitcoind { cfg, http };
		chain.open_wallet().await?;
		Ok(chain)
	}

	async fn open_wallet(&self) -> anyhow::Result<()> {
		let name = self.cfg.wallet.clone();
		let loaded = self.call(None, "listwallets", json!([])).await?;
		if loaded.as_array().is_some_and(|w| w.iter().any(|n| n.as_str() == Some(&name))) {
			return Ok(());
		}
		if self.call(None, "loadwallet", json!([name])).await.is_ok() {
			return Ok(());
		}
		// A blank descriptor wallet without private keys.
		self.call(None, "createwallet", json!([name, true, true, "", false, true, true]))
			.await.context("failed to create the watch-only wallet")?;
		Ok(())
	}

	/// Starts watching an address, from two hours ago: bitcoind rescans
	/// the blocks of its `timestamp` window.
	async fn watch(&self, address: &Address) -> anyhow::Result<()> {
		let desc = format!("addr({})", address);
		let info = self.call(None, "getdescriptorinfo", json!([desc])).await?;
		let desc = info.get("descriptor").and_then(|d| d.as_str())
			.context("getdescriptorinfo returned no descriptor")?;
		let res = self.call(Some(&self.cfg.wallet), "importdescriptors", json!([[{
			"desc": desc,
			"timestamp": "now",
			"label": "",
		}]])).await?;
		let ok = res.get(0).and_then(|r| r.get("success")).and_then(|s| s.as_bool());
		if ok != Some(true) {
			bail!("bitcoind refused to watch {}: {}", address, res);
		}
		Ok(())
	}

	async fn funds(&self, address: &Address) -> anyhow::Result<AddressFunds> {
		let wallet = Some(self.cfg.wallet.as_str());
		let addr = address.to_string();
		let all = btc_value(&self.call(wallet, "getreceivedbyaddress", json!([addr, 0])).await?)?;
		// Most watched addresses never get paid: one call answers for them.
		if all == Amount::ZERO {
			return Ok(AddressFunds::default());
		}
		let confirmed = btc_value(&self.call(wallet, "getreceivedbyaddress", json!([addr, 1])).await?)?;
		Ok(AddressFunds {
			confirmed,
			pending: all.checked_sub(confirmed).unwrap_or(Amount::ZERO),
		})
	}

	async fn txids(&self, address: &Address) -> anyhow::Result<Vec<String>> {
		let wallet = Some(self.cfg.wallet.as_str());
		let list = self.call(wallet, "listreceivedbyaddress", json!([0, false, true, address.to_string()])).await?;
		Ok(list.get(0).and_then(|e| e.get("txids")).and_then(|t| t.as_array())
			.map(|t| t.iter().filter_map(|x| x.as_str().map(String::from)).collect())
			.unwrap_or_default())
	}

	async fn call(&self, wallet: Option<&str>, method: &str, params: Value) -> anyhow::Result<Value> {
		let url = match wallet {
			Some(w) => format!("{}/wallet/{}", self.cfg.url.trim_end_matches('/'), w),
			None => self.cfg.url.clone(),
		};
		let (user, pass) = self.credentials().await?;
		let body = json!({ "jsonrpc": "1.0", "id": "posd", "method": method, "params": params });
		let resp = self.http.post(url).basic_auth(user, Some(pass)).json(&body).send().await
			.with_context(|| format!("bitcoind {} request failed", method))?;
		let status = resp.status();
		let resp: Value = resp.json().await
			.with_context(|| format!("bitcoind {} returned {} without JSON", method, status))?;
		if let Some(err) = resp.get("error").filter(|e| !e.is_null()) {
			bail!("bitcoind {} failed: {}", method, err);
		}
		Ok(resp.get("result").cloned().unwrap_or(Value::Null))
	}

	async fn credentials(&self) -> anyhow::Result<(String, String)> {
		if let Some(user) = &self.cfg.rpc_user {
			return Ok((user.clone(), self.cfg.rpc_password.clone().unwrap_or_default()));
		}
		let path = self.cfg.cookie.as_ref().context("bitcoind needs rpc_user or a cookie file")?;
		// Read on every call: bitcoind writes a new cookie when it restarts.
		let cookie = tokio::fs::read_to_string(path).await
			.with_context(|| format!("failed to read the bitcoind cookie {}", path.display()))?;
		let (user, pass) = cookie.trim().split_once(':').context("the bitcoind cookie is malformed")?;
		Ok((user.to_string(), pass.to_string()))
	}
}

/// An amount in bitcoin as bitcoind answers it: a JSON number with 8
/// decimals. JSON numbers arrive as `f64`, whose text form is in scientific
/// notation below 0.00001, so the sats are rounded from the number itself.
/// Every amount up to 21 million bitcoin is exact in an `f64` of sats.
fn btc_value(v: &Value) -> anyhow::Result<Amount> {
	let btc = v.as_f64().with_context(|| format!("bitcoind returned an invalid amount: {}", v))?;
	let sat = (btc * 100_000_000.0).round();
	if !(0.0..=crate::money::MAX_SAT as f64).contains(&sat) {
		anyhow::bail!("bitcoind returned an invalid amount: {}", v);
	}
	Ok(Amount::from_sat(sat as u64))
}

#[cfg(test)]
mod test {
	use super::*;

	use bitcoin::bip32::{Xpriv, Xpub};
	use bitcoin::secp256k1::Secp256k1;

	#[test]
	fn parses_descriptors() {
		let xpriv = Xpriv::new_master(Network::Regtest, &[7u8; 32]).unwrap();
		let xpub = Xpub::from_priv(&Secp256k1::new(), &xpriv).to_string();
		let desc = StoreDescriptor::parse(&format!("tr({}/0/*)", xpub)).unwrap();
		let a0 = desc.address(0, Network::Regtest).unwrap();
		let a1 = desc.address(1, Network::Regtest).unwrap();
		assert_ne!(a0, a1);
		assert_eq!(a0, desc.address(0, Network::Regtest).unwrap());

		assert!(StoreDescriptor::parse(&format!("tr({}/0/0)", xpub)).is_err(), "not ranged");
		assert!(StoreDescriptor::parse(&format!("tr({}/<0;1>/*)", xpub)).is_err(), "multipath");
		assert!(StoreDescriptor::parse(&format!("tr({}/0/*')", xpub)).is_err(), "hardened");
		assert!(StoreDescriptor::parse("tr(nonsense)").is_err());

		// For a network: keys of that network, and an address form.
		assert!(StoreDescriptor::parse_for(&format!("tr({}/0/*)", xpub), Network::Regtest).is_ok());
		assert!(StoreDescriptor::parse_for(&format!("tr({}/0/*)", xpub), Network::Bitcoin).is_err(), "a tpub on mainnet");
		assert!(StoreDescriptor::parse_for(&format!("pk({}/0/*)", xpub), Network::Regtest).is_err(), "no address form");
	}

	#[test]
	fn reads_btc_values() {
		assert_eq!(btc_value(&json!(0.00035)).unwrap(), Amount::from_sat(35_000));
		assert_eq!(btc_value(&json!(0)).unwrap(), Amount::ZERO);
		assert_eq!(btc_value(&json!(21.12345678)).unwrap(), Amount::from_sat(2_112_345_678));
		// Below 0.00001 a JSON number prints in scientific notation.
		assert_eq!(btc_value(&json!(0.00000001)).unwrap(), Amount::from_sat(1));
		assert_eq!(btc_value(&json!(0.00000546)).unwrap(), Amount::from_sat(546));
		assert_eq!(btc_value(&json!(0.00000999)).unwrap(), Amount::from_sat(999));
		assert_eq!(btc_value(&json!(20999999.99999999)).unwrap(), Amount::from_sat(2_099_999_999_999_999));
		assert!(btc_value(&json!(-1)).is_err());
		assert!(btc_value(&json!("0.1")).is_err());
	}
}

/// Checks the Esplora source against a live server. Run with
/// `POSD_ESPLORA=<url> POSD_ADDRESS=<address> POSD_CONFIRMED_SAT=<n>
/// cargo test -p bark-pos esplora_live -- --ignored`.
#[cfg(test)]
#[tokio::test]
#[ignore]
async fn esplora_live() {
	let url = std::env::var("POSD_ESPLORA").unwrap();
	let address = Address::from_str(&std::env::var("POSD_ADDRESS").unwrap()).unwrap().assume_checked();
	let expected = std::env::var("POSD_CONFIRMED_SAT").unwrap().parse::<u64>().unwrap();
	let funds = Esplora::new(&url).unwrap().funds(&address).await.unwrap();
	assert_eq!(funds.confirmed, Amount::from_sat(expected), "{:?}", funds);
}
