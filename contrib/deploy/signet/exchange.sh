#!/usr/bin/env bash
# Runs the BULL app's dummy exchange on a regtest deployment, so a regtest
# build of the app can be pointed at it: an in-memory stand-in for the
# Bull Bitcoin exchange JSON-RPC API, from the app's own regtest stack
# (tools/regtest/exchange.py). Fiat balances and quotes are fake; buy and
# sell orders settle with real regtest bitcoin from the wallet `bull-regtest`.
# This chain has no Liquid, so Liquid orders fail. Run after setup.sh; publish
# port 8088 with cloudflare.sh (the <name>-exchange host).
#
#   ./contrib/deploy/signet/exchange.sh
set -euo pipefail

# Pinned: the dummy's methods follow the app's exchange models at this commit.
REV=2185a48ddd52c6f94e5d4f398d20ee5aa02fb568
SHA256=96ae21bb0edad97ccb0492b8ed01a91a25504b58340259ea9e9e925c43866eba
URL="https://raw.githubusercontent.com/ethicnology-agent/bullbitcoin-mobile/$REV/tools/regtest/exchange.py"
DIR=/opt/scuttle/exchange
FUND_BTC="${FUND_BTC:-100}"
eval "$(sudo cat /etc/scuttle/deploy.env)"   # NETWORK, RPC_PORT
[ "$NETWORK" = regtest ] || { echo "the dummy exchange settles with regtest coins only" >&2; exit 1; }
cli() { sudo -u scuttle /opt/scuttle/bin/chain-cli "$@"; }

echo "== exchange.py @ ${REV:0:10}"
sudo install -d -m 755 "$DIR"
curl -fsSL "$URL" -o /tmp/exchange.py
echo "$SHA256  /tmp/exchange.py" | sha256sum -c --quiet
sudo install -m 644 /tmp/exchange.py "$DIR/exchange.py"
rm -f /tmp/exchange.py

# The dummy talks to bitcoind with Nigiri's public development credentials.
# Instead of adding those to this node, the wrapper gives it this
# deployment's own, read from the root-and-scuttle-only secrets file.
sudo tee "$DIR/run.py" >/dev/null <<'EOF'
import os
import signal
import sys
from http.server import HTTPServer

sys.path.insert(0, os.path.dirname(__file__))
import exchange  # noqa: E402

exchange.Node.__init__.__defaults__ = ("bull-regtest", "scuttle", os.environ["BITCOIND_RPC_PASS"])
exchange.Handler.exchange = exchange.Exchange()
server = HTTPServer(("127.0.0.1", 8088), exchange.Handler)
signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
try:
    server.serve_forever()
finally:
    server.server_close()
EOF

echo "== wallet bull-regtest"
cli -named createwallet wallet_name=bull-regtest >/dev/null 2>&1 || cli loadwallet bull-regtest >/dev/null 2>&1 || true
balance=$(cli -rpcwallet=bull-regtest getbalance)
if awk "BEGIN{exit !($balance < $FUND_BTC / 2)}"; then
	cli -rpcwallet=miner sendtoaddress "$(cli -rpcwallet=bull-regtest getnewaddress)" "$FUND_BTC" >/dev/null
	sudo -u scuttle /opt/scuttle/bin/mine 1 >/dev/null
fi
echo "bull-regtest holds $(cli -rpcwallet=bull-regtest getbalance) BTC"

echo "== service"
sudo tee /etc/systemd/system/scuttle-exchange.service >/dev/null <<EOF
[Unit]
Description=scuttle BULL dummy exchange (regtest)
After=scuttle-bitcoind.service
Requires=scuttle-bitcoind.service
[Service]
User=scuttle
EnvironmentFile=/etc/scuttle/secrets.env
Environment=BITCOIND_URL=http://127.0.0.1:$RPC_PORT
# No Liquid on this chain: Liquid orders fail instead of reaching a node.
Environment=ELEMENTSD_URL=http://127.0.0.1:9
ExecStart=/usr/bin/python3 $DIR/run.py
Restart=on-failure
[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable scuttle-exchange.service
sudo systemctl restart scuttle-exchange.service
for i in $(seq 1 20); do curl -sf http://127.0.0.1:8088/health && echo && break; sleep 1; done
