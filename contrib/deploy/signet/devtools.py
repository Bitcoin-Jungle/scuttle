#!/usr/bin/env python3
"""Test helpers for the private chain: a faucet, a miner and store wallets.

Served at https://<host>/dev/ and, behind cloudflare.sh, at the root of
https://<name>-faucet.<domain>. GET / shows a web page; the API is:

  GET  status         {network, height, faucet_btc, lightning_sat, limits}
  POST faucet         {"address": "bcrt1...", "btc": 0.01}
      sends coins on-chain and mines a block to confirm them.
  POST lightning      {"invoice": "lnbcrt...", "sat": 1000}
      pays a Lightning invoice from the outside node (cln-external). "sat"
      is needed only for an invoice without an amount.
  POST mine           {"blocks": 1}
  POST store-wallet   {"name": "My Store"}
      creates a wallet on the VM and returns its public descriptor for store
      registration. The VM holds the wallet's spending keys.

The coins have no value, but the faucet is shared, so every endpoint has a
cap per request and a rate limit per client, and a global one behind it.
Signet mines at most a few blocks per request, because signet retargets like
mainnet: many quick blocks raise the difficulty for everyone. Regtest never
retargets, so a few requests reach a VTXO's expiry.
"""

import json
import re
import subprocess
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

CHAIN_CLI = "/opt/scuttle/bin/chain-cli"
MINER = "/opt/scuttle/bin/mine"
LISTEN = ("127.0.0.1", 3090)


def deployed_network() -> str:
	with open("/etc/scuttle/deploy.env") as f:
		for line in f:
			if line.startswith("NETWORK="):
				return line.split("=", 1)[1].strip()
	return "signet"


NETWORK = deployed_network()
REGTEST = NETWORK == "regtest"
# The bech32 prefix of this network's invoices.
INVOICE_CURRENCY = {"regtest": "bcrt", "signet": "tbs"}[NETWORK]

MAX_FAUCET_BTC = 1.0 if REGTEST else 0.1
MAX_LIGHTNING_SAT = 1_000_000 if REGTEST else 100_000
LIGHTNING_WAIT_SECS = 15
# Regtest mines about 15 blocks a second. A request must finish within
# Cloudflare's 100-second origin timeout.
MAX_BLOCKS = 1000 if REGTEST else 6
# The largest request body read. Every endpoint takes a few small fields.
MAX_BODY = 4096
# Sliding windows, (max calls, seconds): per client, then for everyone.
LIMITS = {
	"faucet": ((10, 600), (60, 600)),
	"lightning": ((10, 600), (60, 600)),
	"mine": ((10, 600), (40, 600)),
	"store-wallet": ((10, 600), (40, 600)),
}

_calls: dict[tuple[str, str], list[float]] = {}
_lock = threading.Lock()
# One mining run at a time: bitcoind mines no faster in parallel.
_mine_lock = threading.Lock()


def take(key: tuple[str, str], limit: int, window: int, now: float) -> bool:
	calls = [t for t in _calls.get(key, []) if now - t < window]
	_calls[key] = calls
	if len(calls) >= limit:
		return False
	calls.append(now)
	return True


def allowed(endpoint: str, client: str) -> bool:
	(per_client, per_window), (total, total_window) = LIMITS[endpoint]
	now = time.time()
	with _lock:
		ok = len([t for t in _calls.get((endpoint, client), []) if now - t < per_window]) < per_client \
			and len([t for t in _calls.get((endpoint, "*"), []) if now - t < total_window]) < total
		if ok:
			take((endpoint, client), per_client, per_window, now)
			take((endpoint, "*"), total, total_window, now)
		return ok


def run(args: list[str], timeout: int = 120) -> str:
	out = subprocess.run(args, capture_output=True, text=True, timeout=timeout)
	if out.returncode != 0:
		raise RuntimeError((out.stderr or out.stdout).strip() or f"{args[0]} failed")
	return out.stdout.strip()


def cli(*args: str) -> str:
	return run([CHAIN_CLI, *args])


# lightning-cli of the outside node. "-N none" keeps progress notifications,
# such as xpay's, out of the JSON output.
LNCLI = ["docker", "exec", "scuttle-cln-external", "lightning-cli",
	f"--network={NETWORK}", "--lightning-dir=/data/.lightning", "-N", "none", "-k"]


def lncli(*args: str) -> dict:
	return json.loads(run([*LNCLI, *args]))


def mine(blocks: int) -> int:
	with _mine_lock:
		out = run([MINER, str(blocks)], timeout=300)
	return int(out.splitlines()[-1])


def status(_body: dict) -> dict:
	channels = lncli("listpeerchannels")["channels"]
	return {
		"network": NETWORK,
		"height": int(cli("getblockcount")),
		"faucet_btc": json.loads(cli("-rpcwallet=miner", "getbalances"))["mine"]["trusted"],
		"lightning_sat": sum(c.get("spendable_msat", 0) for c in channels
			if c["state"] == "CHANNELD_NORMAL") // 1000,
		"limits": {"faucet_btc": MAX_FAUCET_BTC, "lightning_sat": MAX_LIGHTNING_SAT, "blocks": MAX_BLOCKS},
	}


def faucet(body: dict) -> dict:
	address = str(body.get("address", "")).strip()
	btc = float(body.get("btc", 0.01))
	if not 0 < btc <= MAX_FAUCET_BTC:
		raise ValueError(f"btc must be between 0 and {MAX_FAUCET_BTC}")
	if not json.loads(cli("validateaddress", address)).get("isvalid"):
		raise ValueError(f"not a valid {NETWORK} address")
	txid = cli("-rpcwallet=miner", "sendtoaddress", address, f"{btc:.8f}")
	# Confirm it, so the coins can be boarded right away.
	return {"txid": txid, "btc": btc, "height": mine(1)}


def lightning(body: dict) -> dict:
	invoice = str(body.get("invoice", "")).strip().removeprefix("lightning:").lower()
	if not re.fullmatch(r"ln[a-z0-9]+", invoice):
		raise ValueError("not a Lightning invoice")
	try:
		decoded = lncli("decode", f"string={invoice}")
	except RuntimeError:
		raise ValueError("not a valid Lightning invoice")
	if not decoded.get("valid") or decoded.get("type") != "bolt11 invoice":
		raise ValueError("not a valid BOLT 11 invoice")
	if decoded.get("currency") != INVOICE_CURRENCY:
		raise ValueError(f"not a {NETWORK} invoice")
	if decoded["created_at"] + decoded["expiry"] < time.time():
		raise ValueError("the invoice has expired")
	args = [f"invstring={invoice}"]
	if "amount_msat" in decoded:
		sat = decoded["amount_msat"] // 1000
	else:
		sat = int(body.get("sat") or 0)
		args.append(f"amount_msat={sat * 1000}")
	if not 0 < sat <= MAX_LIGHTNING_SAT:
		raise ValueError(f"the amount must be between 1 and {MAX_LIGHTNING_SAT} sat")
	# An Ark wallet settles a Lightning payment only when it is online to
	# claim it, so a payment can stay in flight for a long time. Wait a
	# little, then report it as pending and let it finish on its own.
	proc = subprocess.Popen([*LNCLI, "xpay", *args],
		stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
	result = {"payment_hash": decoded["payment_hash"], "sat": sat}
	try:
		out, err = proc.communicate(timeout=LIGHTNING_WAIT_SECS)
	except subprocess.TimeoutExpired:
		threading.Thread(target=proc.communicate, daemon=True).start()
		return {**result, "status": "pending",
			"note": "the receiving wallet must be online to claim it, before many blocks pass"}
	if proc.returncode != 0:
		raise RuntimeError((err or out).strip() or "the payment failed")
	paid = json.loads(out)
	return {**result, "status": "paid", "fee_sat": (paid["amount_sent_msat"] - paid["amount_msat"]) // 1000}


def mine_blocks(body: dict) -> dict:
	blocks = int(body.get("blocks", 1))
	if not 1 <= blocks <= MAX_BLOCKS:
		raise ValueError(f"blocks must be between 1 and {MAX_BLOCKS}")
	return {"height": mine(blocks)}


def store_wallet(body: dict) -> dict:
	name = str(body.get("name", "")).strip()
	slug = re.sub(r"[^a-z0-9]", "", name.lower())[:40]
	if not slug:
		raise ValueError("name must contain letters or digits")
	wallet = f"store-{slug}"
	try:
		cli("-named", "createwallet", f"wallet_name={wallet}")
	except RuntimeError as e:
		# The wallet is on disk from before: load it, unless it is loaded.
		if "already exists" not in str(e):
			raise
		try:
			cli("loadwallet", wallet)
		except RuntimeError as e:
			if "already loaded" not in str(e):
				raise
	descs = json.loads(cli(f"-rpcwallet={wallet}", "listdescriptors"))["descriptors"]
	desc = next(d["desc"] for d in descs if d["desc"].startswith("tr(") and not d["internal"])
	return {"wallet": wallet, "onchain_descriptor": desc}


POST = {"faucet": faucet, "lightning": lightning, "mine": mine_blocks, "store-wallet": store_wallet}

PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Scuttle faucet</title>
<style>
:root { --bg: #f7f6f2; --fg: #1d1c1a; --muted: #6b6860; --card: #fff; --line: #e2dfd6; --accent: #e8730c; --bad: #b3261e; }
@media (prefers-color-scheme: dark) { :root { --bg: #121211; --fg: #ecebe6; --muted: #9a978e; --card: #1c1b19; --line: #2e2d29; --accent: #f59a3c; --bad: #f2b8b5; } }
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--fg); font: 16px/1.5 system-ui, sans-serif; }
main { max-width: 640px; margin: 0 auto; padding: 32px 16px 64px; }
h1 { font-size: 1.6rem; margin: 0 0 4px; } h2 { font-size: 1.05rem; margin: 0 0 12px; }
.sub { color: var(--muted); margin: 0 0 24px; }
.stats { display: grid; grid-template-columns: repeat(3, 1fr); gap: 8px; margin-bottom: 24px; }
.stat { background: var(--card); border: 1px solid var(--line); border-radius: 10px; padding: 12px; }
.stat b { display: block; font-size: 1.15rem; font-variant-numeric: tabular-nums; }
.stat span { color: var(--muted); font-size: .8rem; }
form { background: var(--card); border: 1px solid var(--line); border-radius: 12px; padding: 16px; margin-bottom: 16px; }
label { display: block; font-size: .85rem; color: var(--muted); margin: 8px 0 4px; }
input { width: 100%; padding: 10px; border: 1px solid var(--line); border-radius: 8px; background: var(--bg); color: var(--fg); font: inherit; font-family: ui-monospace, monospace; font-size: .9rem; }
.row { display: flex; gap: 8px; align-items: end; } .row > div { flex: 1; }
button { margin-top: 12px; padding: 10px 16px; border: 0; border-radius: 8px; background: var(--accent); color: #fff; font: inherit; font-weight: 600; cursor: pointer; }
button:disabled { opacity: .5; cursor: wait; }
.out { margin-top: 10px; font-family: ui-monospace, monospace; font-size: .8rem; word-break: break-all; white-space: pre-wrap; }
.out.bad { color: var(--bad); }
code { font-size: .85rem; }
.hint { color: var(--muted); font-size: .85rem; }
</style></head><body><main>
<h1>Scuttle faucet</h1>
<p class="sub">A private <b id="net">…</b> chain for testing Ark wallets and the point of sale. The coins have no value.</p>
<div class="stats">
<div class="stat"><b id="height">…</b><span>block height</span></div>
<div class="stat"><b id="btc">…</b><span>faucet BTC</span></div>
<div class="stat"><b id="ln">…</b><span>Lightning sat</span></div>
</div>

<form data-ep="faucet">
<h2>On-chain coins</h2>
<label for="address">Address</label><input id="address" name="address" required autocomplete="off" spellcheck="false">
<label for="btc-amt">BTC</label><input id="btc-amt" name="btc" type="number" step="0.00000001" min="0.00000001" value="0.01" required>
<button>Send coins</button><div class="out"></div>
<p class="hint">A block confirms them right away, so a wallet can board them.</p>
</form>

<form data-ep="lightning">
<h2>Pay a Lightning invoice</h2>
<label for="invoice">Invoice</label><input id="invoice" name="invoice" required autocomplete="off" spellcheck="false">
<label for="sat">Sats, for an invoice without an amount</label><input id="sat" name="sat" type="number" min="1">
<button>Pay invoice</button><div class="out"></div>
<p class="hint">Paid by a Lightning node outside the Ark, as an outside payer would. An Ark wallet must be open to claim the payment: keep it open until it arrives, and don't mine many blocks meanwhile.</p>
</form>

<form data-ep="mine">
<h2>Mine blocks</h2>
<label for="blocks">Blocks</label><input id="blocks" name="blocks" type="number" min="1" value="1" required>
<button>Mine</button><div class="out"></div>
<p class="hint" id="mine-hint"></p>
</form>

<p class="hint">API: <code>POST /faucet</code>, <code>/lightning</code>, <code>/mine</code>, <code>/store-wallet</code> with JSON; <code>GET /status</code>. Ark server <code id="ark"></code>, Esplora <code id="esplora"></code>.</p>
</main>
<script>
const base = location.pathname.startsWith("/dev") ? "/dev/" : "/";
const fmt = n => Number(n).toLocaleString("en-US", {maximumFractionDigits: 8});
async function call(ep, body) {
	const r = await fetch(base + ep, body ? {method: "POST", headers: {"Content-Type": "application/json"}, body: JSON.stringify(body)} : {});
	const j = await r.json().catch(() => ({error: "HTTP " + r.status}));
	if (!r.ok) throw new Error(j.error || "HTTP " + r.status);
	return j;
}
async function refresh() {
	try {
		const s = await call("status");
		net.textContent = s.network; height.textContent = fmt(s.height);
		btc.textContent = fmt(Math.floor(s.faucet_btc)); ln.textContent = fmt(s.lightning_sat);
		document.querySelector("#btc-amt").max = s.limits.faucet_btc;
		document.querySelector("#blocks").max = s.limits.blocks;
		document.querySelector("#mine-hint").textContent = s.network === "regtest"
			? "A block comes every minute on its own. Up to " + fmt(s.limits.blocks) + " blocks a request: a VTXO expires after 4,320."
			: "A block comes every 10 minutes on its own. Up to " + s.limits.blocks + " blocks a request.";
	} catch (e) {}
}
const host = location.hostname.replace(/-faucet\\./, ".");
const [stem, ...rest] = host.split(".");
if (location.hostname.includes("-faucet.")) {
	ark.textContent = "https://" + stem + "-ark." + rest.join(".");
	esplora.textContent = "https://" + stem + "-esplora." + rest.join(".");
}
for (const form of document.querySelectorAll("form")) {
	form.addEventListener("submit", async ev => {
		ev.preventDefault();
		const out = form.querySelector(".out"), btn = form.querySelector("button");
		const body = {};
		for (const [k, v] of new FormData(form)) if (v !== "") body[k] = form.elements[k].type === "number" ? Number(v) : v;
		btn.disabled = true; out.className = "out"; out.textContent = "…";
		try { out.textContent = JSON.stringify(await call(form.dataset.ep, body), null, 1); refresh(); }
		catch (e) { out.className = "out bad"; out.textContent = e.message; }
		finally { btn.disabled = false; }
	});
}
refresh(); setInterval(refresh, 15000);
</script></body></html>
"""


class Handler(BaseHTTPRequestHandler):
	# A client that stops sending is dropped instead of holding a thread.
	timeout = 10

	def reply(self, status: int, body, content_type: str = "application/json") -> None:
		data = (body if isinstance(body, str) else json.dumps(body)).encode()
		self.send_response(status)
		self.send_header("Content-Type", content_type)
		self.send_header("Content-Length", str(len(data)))
		self.send_header("Cache-Control", "no-store")
		self.end_headers()
		self.wfile.write(data)

	def endpoint(self) -> str:
		# Served at /dev/ on the posd host and at / on the faucet host.
		path = self.path.split("?")[0].rstrip("/")
		return path.removeprefix("/dev").lstrip("/")

	def client(self) -> str:
		# Cloudflare sets CF-Connecting-IP; Tailscale Funnel sets none, so
		# its clients share the global limit only.
		return self.headers.get("CF-Connecting-IP") or "unknown"

	def do_GET(self) -> None:
		endpoint = self.endpoint()
		if endpoint == "":
			return self.reply(200, PAGE, "text/html; charset=utf-8")
		if endpoint == "status":
			try:
				return self.reply(200, status({}))
			except Exception as e:  # noqa: BLE001
				return self.reply(502, {"error": str(e)})
		if endpoint == "api":
			return self.reply(200, __doc__, "text/plain; charset=utf-8")
		self.reply(404, {"error": "no such endpoint"})

	def do_POST(self) -> None:
		endpoint = self.endpoint()
		if endpoint not in POST:
			return self.reply(404, {"error": "no such endpoint"})
		if not allowed(endpoint, self.client()):
			return self.reply(429, {"error": "rate limited, try again in a few minutes"})
		try:
			length = int(self.headers.get("Content-Length") or 0)
			if length < 0 or length > MAX_BODY:
				return self.reply(413, {"error": f"the body is over {MAX_BODY} bytes"})
			body = json.loads(self.rfile.read(length) or b"{}")
			if not isinstance(body, dict):
				raise ValueError("the body must be a JSON object")
			self.reply(200, POST[endpoint](body))
		except (ValueError, TypeError, KeyError) as e:
			self.reply(400, {"error": str(e)})
		except Exception as e:  # noqa: BLE001 - report bitcoind and CLN errors to the caller
			self.reply(502, {"error": str(e)})

	def log_message(self, fmt: str, *args) -> None:
		print(f"{self.client()} {fmt % args}", flush=True)


if __name__ == "__main__":
	ThreadingHTTPServer(LISTEN, Handler).serve_forever()
