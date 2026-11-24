use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bitcoin::Amount;

use ark::fees::{IntraArkFees, LightningReceiveFees, LightningSendFees, PpmFeeRate};
use ark::lightning::{Bolt11Invoice, Invoice, PaymentHash};
use bark::actions::lightning::pay::LightningSendState;
use bark::actions::lightning::receive::LightningReceiveState;
use server::database::Db;
use server_rpc::protos;

use ark_testing::{TestContext, btc, sat, util::{FutureExt, poll_interval}};
use ark_testing::balance::assert_balance_consistent;
use ark_testing::daemon::captaind::{self, ArkClient};

/// Claims a receive and waits until it is settled. The wallet's own sync
/// can be claiming it already, in which case the claim call returns early.
async fn claim_until_settled(wallet: &bark::Wallet, hash: PaymentHash) {
	let _ = wallet.try_claim_lightning_receive(hash, true).await;
	async {
		loop {
			if let LightningReceiveState::Settled(_) = wallet.lightning_receive_state(hash).await.unwrap() {
				return;
			}
			tokio::time::sleep(poll_interval()).await;
		}
	}.wait_millis(30_000).await;
}

fn flat_send_fees(base: Amount) -> LightningSendFees {
	LightningSendFees { min_fee: Amount::ZERO, base_fee: base, ppm_expiry_table: vec![] }
}

fn flat_receive_fees(base: Amount) -> LightningReceiveFees {
	LightningReceiveFees { base_fee: base, ppm: PpmFeeRate::ZERO }
}

#[derive(Clone, Default)]
struct FailFirstReceiveLookup(Arc<AtomicBool>);

#[async_trait::async_trait]
impl captaind::proxy::ArkRpcProxy for FailFirstReceiveLookup {
	async fn check_lightning_receive(
		&self, upstream: &mut ArkClient, req: protos::CheckLightningReceiveRequest,
	) -> Result<protos::CheckLightningReceiveResponse, tonic::Status> {
		if !self.0.swap(true, Ordering::Relaxed) {
			return Err(tonic::Status::unavailable("receive lookup unavailable"));
		}
		Ok(upstream.check_lightning_receive(req).await?.into_inner())
	}
}

/// A Lightning payment between two wallets of the same server pays the
/// intra-Ark fees on both sides. Payments to and from the outside still pay
/// the normal Lightning fees.
#[tokio::test]
async fn intra_ark_payment_pays_intra_ark_fees() {
	let ctx = TestContext::new("bark_sdk/intra_ark_payment_pays_intra_ark_fees").await;
	let lightning = ctx.new_lightning_setup("lightningd").await;
	let srv = ctx.captaind("server").lightningd(&lightning.internal).funded(btc(10))
		.cfg(|cfg| {
			cfg.expiry_payouts = true;
			cfg.fees.lightning_send = flat_send_fees(sat(75));
			cfg.fees.lightning_receive = flat_receive_fees(sat(100));
			cfg.fees.intra_ark = Some(IntraArkFees {
				lightning_send: flat_send_fees(sat(10)),
				lightning_receive: flat_receive_fees(sat(7)),
			});
		})
		.create().await;
	srv.wait_for_vtxopool(&ctx).await;
	let board = sat(400_000);
	let proxy = srv.start_proxy_no_mailbox(FailFirstReceiveLookup::default()).await;
	let payer = ctx.bark_sdk("payer", &proxy).boarded(board).create().await;
	let receiver = ctx.bark_sdk("receiver", &srv).create().await;
	lightning.sync().await;

	let postgres = srv.config().postgres.clone();
	// Let the runtime poll the wallets independently of the daemon setup future.
	tokio::spawn(async move {
		// The two wallets pay the configured send and receive fees.
		let amount = sat(50_000);
		let invoice = receiver.bolt11_invoice(amount, None, None).await.unwrap();
		let hash = PaymentHash::from(&invoice);
		let err = payer.pay_lightning_invoice(invoice.clone(), None, false).await.unwrap_err();
		assert!(format!("{err:#}").contains("could not determine the Lightning send fee"), "{err:#}");

		let estimate = payer.estimate_invoice_payment_fee(&Invoice::Bolt11(invoice.clone()), amount).await.unwrap();
		assert_eq!(estimate.fee, sat(10));
		payer.pay_lightning_invoice(invoice, None, false).await.unwrap();
		srv.wait_for_lightning_receive_accepted(hash).await;
		receiver.cancel_lightning_receive(hash).await.expect_err("accepted payment must stay claimable");
		claim_until_settled(&receiver, hash).await;
		let sent = payer.check_lightning_payment(hash, true).await.unwrap();
		let LightningSendState::Paid(proof) = sent else { panic!("payment did not settle: {sent:?}") };
		assert_eq!(PaymentHash::from(proof.preimage), hash);
		let db = Db::connect(&postgres).await.unwrap();
		let attempt = db.read(async |t| t.get_latest_payment_attempt_by_payment_hash(hash).await).await.unwrap().unwrap();
		assert!(attempt.lightning_htlc_subscription_id.is_some());
		assert_eq!(attempt.final_amount_msat, None, "no payment was sent over a Lightning channel");
		assert_eq!(assert_balance_consistent(&payer, false).await.spendable, board - amount - sat(10));
		assert_eq!(assert_balance_consistent(&receiver, false).await.spendable, amount - sat(7));

		// To the outside: the normal send fee.
		let out_amount = sat(30_000);
		let outgoing = lightning.external.invoice(Some(out_amount), "intra-ark-fees-out", "out").await;
		let outgoing = Invoice::Bolt11(outgoing.parse::<Bolt11Invoice>().unwrap());
		let estimate = payer.estimate_invoice_payment_fee(&outgoing, out_amount).await.unwrap();
		assert_eq!(estimate.fee, sat(75));
		payer.pay_lightning_invoice(outgoing, None, true).await.unwrap();
		assert_eq!(assert_balance_consistent(&payer, false).await.spendable,
			board - amount - sat(10) - out_amount - sat(75));

		// From the outside: the normal receive fee.
		let in_amount = sat(20_000);
		let incoming = receiver.bolt11_invoice(in_amount, None, None).await.unwrap();
		let hash = PaymentHash::from(&incoming);
		let (paid, ()) = tokio::join!(
			lightning.external.try_pay_bolt11(incoming.to_string()),
			claim_until_settled(&receiver, hash),
		);
		paid.expect("external payment failed");
		assert_eq!(assert_balance_consistent(&receiver, false).await.spendable, amount - sat(7) + in_amount - sat(100));
	}).await.unwrap();
}

/// Without intra-Ark fees, an intra-Ark payment pays the normal fees, and
/// the wallet does not ask the server about the invoice.
#[tokio::test]
async fn intra_ark_payment_without_intra_ark_fees() {
	let ctx = TestContext::new("bark_sdk/intra_ark_payment_without_intra_ark_fees").await;
	let lightning = ctx.new_lightning_setup("lightningd").await;
	let srv = ctx.captaind("server").lightningd(&lightning.internal).funded(btc(10))
		.cfg(|cfg| {
			cfg.fees.lightning_send = flat_send_fees(sat(75));
			cfg.fees.lightning_receive = flat_receive_fees(sat(100));
		})
		.create().await;
	srv.wait_for_vtxopool(&ctx).await;
	let board = sat(400_000);
	let proxy = srv.start_proxy_no_mailbox(FailFirstReceiveLookup::default()).await;
	let payer = ctx.bark_sdk("payer", &proxy).boarded(board).create().await;
	let receiver = ctx.bark_sdk("receiver", &srv).create().await;

	let amount = sat(50_000);
	let invoice = receiver.bolt11_invoice(amount, None, None).await.unwrap();
	let hash = PaymentHash::from(&invoice);
	let estimate = payer.estimate_invoice_payment_fee(&Invoice::Bolt11(invoice.clone()), amount).await.unwrap();
	assert_eq!(estimate.fee, sat(75));
	payer.pay_lightning_invoice(invoice, None, false).await.unwrap();
	claim_until_settled(&receiver, hash).await;
	payer.check_lightning_payment(hash, true).await.unwrap();
	assert_eq!(assert_balance_consistent(&payer, false).await.spendable, board - amount - sat(75));
	assert_eq!(assert_balance_consistent(&receiver, false).await.spendable, amount - sat(100));
}
