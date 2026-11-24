//! posd, a point-of-sale and Lightning address service for Ark wallets.
//!
//! posd pairs terminals to a store, creates sales and watches two payment
//! rails. Both pay the store owner's wallet directly: posd never holds the
//! bitcoin and terminals cannot send.
//!
//! - Lightning: a forwarding invoice from a private barkd. A wallet on the
//!   same Ark server pays it too: the server settles that payment without
//!   Lightning, at its intra-Ark fees.
//! - On-chain: an address from the owner's public descriptor, made when the
//!   cashier picks on-chain.

pub mod api;
pub mod ark_server;
pub mod brand;
pub mod config;
pub mod database;
pub mod error;
pub mod login;
pub mod money;
pub mod price;
pub mod rails;
pub mod registration;
pub mod names;
pub mod sales;
pub mod silent_payment;
pub mod token;
pub mod watcher;

use std::sync::Arc;

use anyhow::Context;
use tokio::sync::watch;

use crate::ark_server::ArkServer;
use crate::config::Config;
use crate::database::Db;
use crate::login::LoginKey;
use crate::price::PriceFeed;
use crate::rails::forward::Forwarder;
use crate::rails::onchain::Chain;


pub struct App {
	pub cfg: Config,
	pub db: Db,
	pub price: PriceFeed,
	pub forwarder: Forwarder,
	pub ark: ArkServer,
	pub chain: Chain,
	/// Authenticates owner login challenges.
	pub login_key: LoginKey,
	/// Bumped whenever a sale changes state, to wake long-polls.
	pub sale_changes: watch::Sender<u64>,
	/// Lightning address invoices being made, which count toward the caps.
	pub lnurl_pending: api::lnurl::Pending,
	/// What the terminal knows of each brand, by key, the neutral one too.
	/// Worked out once, as it looks at the brands' files.
	pub brands: std::collections::BTreeMap<String, brand::BrandInfo>,
}

impl App {
	pub async fn new(cfg: Config) -> anyhow::Result<Arc<App>> {
		cfg.validate()?;
		let db = Db::connect(&cfg.postgres).await?;
		let ark = ArkServer::connect(&cfg).await?;
		let info = ark.ark_info().await;
		if info.network != cfg.network {
			anyhow::bail!("the Ark server runs on {}, not {}", info.network, cfg.network);
		}
		let login_key = db.login_key().await.context("failed to load the login key")?;
		let app = App {
			login_key,
			price: PriceFeed::new(cfg.price.clone())?,
			forwarder: Forwarder::new(&cfg.barkd)?,
			chain: Chain::connect(&cfg).await.context("failed to set up the chain source")?,
			sale_changes: watch::channel(0).0,
			lnurl_pending: Default::default(),
			brands: brand::BrandInfo::all(&cfg),
			db,
			ark,
			cfg,
		};
		Ok(Arc::new(app))
	}

	pub fn notify_sale_change(&self) {
		self.sale_changes.send_modify(|v| *v = v.wrapping_add(1));
	}
}
