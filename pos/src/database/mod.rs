//! The posd database.

mod migrations {
	include!(concat!(env!("OUT_DIR"), "/migrations.rs"));
}

use std::fmt;
use std::str::FromStr;

use anyhow::Context;
use bb8::{Pool, PooledConnection};
use bb8_postgres::PostgresConnectionManager;
use chrono::{DateTime, Utc};
use postgres_types::{FromSql, ToSql};
use serde::{Deserialize, Serialize};
use tokio_postgres::{NoTls, Row};

use crate::brand::BrandKey;
use crate::config;


pub type PgPool = Pool<PostgresConnectionManager<NoTls>>;

#[derive(Clone)]
pub struct Db {
	pool: PgPool,
}

async fn pool(cfg: &config::Postgres) -> anyhow::Result<PgPool> {
	let manager = PostgresConnectionManager::new(cfg.to_pg_config(), NoTls);
	Pool::builder().max_size(cfg.max_connections).build(manager).await
		.with_context(|| format!("failed to connect to Postgres database {}", cfg.name))
}

impl Db {
	/// Connects and runs the migrations.
	pub async fn connect(cfg: &config::Postgres) -> anyhow::Result<Db> {
		let pool = pool(cfg).await?;
		let mut conn = pool.dedicated_connection().await?;
		migrations::runner().run_async(&mut conn).await.context("database migration failed")?;
		Ok(Db { pool })
	}

	/// The key that authenticates login challenges, made on first use.
	pub async fn login_key(&self) -> anyhow::Result<crate::login::LoginKey> {
		let conn = self.conn().await?;
		conn.execute(
			"INSERT INTO server_key (name, key) VALUES ('owner-login', $1) ON CONFLICT DO NOTHING",
			&[&crate::login::LoginKey::random().as_bytes()],
		).await?;
		let key: Vec<u8> = conn.query_one("SELECT key FROM server_key WHERE name = 'owner-login'", &[])
			.await?.get("key");
		crate::login::LoginKey::from_slice(&key).context("the stored login key is not 32 bytes")
	}

	pub async fn conn(&self) -> anyhow::Result<PooledConnection<'_, PostgresConnectionManager<NoTls>>> {
		Ok(self.pool.get().await?)
	}
}

/// The state of a sale. `paid`, `expired` and `canceled` are final; an
/// `in-progress` sale has an on-chain payment in the mempool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSql, FromSql, utoipa::ToSchema)]
#[postgres(name = "sale_state")]
#[serde(rename_all = "kebab-case")]
pub enum SaleState {
	#[postgres(name = "open")]
	Open,
	#[postgres(name = "in-progress")]
	InProgress,
	#[postgres(name = "paid")]
	Paid,
	#[postgres(name = "expired")]
	Expired,
	#[postgres(name = "canceled")]
	Canceled,
}

impl SaleState {
	pub fn as_str(&self) -> &'static str {
		match self {
			SaleState::Open => "open",
			SaleState::InProgress => "in-progress",
			SaleState::Paid => "paid",
			SaleState::Expired => "expired",
			SaleState::Canceled => "canceled",
		}
	}

	/// Whether the state can still change through a payment.
	pub fn is_live(&self) -> bool {
		matches!(self, SaleState::Open | SaleState::InProgress)
	}
}

impl fmt::Display for SaleState {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.write_str(self.as_str())
	}
}

impl FromStr for SaleState {
	type Err = anyhow::Error;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Ok(match s {
			"open" => SaleState::Open,
			"in-progress" => SaleState::InProgress,
			"paid" => SaleState::Paid,
			"expired" => SaleState::Expired,
			"canceled" => SaleState::Canceled,
			_ => anyhow::bail!("unknown sale state: {}", s),
		})
	}
}

/// The rail that paid a sale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSql, FromSql, utoipa::ToSchema)]
#[postgres(name = "payment_rail")]
#[serde(rename_all = "kebab-case")]
pub enum Rail {
	#[postgres(name = "lightning")]
	Lightning,
	#[postgres(name = "onchain")]
	Onchain,
}

impl Rail {
	pub fn as_str(&self) -> &'static str {
		match self {
			Rail::Lightning => "lightning",
			Rail::Onchain => "onchain",
		}
	}
}

#[derive(Debug, Clone)]
pub struct Store {
	pub id: i64,
	pub name: String,
	pub ark_address: String,
	pub onchain_descriptor: String,
	pub next_onchain_index: i64,
	pub silent_payment_address: Option<String>,
	pub currency: String,
	pub brand: BrandKey,
	pub onchain_min_sat: i64,
	pub lightning_address_name: Option<String>,
}

pub const STORE_COLUMNS: &str = "s.id, s.name, s.ark_address, s.onchain_descriptor, \
	s.next_onchain_index, s.silent_payment_address, s.currency, s.brand, s.onchain_min_sat, \
	s.lightning_address_name";

impl Store {
	pub fn from_row(row: &Row) -> Store {
		Store {
			id: row.get("id"),
			name: row.get("name"),
			ark_address: row.get("ark_address"),
			onchain_descriptor: row.get("onchain_descriptor"),
			next_onchain_index: row.get("next_onchain_index"),
			silent_payment_address: row.get("silent_payment_address"),
			currency: row.get("currency"),
			// Only posd writes the column, with a valid key.
			brand: BrandKey::from_db(row.get("brand")),
			onchain_min_sat: row.get("onchain_min_sat"),
			lightning_address_name: row.get("lightning_address_name"),
		}
	}
}

#[derive(Debug, Clone)]
pub struct Device {
	pub id: i64,
	pub name: String,
	pub created_at: DateTime<Utc>,
	pub revoked_at: Option<DateTime<Utc>>,
}

impl Device {
	pub fn from_row(row: &Row) -> Device {
		Device {
			id: row.get("id"),
			name: row.get("name"),
			created_at: row.get("created_at"),
			revoked_at: row.get("revoked_at"),
		}
	}
}

/// A sale, with the payment details of its rails.
#[derive(Debug, Clone)]
pub struct Sale {
	pub id: i64,
	pub public_id: String,
	pub device_name: Option<String>,
	pub amount_sat: i64,
	pub fiat_amount: Option<String>,
	pub currency: Option<String>,
	pub btc_price: Option<String>,
	pub memo: Option<String>,
	pub state: SaleState,
	pub paid_rail: Option<Rail>,
	pub received_sat: i64,
	pub created_at: DateTime<Utc>,
	pub expires_at: DateTime<Utc>,
	pub paid_at: Option<DateTime<Utc>>,
	pub lightning_invoice: String,
	/// Whether the sale offers on-chain, or can once the cashier picks it:
	/// a terminal's sale of at least the store's on-chain minimum.
	pub onchain_available: bool,
	pub onchain_address: Option<String>,
	/// Confirmed to the on-chain address.
	pub onchain_received_sat: i64,
	/// In the mempool to the on-chain address.
	pub onchain_pending_sat: i64,
}

/// Selects a sale joined with its store, its device and its rails. Callers
/// add the WHERE clause. Every sale has a Lightning rail; it has an
/// on-chain one once the cashier picks on-chain.
pub const SALE_SELECT: &str = "
	SELECT s.id, s.public_id, d.name AS device_name, s.amount_sat, s.fiat_amount,
		s.currency, s.btc_price, s.memo, s.state, s.paid_rail, s.received_sat, s.created_at,
		s.expires_at, s.paid_at, ln.invoice AS lightning_invoice,
		oc.id IS NOT NULL OR (s.device_id IS NOT NULL AND s.amount_sat >= st.onchain_min_sat)
			AS onchain_available,
		oc.onchain_address, coalesce(oc.received_sat, 0) AS onchain_received_sat,
		coalesce(oc.pending_sat, 0) AS onchain_pending_sat
	FROM sale s
	JOIN store st ON st.id = s.store_id
	LEFT JOIN device d ON d.id = s.device_id
	JOIN sale_payment ln ON ln.sale_id = s.id AND ln.rail = 'lightning'
	LEFT JOIN sale_payment oc ON oc.sale_id = s.id AND oc.rail = 'onchain'
";

impl Sale {
	pub fn from_row(row: &Row) -> Sale {
		Sale {
			id: row.get("id"),
			public_id: row.get("public_id"),
			device_name: row.get("device_name"),
			amount_sat: row.get("amount_sat"),
			fiat_amount: row.get("fiat_amount"),
			currency: row.get("currency"),
			btc_price: row.get("btc_price"),
			memo: row.get("memo"),
			state: row.get("state"),
			paid_rail: row.get("paid_rail"),
			received_sat: row.get("received_sat"),
			created_at: row.get("created_at"),
			expires_at: row.get("expires_at"),
			paid_at: row.get("paid_at"),
			lightning_invoice: row.get("lightning_invoice"),
			onchain_available: row.get("onchain_available"),
			onchain_address: row.get("onchain_address"),
			onchain_received_sat: row.get("onchain_received_sat"),
			onchain_pending_sat: row.get("onchain_pending_sat"),
		}
	}
}

/// A payment rail of a sale that the watcher checks.
#[derive(Debug, Clone)]
pub struct WatchedPayment {
	pub payment_id: i64,
	pub sale_id: i64,
	pub sale_public_id: String,
	pub rail: Rail,
	pub payment_hash: Option<String>,
	pub onchain_address: Option<String>,
	pub received_sat: i64,
	pub pending_sat: i64,
}

impl WatchedPayment {
	pub fn from_row(row: &Row) -> WatchedPayment {
		WatchedPayment {
			payment_id: row.get("payment_id"),
			sale_id: row.get("sale_id"),
			sale_public_id: row.get("public_id"),
			rail: row.get("rail"),
			payment_hash: row.get("payment_hash"),
			onchain_address: row.get("onchain_address"),
			received_sat: row.get("received_sat"),
			pending_sat: row.get("pending_sat"),
		}
	}
}
