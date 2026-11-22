//! The connection to the Ark server.
//!
//! posd only reads the server's parameters: its network and public key,
//! which a store registration must match. The payments themselves go
//! through barkd and the chain.

use anyhow::Context;
use ark::ArkInfo;
use server_rpc::ServerConnection;

use crate::config;


pub struct ArkServer {
	conn: ServerConnection,
}

impl ArkServer {
	pub async fn connect(cfg: &config::Config) -> anyhow::Result<ArkServer> {
		let conn = ServerConnection::builder()
			.address(&cfg.ark_server_url)
			.network(cfg.network)
			.user_agent(format!("posd/{}", env!("CARGO_PKG_VERSION")))
			.connect().await
			.context("failed to connect to the Ark server")?;
		Ok(ArkServer { conn })
	}

	pub async fn ark_info(&self) -> ArkInfo {
		self.conn.ark_info().await
	}
}
