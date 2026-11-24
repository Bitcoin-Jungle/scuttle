//! Canceling a Lightning receive whose invoice the server already canceled.

use std::time::Duration;

use ark::lightning::PaymentHash;
use server_rpc::protos;

use ark_testing::{TestContext, btc, sat, util::FutureExt};

/// The server cancels an invoice that expired unpaid. Canceling the receive
/// in the wallet afterwards succeeds and removes it.
#[tokio::test]
async fn cancel_of_a_receive_the_server_canceled() {
	let ctx = TestContext::new("bark_sdk/cancel_of_a_receive_the_server_canceled").await;
	let lightning = ctx.new_lightning_setup("lightningd").await;
	let srv = ctx.captaind("server").lightningd(&lightning.internal).funded(btc(10)).create().await;
	let wallet = ctx.bark_sdk("receiver", &srv).cfg(|cfg| cfg.daemon_manual_sync = true).create().await;

	let invoice = wallet.bolt11_invoice_builder().amount(sat(50_000)).expiry(Duration::from_secs(2))
		.build().await.unwrap();
	let hash = PaymentHash::from(&invoice);
	let status = srv.get_public_rpc().await.check_lightning_receive(protos::CheckLightningReceiveRequest {
		hash: hash.to_vec(), wait: true,
	}).wait_millis(30_000).await.unwrap().into_inner().status();
	assert_eq!(status, protos::LightningReceiveStatus::Canceled);
	assert_eq!(wallet.pending_lightning_receives().await.unwrap().len(), 1);

	wallet.cancel_lightning_receive(hash).await.expect("the receive closes");
	assert!(wallet.pending_lightning_receives().await.unwrap().is_empty());
}
