//! End-to-end tests of posd, the point-of-sale service.
//!
//! posd runs in the test process against a real Ark server, a barkd that
//! forwards Lightning receives, lightningd nodes and bitcoind. The store
//! owner's wallet registers the store and then goes offline for every sale.

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use bitcoin::bip32::{Xpriv, Xpub};
use bitcoin::{Amount, Network};
use bitcoin_ext::BlockDelta;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use ark::SECP;
use ark::fees::{IntraArkFees, LightningReceiveFees, LightningSendFees, PpmFeeRate};
use bark::Wallet;
use bark::payment_request::PaymentRequest;

use ark_testing::{Captaind, TestContext, btc, sat};
use ark_testing::context::LightningPaymentSetup;
use ark_testing::daemon::captaind::ArkClient;
use ark_testing::daemon::captaind::proxy::{ArkRpcProxy, ArkRpcProxyServer};
use ark_testing::util::FutureExt;
use server_rpc::protos;
use ark_testing::pos::{ALPHA, BETA, BETA_CSS, Posd, admin_token, brand};
use pos::api::lnurl::MAX_OPEN_PER_STORE;
use pos::config::Config;
use pos::rails::onchain::StoreDescriptor;
use pos::registration::Registration;


const SALE_TIMEOUT: Duration = Duration::from_secs(90);
const LIGHTNING_SEND_FEE: Amount = Amount::from_sat(75);
const LIGHTNING_RECEIVE_FEE: Amount = Amount::from_sat(100);

/// A store: the Ark server, posd, an owner token and a paired terminal.
struct Store {
	srv: Arc<Captaind>,
	lightning: LightningPaymentSetup,
	posd: Posd,
	owner_seed: bip39::Mnemonic,
	owner_token: String,
	till: String,
	// Kept alive for posd.
	_forwarder: ark_testing::Barkd,
	_forwarder_proxy: Option<ArkRpcProxyServer>,
}

/// Holds the forwarder's receive claims while `hold` is set. The forwarder
/// has revealed the preimage, so the payment is on its way to the owner,
/// but it cannot be delivered yet.
#[derive(Clone, Default)]
struct HoldClaims {
	hold: Arc<AtomicBool>,
	held: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl ArkRpcProxy for HoldClaims {
	async fn claim_lightning_receive(
		&self, upstream: &mut ArkClient, request: protos::ClaimLightningReceiveRequest,
	) -> Result<protos::ArkoorPackageCosignResponse, tonic::Status> {
		if self.hold.load(Ordering::SeqCst) {
			self.held.notify_one();
			while self.hold.load(Ordering::SeqCst) {
				tokio::time::sleep(Duration::from_millis(100)).await;
			}
		}
		Ok(upstream.claim_lightning_receive(request).await?.into_inner())
	}
}

/// The public descriptor of the host app's on-chain account of the owner.
fn owner_descriptor() -> String {
	let xpriv = Xpriv::new_master(Network::Regtest, &[42u8; 32]).unwrap();
	format!("tr({}/0/*)", Xpub::from_priv(&SECP, &xpriv))
}

async fn store_setup(ctx: &TestContext) -> Store {
	store_setup_with(ctx, |_| {}).await
}

/// A store on a posd with a config `change` makes.
async fn store_setup_with(ctx: &TestContext, change: impl FnOnce(&mut Config)) -> Store {
	store_setup_held(ctx, change, None).await
}

/// A store whose forwarder reaches the Ark server through `hold`, if given.
async fn store_setup_held(ctx: &TestContext, change: impl FnOnce(&mut Config), hold: Option<HoldClaims>) -> Store {
	let lightning = ctx.new_lightning_setup("lightningd").await;
	let srv = ctx.captaind("server").lightningd(&lightning.internal).funded(btc(10))
		.cfg(|cfg| {
			cfg.expiry_payouts = true;
			// Lightning payments from and to the outside pay fees; payments
			// inside the Ark are free.
			cfg.fees.lightning_send = LightningSendFees {
				min_fee: Amount::ZERO, base_fee: LIGHTNING_SEND_FEE, ppm_expiry_table: vec![],
			};
			cfg.fees.lightning_receive = LightningReceiveFees {
				base_fee: LIGHTNING_RECEIVE_FEE, ppm: PpmFeeRate::ZERO,
			};
			cfg.fees.intra_ark = Some(IntraArkFees {
				lightning_send: LightningSendFees {
					min_fee: Amount::ZERO, base_fee: Amount::ZERO, ppm_expiry_table: vec![],
				},
				lightning_receive: LightningReceiveFees { base_fee: Amount::ZERO, ppm: PpmFeeRate::ZERO },
			});
		}).create().await;
	// Lightning receives draw their VTXOs from the pool. Confirm the pool
	// transaction, so a node that starts later knows the VTXO anchor.
	srv.wait_for_vtxopool(ctx).await;
	ctx.generate_blocks(1).await;
	let forwarder_proxy = match hold {
		Some(hold) => Some(srv.start_proxy_no_mailbox(hold).await),
		None => None,
	};
	let forwarder = match &forwarder_proxy {
		Some(proxy) => ctx.barkd("forwarder", proxy).create().await,
		None => ctx.barkd("forwarder", &*srv).create().await,
	};

	// The owner registers the store, then goes offline.
	let owner_seed = bip39::Mnemonic::generate(12).unwrap();
	let owner = ctx.bark_sdk("owner", &srv).mnemonic(owner_seed.clone()).create().await;
	// The owner's wallet signs the registration with its plain message
	// signing: nothing in the wallet is specific to the point of sale.
	let address = owner.new_address().await.unwrap();
	let registration = Registration {
		name: "Corner Café".into(),
		ark_address: address.to_string(),
		onchain_descriptor: owner_descriptor(),
		currency: "CRC".into(),
		brand: brand(ALPHA),
		lightning_address_name: String::new(),
		timestamp: chrono::Utc::now().timestamp() as u64,
	};
	let signature = owner.sign_message(registration.message().as_bytes(), &address)
		.await.unwrap().expect("the owner holds the key");

	let posd = Posd::start_with(ctx, &srv, &forwarder, change).await;
	posd.register(serde_json::to_value(&registration).unwrap(), signature.to_string()).await;
	let challenge = posd.challenge(&registration.ark_address).await;
	let login_sig = owner.sign_message(challenge.as_bytes(), &address)
		.await.unwrap().expect("the owner holds the key");
	let owner_token = posd.login(&registration.ark_address, &challenge, login_sig.to_string()).await;
	// A challenge works once.
	let (status, _) = posd.call(Method::POST, "/api/v1/owner/login", None, Some(json!({
		"ark_address": registration.ark_address, "challenge": challenge,
		"signature": login_sig.to_string(),
	}))).await;
	assert_eq!(status, StatusCode::BAD_REQUEST);
	// The owner's wallet goes offline: its daemon stops reading the mailbox.
	owner.stop_daemon_wait().await.unwrap();
	drop(owner);
	let till = posd.pair(&owner_token, "Front till").await;

	Store { srv, lightning, posd, owner_seed, owner_token, till, _forwarder: forwarder, _forwarder_proxy: forwarder_proxy }
}

async fn new_sale(store: &Store, body: Value) -> Value {
	store.posd.ok(Method::POST, "/api/v1/sales", Some(&store.till), Some(body)).await
}

/// A payer that pays a payment request the default way.
async fn pay(payer: &Wallet, payment_request: &str) -> String {
	let req: PaymentRequest = payer.parse_payment_request(payment_request).await.unwrap();
	let option = req.default_option().expect("no usable payment option").clone();
	let kind = option.method.type_str().to_string();
	// Poll the SDK payment on its own task, outside the recovery test's stack frame.
	let payer = payer.clone();
	tokio::spawn(async move {
		payer.send_payment(&option.method, None, None::<String>, true).await.unwrap();
	}).await.unwrap();
	kind
}

/// Syncs `wallet` until it has `expected` spendable.
async fn wait_spendable(wallet: &Wallet, expected: Amount) {
	let deadline = tokio::time::Instant::now() + SALE_TIMEOUT;
	loop {
		wallet.sync().await;
		let spendable = wallet.balance().await.unwrap().spendable;
		if spendable == expected {
			return;
		}
		assert!(tokio::time::Instant::now() < deadline,
			"owner has {} spendable, expected {}", spendable, expected);
		tokio::time::sleep(Duration::from_millis(500)).await;
	}
}

/// The owner's phone comes back online and its wallet has the funds.
async fn owner_comes_back(ctx: &TestContext, store: &Store, expected: Amount) -> Wallet {
	let owner = ctx.bark_sdk("owner", &store.srv).mnemonic(store.owner_seed.clone()).create().await;
	wait_spendable(&owner, expected).await;
	owner
}

/// The owner restores the wallet from the seed phrase on a new phone and
/// has the funds.
async fn owner_restores(ctx: &TestContext, store: &Store, expected: Amount) -> Wallet {
	let restored = ctx.bark_sdk("owner-new-phone", &store.srv)
		.mnemonic(store.owner_seed.clone()).create().await;
	wait_spendable(&restored, expected).await;
	restored
}

#[tokio::test]
async fn pos_sale_paid_by_lightning() {
	let ctx = TestContext::new("pos/pos_sale_paid_by_lightning").await;
	let store = store_setup(&ctx).await;

	// 12,500 CRC at the test price is 20,000 sats, below the on-chain minimum.
	let sale = new_sale(&store, json!({ "fiat_amount": "12500", "currency": "CRC", "memo": "2 coffees" })).await;
	assert_eq!(sale["state"], "open");
	assert_eq!(sale["amount_sat"], 20_000);
	assert_eq!(sale["fiat_amount"], "12500.00");
	assert_eq!(sale["onchain_address"], Value::Null);
	let pr = sale["payment_request"].as_str().unwrap();
	assert_eq!(pr, format!("bitcoin:?lightning={}", sale["lightning_invoice"].as_str().unwrap()));

	store.lightning.external.pay_bolt11(sale["lightning_invoice"].as_str().unwrap()).await;
	let id = sale["id"].as_str().unwrap();
	let paid = store.posd.wait_sale_state(&store.till, id, "paid", SALE_TIMEOUT).await;
	assert_eq!(paid["paid_rail"], "lightning");

	// The owner pays the Lightning receive fee on a payment from outside.
	owner_comes_back(&ctx, &store, sat(20_000) - LIGHTNING_RECEIVE_FEE).await;

	let csv = reqwest::Client::new().get(format!("{}/api/v1/sales.csv", store.posd.url))
		.bearer_auth(&store.till).send().await.unwrap().text().await.unwrap();
	assert!(csv.contains(id) && csv.contains("2 coffees") && csv.contains(",paid,lightning,"), "{}", csv);
}

/// A Lightning payment that is on its way when its sale expires keeps the
/// sale open, so the terminal does not offer to charge the customer again.
/// The sale is paid once the payment is delivered.
#[tokio::test]
async fn pos_sale_waits_for_an_arriving_lightning_payment() {
	let ctx = TestContext::new("pos/pos_sale_waits_for_an_arriving_lightning_payment").await;
	let hold = HoldClaims::default();
	let expiry = Duration::from_secs(15);
	let store = store_setup_held(&ctx, |cfg| cfg.sales.expiry_secs = expiry.as_secs(), Some(hold.clone())).await;
	hold.hold.store(true, Ordering::SeqCst);

	let sale = new_sale(&store, json!({ "amount_sat": 20_000 })).await;
	let created = Instant::now();
	let id = sale["id"].as_str().unwrap().to_string();
	let invoice = sale["lightning_invoice"].as_str().unwrap().to_string();
	let (paid, ()) = tokio::join!(store.lightning.external.try_pay_bolt11(invoice), async {
		hold.held.notified().wait(SALE_TIMEOUT).await;
		// Past the expiry, with time for the watcher to run several passes.
		tokio::time::sleep((expiry + Duration::from_secs(4)).saturating_sub(created.elapsed())).await;
		let open = store.posd.ok(Method::GET, &format!("/api/v1/sales/{}", id), Some(&store.till), None).await;
		assert_eq!(open["state"], "open", "the sale expired while its payment was on its way");
		hold.hold.store(false, Ordering::SeqCst);
	});
	paid.expect("the customer's payment completes");
	let paid = store.posd.wait_sale_state(&store.till, &id, "paid", SALE_TIMEOUT).await;
	assert_eq!(paid["paid_rail"], "lightning");
}

/// A sale request repeated with its Idempotency-Key, as a terminal does after
/// losing the response, returns the first sale instead of a second one.
#[tokio::test]
async fn pos_sale_request_key() {
	let ctx = TestContext::new("pos/pos_sale_request_key").await;
	let store = store_setup(&ctx).await;
	async fn create(store: &Store, key: &str, body: Value) -> (StatusCode, Value) {
		let resp = reqwest::Client::new().post(format!("{}/api/v1/sales", store.posd.url))
			.bearer_auth(&store.till).header("Idempotency-Key", key).json(&body)
			.send().await.unwrap();
		(resp.status(), resp.json().await.unwrap())
	}

	let (status, first) = create(&store, "k1", json!({ "amount_sat": 5_000, "memo": "m" })).await;
	assert_eq!(status, StatusCode::OK, "{}", first);
	let (_, again) = create(&store, "k1", json!({ "amount_sat": 5_000, "memo": "m" })).await;
	assert_eq!(again["id"], first["id"]);
	assert_eq!(again["lightning_invoice"], first["lightning_invoice"]);

	// The same key for another sale is a client bug, not a repeat.
	let (status, _) = create(&store, "k1", json!({ "amount_sat": 6_000, "memo": "m" })).await;
	assert_eq!(status, StatusCode::CONFLICT);
	let (status, _) = create(&store, "not a key", json!({ "amount_sat": 5_000 })).await;
	assert_eq!(status, StatusCode::BAD_REQUEST);

	let (_, other) = create(&store, "k2", json!({ "amount_sat": 5_000, "memo": "m" })).await;
	assert_ne!(other["id"], first["id"]);
	let sales = store.posd.ok(Method::GET, "/api/v1/sales", Some(&store.till), None).await;
	assert_eq!(sales["sales"].as_array().unwrap().len(), 2);
}

/// A wallet on the same Ark server pays the Lightning invoice inside the
/// Ark: neither side pays a fee, and the owner gets VTXOs from the server's
/// pool, which outlive the payer's.
#[tokio::test]
async fn pos_sale_paid_from_the_same_ark() {
	let ctx = TestContext::new("pos/pos_sale_paid_from_the_same_ark").await;
	let store = store_setup(&ctx).await;
	// The payer's VTXOs expire well before the pool's.
	let payer = ctx.bark_sdk("payer", &store.srv).boarded(sat(200_000))
		.cfg(|cfg| cfg.board_lifetime = BlockDelta::new(350))
		.create().await;
	let payer_expiry = payer.vtxos().await.unwrap().iter()
		.map(|v| v.expiry_height()).max().expect("the payer has VTXOs");

	let sale = new_sale(&store, json!({ "amount_sat": 30_000 })).await;
	let id = sale["id"].as_str().unwrap();
	let kind = pay(&payer, sale["payment_request"].as_str().unwrap()).await;
	let paid = store.posd.wait_sale_state(&store.till, id, "paid", SALE_TIMEOUT).await;
	assert_eq!(paid["paid_rail"], "lightning", "payer chose {}", kind);
	assert_eq!(paid["received_sat"], 30_000);
	assert_eq!(payer.balance().await.unwrap().spendable, sat(170_000), "the payer paid a fee");

	// The owner paid no fee.
	let owner = owner_comes_back(&ctx, &store, sat(30_000)).await;
	for vtxo in owner.vtxos().await.unwrap() {
		assert!(vtxo.expiry_height() > payer_expiry,
			"the owner's VTXO {} expires at {}, not after the payer's {}",
			vtxo.id(), vtxo.expiry_height(), payer_expiry);
	}
}

/// The owner loses the phone. Sales are paid while the owner has no wallet,
/// then the owner restores the seed on a new phone, which recovers them and
/// receives the sales paid after it.
#[tokio::test]
async fn pos_owner_restores_on_a_new_phone() {
	let ctx = TestContext::new("pos/pos_owner_restores_on_a_new_phone").await;
	let store = store_setup(&ctx).await;
	let payer = ctx.bark_sdk("payer", &store.srv).boarded(sat(200_000)).create().await;

	// Paid from outside and from the same Ark while the owner has no phone.
	let sale = new_sale(&store, json!({ "amount_sat": 20_000 })).await;
	store.lightning.external.pay_bolt11(sale["lightning_invoice"].as_str().unwrap()).await;
	store.posd.wait_sale_state(&store.till, sale["id"].as_str().unwrap(), "paid", SALE_TIMEOUT).await;
	let sale = new_sale(&store, json!({ "amount_sat": 30_000 })).await;
	pay(&payer, sale["payment_request"].as_str().unwrap()).await;
	store.posd.wait_sale_state(&store.till, sale["id"].as_str().unwrap(), "paid", SALE_TIMEOUT).await;

	let before = sat(20_000) - LIGHTNING_RECEIVE_FEE + sat(30_000);
	let phone = owner_restores(&ctx, &store, before).await;

	// The store keeps selling, and the new phone receives it.
	let sale = new_sale(&store, json!({ "amount_sat": 10_000 })).await;
	pay(&payer, sale["payment_request"].as_str().unwrap()).await;
	store.posd.wait_sale_state(&store.till, sale["id"].as_str().unwrap(), "paid", SALE_TIMEOUT).await;
	wait_spendable(&phone, before + sat(10_000)).await;
}

#[tokio::test]
async fn pos_sale_paid_onchain() {
	let ctx = TestContext::new("pos/pos_sale_paid_onchain").await;
	let store = store_setup(&ctx).await;

	// At or above the 50,000-sat minimum, the sale offers an address.
	let sale = new_sale(&store, json!({ "amount_sat": 60_000 })).await;
	let id = sale["id"].as_str().unwrap();
	let address = sale["onchain_address"].as_str().expect("an on-chain address").to_string();
	assert!(sale["payment_request"].as_str().unwrap()
		.starts_with(&format!("bitcoin:{}?amount=0.0006&lightning=", address)));
	let second = new_sale(&store, json!({ "amount_sat": 60_000 })).await;
	assert_ne!(second["onchain_address"].as_str().unwrap(), address, "each sale has its own address");

	// The payment is in the mempool, and posd's chain source has it. A
	// cancel now, before the watcher's next look at the address, finds it
	// and refuses: the customer paid.
	ctx.bitcoind().fund_addr(&address, sat(60_000)).await;
	let chain_address = bitcoin::Address::from_str(&address).unwrap().assume_checked();
	let deadline = Instant::now() + SALE_TIMEOUT;
	while store.posd.app.chain.funds(&chain_address).await.unwrap().pending == Amount::ZERO {
		assert!(Instant::now() < deadline, "the chain source does not see the payment");
		tokio::time::sleep(Duration::from_millis(200)).await;
	}
	let (status, body) = store.posd.call(Method::POST, &format!("/api/v1/sales/{}/cancel", id),
		Some(&store.till), None).await;
	assert_eq!(status, StatusCode::CONFLICT, "{}", body);
	store.posd.wait_sale_state(&store.till, id, "in-progress", SALE_TIMEOUT).await;
	ctx.generate_blocks(1).await;
	let paid = store.posd.wait_sale_state(&store.till, id, "paid", SALE_TIMEOUT).await;
	assert_eq!(paid["paid_rail"], "onchain");
	assert_eq!(paid["received_sat"], 60_000);
}

#[tokio::test]
async fn pos_pairing_and_lightning_address() {
	let ctx = TestContext::new("pos/pos_pairing_and_lightning_address").await;
	let store = store_setup(&ctx).await;
	let posd = &store.posd;

	// A pairing code works once.
	let invite = posd.ok(Method::POST, "/api/v1/owner/invites", Some(&store.owner_token), None).await;
	let body = json!({ "code": invite["code"], "device_name": "Back till" });
	let paired = posd.ok(Method::POST, "/api/v1/pair", None, Some(body.clone())).await;
	let (status, _) = posd.call(Method::POST, "/api/v1/pair", None, Some(body)).await;
	assert_eq!(status, StatusCode::NOT_FOUND);
	let back_till = paired["token"].as_str().unwrap();

	// The owner revokes the device, and its token stops working.
	let devices = posd.ok(Method::GET, "/api/v1/owner/devices", Some(&store.owner_token), None).await;
	let back = devices["devices"].as_array().unwrap().iter()
		.find(|d| d["name"] == "Back till").unwrap()["id"].as_i64().unwrap();
	let (status, _) = posd.call(Method::DELETE, &format!("/api/v1/owner/devices/{}", back),
		Some(&store.owner_token), None).await;
	assert_eq!(status, StatusCode::NO_CONTENT);
	let (status, _) = posd.call(Method::GET, "/api/v1/terminal", Some(back_till), None).await;
	assert_eq!(status, StatusCode::UNAUTHORIZED);
	posd.ok(Method::GET, "/api/v1/terminal", Some(&store.till), None).await;

	// A terminal cannot use owner endpoints.
	let (status, _) = posd.call(Method::POST, "/api/v1/owner/invites", Some(&store.till), None).await;
	assert_eq!(status, StatusCode::UNAUTHORIZED);

	// The terminal shows the store's brand, and the owner can change it. The
	// install manifest of a pairing link is in the brand too.
	let terminal = posd.ok(Method::GET, "/api/v1/terminal", Some(&store.till), None).await;
	assert_eq!(terminal["store"]["brand"], ALPHA);
	assert_eq!(terminal["store"]["currency_decimals"], 2);
	posd.ok(Method::PATCH, "/api/v1/owner/store", Some(&store.owner_token),
		Some(json!({ "brand": BETA }))).await;
	let terminal = posd.ok(Method::GET, "/api/v1/terminal", Some(&store.till), None).await;
	assert_eq!(terminal["store"]["brand"], BETA);
	let invite = posd.ok(Method::POST, "/api/v1/owner/invites", Some(&store.owner_token), None).await;
	let manifest = posd.ok(Method::GET,
		&format!("/pair/{}/manifest.webmanifest", invite["code"].as_str().unwrap()), None, None).await;
	assert_eq!(manifest["name"], "Beta Till");
	assert_eq!(manifest["theme_color"], "#123456");
	assert_eq!(manifest["icons"][0]["src"], format!("/brands/{}/icon-192.png", BETA));
	let (status, _) = posd.call(Method::PATCH, "/api/v1/owner/store", Some(&store.owner_token),
		Some(json!({ "brand": "neon" }))).await;
	assert!(status.is_client_error(), "an unknown brand must be refused");

	// Lightning address. The store is now a Beta store, so its address is on
	// Beta's domain only.
	let beta = posd.brand_url(BETA);
	let alpha = posd.brand_url(ALPHA);
	let beta_host = beta.strip_prefix("http://").unwrap();
	let updated = posd.ok(Method::PATCH, "/api/v1/owner/store", Some(&store.owner_token),
		Some(json!({ "lightning_address_name": "cafe" }))).await;
	assert_eq!(updated["lightning_address"], format!("cafe@{}", beta_host));
	let (status, pay_request) = get(&format!("{}/.well-known/lnurlp/cafe", beta)).await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(pay_request["tag"], "payRequest");
	let metadata: Value = serde_json::from_str(pay_request["metadata"].as_str().unwrap()).unwrap();
	assert_eq!(metadata[1], json!(["text/identifier", format!("cafe@{}", beta_host)]));
	let (status, _) = get(&format!("{}/.well-known/lnurlp/cafe", alpha)).await;
	assert_eq!(status, StatusCode::NOT_FOUND, "the name is not on Alpha's domain");

	// The same name on the other domain is another store's, given at
	// registration. A store can also register without an address.
	let (status, other, _) = register(&ctx, &store, "owner2", ALPHA, "cafe").await;
	assert_eq!(status, StatusCode::OK, "{}", other);
	let alpha_host = alpha.strip_prefix("http://").unwrap();
	assert_eq!(other["store"]["lightning_address"], format!("cafe@{}", alpha_host));
	let (_, alpha_request) = get(&format!("{}/.well-known/lnurlp/cafe", alpha)).await;
	assert_eq!(alpha_request["metadata"].as_str().unwrap().contains("Payment to Owner2 Cafe"), true);
	let (status, _, _) = register(&ctx, &store, "owner3", ALPHA, "cafe").await;
	assert_eq!(status, StatusCode::CONFLICT, "a name is taken once per domain");
	let (status, plain, _) = register(&ctx, &store, "owner4", ALPHA, "").await;
	assert_eq!(status, StatusCode::OK, "{}", plain);
	assert_eq!(plain["store"]["lightning_address"], Value::Null);

	// A payment to the Beta address reaches the first store.
	let callback = pay_request["callback"].as_str().unwrap();
	assert!(callback.starts_with(&beta), "the callback is on the brand's domain: {}", callback);
	let (status, invoice) = get(&format!("{}?amount=21000000", callback)).await;
	assert_eq!(status, StatusCode::OK, "{}", invoice);
	store.lightning.external.pay_bolt11(invoice["pr"].as_str().unwrap()).await;

	let deadline = tokio::time::Instant::now() + SALE_TIMEOUT;
	loop {
		let sales = posd.ok(Method::GET, "/api/v1/owner/sales", Some(&store.owner_token), None).await;
		let sale = &sales["sales"][0];
		assert_eq!(sale["device_name"], Value::Null);
		assert_eq!(sale["amount_sat"], 21_000);
		if sale["state"] == "paid" {
			break;
		}
		assert!(tokio::time::Instant::now() < deadline, "Lightning address sale not paid");
		tokio::time::sleep(Duration::from_millis(200)).await;
	}

	// Asking for invoices needs no credentials, so a store has a cap on the
	// unpaid ones open at once. The paid one above does not count.
	for i in 0..MAX_OPEN_PER_STORE {
		let (status, body) = get(&format!("{}?amount=10000000", callback)).await;
		assert_eq!(status, StatusCode::OK, "invoice {}: {}", i, body);
	}
	let (status, body) = get(&format!("{}?amount=10000000", callback)).await;
	assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
	// LNURL errors have the LUD-06 body.
	assert_eq!(body["status"], "ERROR", "{}", body);
	assert!(body["reason"].as_str().is_some_and(|r| !r.is_empty()));
	// Other stores and the terminals are not held up.
	new_sale(&store, json!({ "amount_sat": 1_000 })).await;
}

/// A brand's operator reserves the names of users moving from another
/// service, then gives each one to the store of the user it verified.
/// Nobody else can take a reserved name on that domain.
#[tokio::test]
async fn pos_reserved_lightning_names() {
	let ctx = TestContext::new("pos/pos_reserved_lightning_names").await;
	let store = store_setup(&ctx).await;
	let posd = &store.posd;
	let beta_op = admin_token(BETA);
	let alpha_op = admin_token(ALPHA);
	let reserve = json!({ "names": ["legacy", "oldcafe"] });

	let reserved = posd.ok(Method::POST, "/api/v1/admin/reserved-names", Some(&beta_op), Some(reserve.clone())).await;
	assert_eq!(reserved["reserved"], 2);
	let again = posd.ok(Method::POST, "/api/v1/admin/reserved-names", Some(&beta_op), Some(reserve.clone())).await;
	assert_eq!(again["reserved"], 0);
	let (status, _) = posd.call(Method::POST, "/api/v1/admin/reserved-names", Some(&store.till), Some(reserve)).await;
	assert_eq!(status, StatusCode::UNAUTHORIZED, "a terminal is not an operator");

	// Registration cannot take a reserved name on that domain, but can on
	// another brand's.
	let (status, _, _) = register(&ctx, &store, "squatter", BETA, "legacy").await;
	assert_eq!(status, StatusCode::CONFLICT);
	let (status, _, _) = register(&ctx, &store, "alphacafe", ALPHA, "legacy").await;
	assert_eq!(status, StatusCode::OK);

	// The operator gives the name to the verified user's store.
	let (status, _, migrated) = register(&ctx, &store, "migrated", BETA, "").await;
	assert_eq!(status, StatusCode::OK);
	let assign = json!({ "ark_address": migrated, "name": "legacy" });
	let (status, _) = posd.call(Method::POST, "/api/v1/admin/lightning-addresses", Some(&alpha_op), Some(assign.clone())).await;
	assert_eq!(status, StatusCode::NOT_FOUND, "an operator assigns on its own domain only");
	let assigned = posd.ok(Method::POST, "/api/v1/admin/lightning-addresses", Some(&beta_op), Some(assign)).await;
	let beta = posd.brand_url(BETA);
	assert_eq!(assigned["lightning_address"], format!("legacy@{}", beta.strip_prefix("http://").unwrap()));
	let (status, pay_request) = get(&format!("{}/.well-known/lnurlp/legacy", beta)).await;
	assert_eq!(status, StatusCode::OK);
	assert!(pay_request["metadata"].as_str().unwrap().contains("Payment to Migrated Cafe"));

	// A name a store has cannot go to another store.
	let (_, _, other) = register(&ctx, &store, "other", BETA, "").await;
	let (status, _) = posd.call(Method::POST, "/api/v1/admin/lightning-addresses", Some(&beta_op),
		Some(json!({ "ark_address": other, "name": "legacy" }))).await;
	assert_eq!(status, StatusCode::CONFLICT);

	// An owner cannot rename into a reserved name, nor move a name onto a
	// domain where it is reserved.
	let (status, _) = posd.call(Method::PATCH, "/api/v1/owner/store", Some(&store.owner_token),
		Some(json!({ "lightning_address_name": "oldcafe" }))).await;
	assert_eq!(status, StatusCode::OK, "oldcafe is only reserved on Beta's domain");
	let (status, _) = posd.call(Method::PATCH, "/api/v1/owner/store", Some(&store.owner_token),
		Some(json!({ "brand": BETA }))).await;
	assert_eq!(status, StatusCode::CONFLICT);
	let (status, _) = posd.call(Method::PATCH, "/api/v1/owner/store", Some(&store.owner_token),
		Some(json!({ "currency": "USD" }))).await;
	assert_eq!(status, StatusCode::OK, "keeping a name is always allowed");
}

/// A GET to a full URL, so a test can pick the domain.
async fn get(url: &str) -> (StatusCode, Value) {
	let resp = reqwest::get(url).await.expect("posd request failed");
	(resp.status(), resp.json().await.unwrap_or(Value::Null))
}

/// Registers a store for a new wallet called `wallet`, named after it, and
/// returns posd's answer and the store's Ark address.
async fn register(
	ctx: &TestContext,
	store: &Store,
	wallet: &str,
	brand_key: &str,
	lightning_address_name: &str,
) -> (StatusCode, Value, String) {
	let (status, body, address, _) = register_wallet(ctx, store, wallet, brand_key, lightning_address_name).await;
	(status, body, address)
}

/// As [register], and returns the owner's wallet too.
async fn register_wallet(
	ctx: &TestContext,
	store: &Store,
	wallet: &str,
	brand_key: &str,
	lightning_address_name: &str,
) -> (StatusCode, Value, String, Wallet) {
	let owner = ctx.bark_sdk(wallet, &store.srv).create().await;
	let address = owner.new_address().await.unwrap();
	let mut name = format!("{} Cafe", wallet);
	name[..1].make_ascii_uppercase();
	let registration = Registration {
		name,
		ark_address: address.to_string(),
		onchain_descriptor: owner_descriptor(),
		currency: "CRC".into(),
		brand: brand(brand_key),
		lightning_address_name: lightning_address_name.into(),
		timestamp: chrono::Utc::now().timestamp() as u64,
	};
	let signature = owner.sign_message(registration.message().as_bytes(), &address)
		.await.unwrap().expect("the owner holds the key");
	let (status, body) = store.posd.call(Method::POST, "/api/v1/stores", None, Some(json!({
		"registration": registration, "signature": signature.to_string(),
	}))).await;
	(status, body, address.to_string(), owner)
}

/// Registers another store on the same posd, of a new wallet with the same
/// on-chain descriptor, and pairs a terminal. Returns its terminal token.
async fn second_store(ctx: &TestContext, store: &Store, wallet: &str) -> String {
	let (status, body, address, owner) = register_wallet(ctx, store, wallet, ALPHA, "").await;
	assert_eq!(status, StatusCode::OK, "{}", body);
	// The owner's wallet may write the address in upper case, as in a QR
	// code: posd stores and signs it in lower case.
	let upper = address.to_uppercase();
	let address = ark::Address::from_str(&address).unwrap();
	let challenge = store.posd.challenge(&upper).await;
	let sig = owner.sign_message(challenge.as_bytes(), &address).await.unwrap().unwrap();
	let owner_token = store.posd.login(&upper, &challenge, sig.to_string()).await;
	store.posd.pair(&owner_token, "Second till").await
}

/// A sale's life on the terminal: the long-poll, cancel, expiry, a currency
/// the terminal no longer shows, and the lists and exports of past sales.
#[tokio::test]
async fn pos_sale_lifecycle() {
	let ctx = TestContext::new("pos/pos_sale_lifecycle").await;
	let store = store_setup_with(&ctx, |cfg| cfg.sales.expiry_secs = 6).await;
	let posd = &store.posd;
	let till = Some(store.till.as_str());

	// A long-poll returns as soon as the sale changes: here, a cancel.
	let sale = new_sale(&store, json!({ "amount_sat": 5_000, "memo": "first" })).await;
	let first = sale["id"].as_str().unwrap().to_string();
	let started = Instant::now();
	let waiting = {
		let url = format!("{}/api/v1/sales/{}?wait=open", posd.url, first);
		let till = store.till.clone();
		tokio::spawn(async move {
			reqwest::Client::new().get(url).bearer_auth(till).send().await.unwrap()
				.json::<Value>().await.unwrap()
		})
	};
	tokio::time::sleep(Duration::from_millis(500)).await;
	let canceled = posd.ok(Method::POST, &format!("/api/v1/sales/{}/cancel", first), till, None).await;
	assert_eq!(canceled["state"], "canceled");
	assert_eq!(waiting.await.unwrap()["state"], "canceled");
	assert!(started.elapsed() < Duration::from_secs(10), "the long-poll waited past the change");
	let (status, _) = posd.call(Method::POST, &format!("/api/v1/sales/{}/cancel", first), till, None).await;
	assert_eq!(status, StatusCode::CONFLICT, "a canceled sale cannot be canceled again");

	// An unpaid sale expires.
	let sale = new_sale(&store, json!({ "amount_sat": 6_000 })).await;
	let expired = posd.wait_sale_state(&store.till, sale["id"].as_str().unwrap(), "expired", SALE_TIMEOUT).await;
	assert_eq!(expired["received_sat"], 0);
	let (status, _) = posd.call(Method::POST,
		&format!("/api/v1/sales/{}/cancel", expired["id"].as_str().unwrap()), till, None).await;
	assert_eq!(status, StatusCode::CONFLICT);

	// The terminal shows the store's currency; after the owner changes it, a
	// sale in the old one is refused rather than charged in the new one.
	posd.ok(Method::PATCH, "/api/v1/owner/store", Some(&store.owner_token),
		Some(json!({ "currency": "USD" }))).await;
	let (status, body) = posd.call(Method::POST, "/api/v1/sales", till,
		Some(json!({ "fiat_amount": "1000", "currency": "CRC" }))).await;
	assert_eq!(status, StatusCode::CONFLICT, "{}", body);
	let usd = new_sale(&store, json!({ "fiat_amount": "1000.00", "currency": "USD" })).await;
	assert_eq!(usd["currency"], "USD");
	let (status, _) = posd.call(Method::POST, "/api/v1/sales", till,
		Some(json!({ "fiat_amount": "10.001", "currency": "USD" }))).await;
	assert_eq!(status, StatusCode::BAD_REQUEST, "more decimals than the currency has");
	let (status, _) = posd.call(Method::POST, "/api/v1/sales", till,
		Some(json!({ "amount_sat": u64::MAX }))).await;
	assert_eq!(status, StatusCode::BAD_REQUEST, "more than 21 million bitcoin");

	// Pages of sales, newest first.
	let all = posd.ok(Method::GET, "/api/v1/sales", till, None).await;
	let ids = all["sales"].as_array().unwrap().iter()
		.map(|s| s["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
	assert_eq!(ids.len(), 3);
	assert_eq!(ids[2], first);
	let page = posd.ok(Method::GET, "/api/v1/sales?limit=2", till, None).await;
	let page = page["sales"].as_array().unwrap();
	assert_eq!(page.len(), 2);
	let rest = posd.ok(Method::GET, &format!("/api/v1/sales?limit=2&before={}", ids[1]), till, None).await;
	assert_eq!(rest["sales"].as_array().unwrap().iter().map(|s| s["id"].clone()).collect::<Vec<_>>(), [json!(first)]);

	// The CSV export takes a date range in UTC days or exact times.
	async fn csv(store: &Store, query: &str) -> Vec<String> {
		let resp = reqwest::Client::new().get(format!("{}/api/v1/sales.csv{}", store.posd.url, query))
			.bearer_auth(&store.till).send().await.unwrap();
		assert_eq!(resp.status(), StatusCode::OK);
		resp.text().await.unwrap().lines().skip(1).map(String::from).collect()
	}
	let today = chrono::Utc::now().date_naive();
	assert_eq!(csv(&store, "").await.len(), 3);
	assert_eq!(csv(&store, &format!("?from={}", today)).await.len(), 3);
	assert_eq!(csv(&store, &format!("?to={}", today.pred_opt().unwrap())).await.len(), 0);
	assert_eq!(csv(&store, &format!("?from={}", today.succ_opt().unwrap())).await.len(), 0);
	let rows = csv(&store, &format!("?from={}&to={}", today, today)).await;
	assert!(rows[0].starts_with(&first) && rows[0].contains(",canceled,"), "oldest first: {:?}", rows);
	let (status, _) = posd.call(Method::GET, "/api/v1/sales.csv?from=yesterday", till, None).await;
	assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Two stores that registered the same on-chain descriptor never share an
/// address, and with Esplora a sale skips an address the wallet already
/// received to: one payment cannot pay two sales.
#[tokio::test]
async fn pos_onchain_addresses_are_unused() {
	let ctx = TestContext::new("pos/pos_onchain_addresses_are_unused").await;
	let store = store_setup(&ctx).await;
	let second = second_store(&ctx, &store, "second").await;
	let desc = StoreDescriptor::parse(&owner_descriptor()).unwrap();
	let address = |i: u32| desc.address(i, Network::Regtest).unwrap().to_string();

	let a = new_sale(&store, json!({ "amount_sat": 60_000 })).await;
	assert_eq!(a["onchain_address"], address(0));
	let b = store.posd.ok(Method::POST, "/api/v1/sales", Some(&second), Some(json!({ "amount_sat": 60_000 }))).await;
	assert_eq!(b["onchain_address"], address(1), "the second store skips the first store's address");

	// The wallet itself received to the next two addresses. A sale skips
	// them, once the chain source has seen the payments.
	ctx.bitcoind().fund_addr(&address(2), sat(10_000)).await;
	ctx.bitcoind().fund_addr(&address(3), sat(10_000)).await;
	ctx.generate_blocks(1).await;
	let deadline = Instant::now() + SALE_TIMEOUT;
	loop {
		let c = new_sale(&store, json!({ "amount_sat": 60_000 })).await;
		if c["onchain_address"] == address(4) {
			break;
		}
		// The chain source may not have the block yet: the sale took one of
		// the funded addresses, which a later sale skips.
		assert!(Instant::now() < deadline, "the sales took addresses with history");
		tokio::time::sleep(Duration::from_millis(500)).await;
	}
}

/// posd serves each page in the brand of its domain, and a brand's files
/// from its asset directory, with the neutral brand's for the rest.
#[tokio::test]
async fn pos_brand_pages() {
	let ctx = TestContext::new("pos/pos_brand_pages").await;
	let store = store_setup(&ctx).await;
	let beta = store.posd.brand_url(BETA);
	let alpha = store.posd.brand_url(ALPHA);

	let manifest = reqwest::get(format!("{}/manifest.webmanifest", beta)).await.unwrap()
		.json::<Value>().await.unwrap();
	assert_eq!(manifest["name"], "Beta Till");
	assert_eq!(manifest["start_url"], "/");
	let manifest = reqwest::get(format!("{}/manifest.webmanifest", alpha)).await.unwrap()
		.json::<Value>().await.unwrap();
	assert_eq!(manifest["name"], "Alpha POS");

	let css = reqwest::get(format!("{}/brands/{}/brand.css", beta, BETA)).await.unwrap();
	assert_eq!(css.headers()["content-type"], "text/css; charset=utf-8");
	assert_eq!(css.text().await.unwrap(), BETA_CSS);
	// Alpha has no files of its own: the neutral brand's stand in.
	let mark = reqwest::get(format!("{}/brands/{}/mark.svg", alpha, ALPHA)).await.unwrap();
	assert_eq!(mark.status(), StatusCode::OK);
	assert!(mark.headers().contains_key("content-security-policy"));
	for path in ["/brands/beta/notes.txt", "/brands/beta/..%2Fbrand.css", "/brands/BETA/brand.css"] {
		let resp = reqwest::get(format!("{}{}", beta, path)).await.unwrap();
		assert_ne!(resp.status(), StatusCode::OK, "{}", path);
	}

	// The page, when posd was built with the terminal.
	let page = reqwest::get(format!("{}/", beta)).await.unwrap();
	if page.status() == StatusCode::OK {
		let html = page.text().await.unwrap();
		assert!(html.contains(&format!("data-brand=\"{}\"", BETA)), "{}", html);
		assert!(html.contains(&format!("/brands/{}/brand.css", BETA)));
		assert!(html.contains("\"app_name\":\"Beta Till\""));
	}
}

/// A manual demo, not run by default. It starts a full stack, prints a
/// pairing link for the terminal and pays every sale the terminal creates
/// from an Ark wallet. Run it with
/// `cargo nextest run -p ark-testing --test pos --run-ignored only pos_demo`
/// and open the link in a browser. POS_DEMO_SECS sets how long it runs.
#[tokio::test]
#[ignore]
async fn pos_demo() {
	let ctx = TestContext::new("pos/pos_demo").await;
	let store = store_setup(&ctx).await;
	let payer = ctx.bark_sdk("payer", &store.srv).boarded(sat(2_000_000)).create().await;
	let invite = store.posd.ok(Method::POST, "/api/v1/owner/invites", Some(&store.owner_token), None).await;
	println!("POS_DEMO_PAIRING_URL {}", invite["url"].as_str().unwrap());
	eprintln!("POS_DEMO_PAIRING_URL {}", invite["url"].as_str().unwrap());

	let secs = std::env::var("POS_DEMO_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(900);
	let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
	let mut paid = std::collections::HashSet::new();
	while tokio::time::Instant::now() < deadline {
		let sales = store.posd.ok(Method::GET, "/api/v1/owner/sales", Some(&store.owner_token), None).await;
		for sale in sales["sales"].as_array().unwrap() {
			let id = sale["id"].as_str().unwrap().to_string();
			if sale["state"] == "open" && sale["device_name"] != "Front till" && paid.insert(id.clone()) {
				// Give the screen time to show the QR code.
				tokio::time::sleep(Duration::from_secs(4)).await;
				let kind = pay(&payer, sale["payment_request"].as_str().unwrap()).await;
				eprintln!("POS_DEMO paid sale {} by {}", id, kind);
			}
		}
		tokio::time::sleep(Duration::from_millis(500)).await;
	}
}
