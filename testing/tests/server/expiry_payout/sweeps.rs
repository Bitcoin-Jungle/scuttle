//! Watchman sweeps that the fork adds: an operator claim next to an expiry
//! payout.

use std::time::Duration;

use bitcoin_ext::BlockDelta;
use bitcoin_ext::rpc::RpcApi;
use server::database::Db;
use server_log::{ClaimBroadcast, ClaimBroadcastFailure, ClaimChunkBroadcastFailure, ProgressCpfpFailure};

use ark_testing::{btc, sat, TestContext};
use ark_testing::exit::progress_exit_until_awaiting_delta;
use ark_testing::util::FutureExt;

use super::{assert_paid, wait_payout};

/// In expiry payout mode the watchman still confiscates and sweeps the
/// forfeit of an offboard whose input was exited, and pays an expired board
/// in the same pass.
#[tokio::test]
async fn watchman_sweeps_offboard_forfeit_and_pays_expired_board() {
	let ctx = TestContext::new("server/watchman_sweeps_offboard_forfeit_and_pays_expired_board").await;
	let srv = ctx.captaind("server").funded(btc(10)).cfg(|cfg| {
		cfg.vtxo_lifetime = BlockDelta::new(144);
		cfg.vtxopool.vtxo_targets = vec![];
		cfg.expiry_payouts = true;
	}).watchmand_cfg(|cfg| {
		cfg.watchman.reaction_interval = Duration::from_secs(15 * 60);
		cfg.watchman.sweep_interval = Duration::from_secs(15 * 60);
	}).create().await;
	let wm = srv.watchmand();
	let mut chunk_failures = wm.subscribe_log::<ClaimChunkBroadcastFailure>();
	let mut claim_failures = wm.subscribe_log::<ClaimBroadcastFailure>();
	let mut cpfp_failures = wm.subscribe_log::<ProgressCpfpFailure>();
	let mut log_claim = wm.subscribe_log::<ClaimBroadcast>();

	// fund the watchman so it can pay CPFP fees for the forfeit broadcast
	ctx.bitcoind().fund_addr(wm.wait_wallet_address().await, sat(1_000_000)).await;

	let bark = ctx.bark("bark1", &srv).funded(sat(1_000_000)).create().await;
	bark.board_and_confirm_and_register(&ctx, sat(400_000)).await;
	bark.sync().await;
	let exit_point = bark.vtxo_ids().await[0].to_point();

	// snapshot the wallet before offboarding; the attacker keeps the stale vtxo
	let evil = bark.full_clone("evil").await;

	let payout = ctx.bitcoind().get_new_address();
	bark.offboard_all(&payout).await;
	ctx.generate_blocks(1).await;

	// a board that expires with the offboarded one and is paid to its owner
	let owner = ctx.bark_sdk("owner", &srv).boarded(sat(100_000))
		.cfg(|cfg| cfg.daemon_manual_sync = true).create().await;
	let owner_vtxo = owner.spendable_vtxos().await.unwrap()[0].id();
	let owner_script = owner.payout_script();

	// the attacker exits the now-forfeited vtxo
	evil.start_exit_all().await;
	progress_exit_until_awaiting_delta(&ctx, &evil).await;

	// drive the watchman until it confiscates the exit output
	let tip = ctx.generate_blocks(wm.config().watchman.progress_grace_period.to_u32()).await;
	wm.wait_for_sync_height(tip).await;
	let client = ctx.bitcoind().sync_client();
	for _ in 0..25 {
		wm.trigger_sweep().await;
		tokio::time::sleep(Duration::from_secs(2)).await;
		let tip = ctx.generate_blocks(1).await;
		wm.wait_for_sync_height(tip).await;
		if client.get_tx_out(&exit_point.txid, exit_point.vout, Some(true)).unwrap().is_none() {
			break;
		}
	}
	assert!(
		client.get_tx_out(&exit_point.txid, exit_point.vout, Some(true)).unwrap().is_none(),
		"the watchman should have confiscated the exit output",
	);

	// Once the trees expired, the watchman sweeps the confiscated forfeit
	// output: the input amount plus the connector dust.
	let tip = ctx.generate_blocks(200).await;
	wm.wait_for_sync_height(tip).await;

	wm.trigger_sweep().await;
	let msg = log_claim.recv().wait_millis(15000).await.expect("no claim log");
	assert!(chunk_failures.try_recv().is_err() && claim_failures.try_recv().is_err()
		&& cpfp_failures.try_recv().is_err(), "the watchman logged a failed claim");
	assert_eq!(400_330, msg.total_input_value.to_sat());
	let db = Db::connect(&srv.config().postgres).await.unwrap();
	let (_, payout) = wait_payout(&db, owner_vtxo).await;

	// the forfeit output must actually be gone from the utxo set
	let forfeit_point = msg.vtxo_ids[0].to_point();
	let tip = ctx.generate_blocks(1).await;
	srv.wait_for_sync_height(tip).await;
	assert!(
		client.get_tx_out(&forfeit_point.txid, forfeit_point.vout, Some(true)).unwrap().is_none(),
		"forfeit output {} should be spent by the sweep", forfeit_point,
	);

	assert_paid(&ctx, &srv, &payout, &owner_script, sat(98_000)).await;
}
