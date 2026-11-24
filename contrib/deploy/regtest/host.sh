#!/usr/bin/env bash
# Runs the shared regtest test network from BULL's regtest stack, the same
# one BULL's developers run locally: Bitcoin and Liquid (Nigiri), Electrum for
# both, scuttle (captaind, watchmand, barkd, posd), Boltz, RecoverBull,
# Payjoin mailrooms, crash-cache and the dummy exchange. Then publishes it:
# HTTP through the Cloudflare Tunnel (cloudflare.sh set it up), Electrum
# through Tailscale Funnel with TLS, since a Cloudflare Tunnel carries no raw
# TCP. RPCs and the stack's public development credentials stay on loopback.
#
#   STACK_REV=<commit of Bitcoin-Jungle/bullbitcoin-mobile scuttle-regtest> \
#     ./contrib/deploy/regtest/host.sh
#
# It replaces the systemd services setup.sh made on this VM: they are stopped
# and disabled, and their data stays in /var/lib/scuttle. Rerun to update the
# stack to another revision. Needs podman, podman-compose, passt, nftables, aardvark-dns, python3-yaml,
# Nigiri 0.5.17 in ~/.local/bin, rustup with the toolchains build-services.py
# names, and cloudflared and tailscale already set up.
set -euo pipefail

STACK_REPO="${STACK_REPO:-https://github.com/Bitcoin-Jungle/bullbitcoin-mobile.git}"
STACK_REV="${STACK_REV:?the bullbitcoin-mobile commit to run}"
DOMAIN="${DOMAIN:-vibecode.casa}"
NAME="${NAME:-scuttle-regtest}"
DIR="$HOME/bull-regtest-stack"
PREFIX=/opt/scuttle
HERE="$(cd "$(dirname "$0")" && pwd)"
USER_ID=$(id -u)
export BULL_REGTEST_POSD_URL="https://$NAME.$DOMAIN"
export BULL_REGTEST_POSD_BULL_URL="https://$NAME-bull.$DOMAIN"
export BULL_REGTEST_ESPLORA_URL="https://$NAME-esplora.$DOMAIN"
export BULL_REGTEST_LIQUID_ESPLORA_URL="https://$NAME-liquid.$DOMAIN"
log() { echo "== $*"; }

log "retire the systemd deployment"
for unit in miner.timer liquid-miner.timer posd barkd watchmand captaind cln-internal cln-external \
	electrs electrs-liquid liquid exchange webwallet-web webwallet-api webwallet-barkd bitcoind; do
	sudo systemctl disable --now "scuttle-$unit" 2>/dev/null || true
done

log "stack $STACK_REV"
sudo loginctl enable-linger "$USER"
if [ ! -d "$DIR/.git" ]; then
	git clone -q --filter=blob:none --no-checkout "$STACK_REPO" "$DIR"
	git -C "$DIR" sparse-checkout set --no-cone /tools/regtest/
fi
git -C "$DIR" fetch -q origin "$STACK_REV"
git -C "$DIR" checkout -q --detach "$STACK_REV"
# Rootless netavark runs nft, which Debian keeps in /usr/sbin.
export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH:/usr/sbin:/sbin"
# shellcheck disable=SC1091
. "$HOME/.cargo/env"
python3 "$DIR/tools/regtest/build-services.py"
python3 "$DIR/tools/regtest/stack.py" up

log "Lightning capacity for the faucet"
# The stack's payer (cln-peer) gets half of a 0.1 BTC channel. A shared
# faucet needs more: open 1 BTC from cln-peer to the Ark server's node.
stack() { python3 "$DIR/tools/regtest/stack.py" "$@"; }
peer_out=$(stack ln --peer listpeerchannels | jq '[.channels[] | select(.state == "CHANNELD_NORMAL") | .spendable_msat] | add // 0')
if [ "$peer_out" -lt 50000000000 ]; then
	stack faucet "$(stack ln --peer newaddr | jq -r '.bech32 // .p2tr')" 1.1 >/dev/null
	stack mine 6 >/dev/null
	# Open only once CLN has seen the funds confirm.
	for _ in $(seq 1 60); do
		[ "$(stack ln --peer listfunds | jq '[.outputs[] | select(.status == "confirmed") | .amount_msat] | add // 0')" -ge 100000000000 ] && break
		sleep 2
	done
	cln_id=$(stack ln getinfo | jq -r .id)
	stack ln --peer fundchannel "$cln_id" 100000000 >/dev/null
	stack mine 6 >/dev/null
fi

log "helpers for the faucet and the miners"
# The faucet (devtools.py) and the miner timers call these, as on a systemd
# deployment. Rootless podman: they run as $USER.
sudo install -d -m 755 "$PREFIX/bin"
sudo tee "$PREFIX/bin/chain-cli" >/dev/null <<'EOF'
#!/bin/sh
exec podman exec bull-regtest-bitcoin bitcoin-cli "$@"
EOF
sudo tee "$PREFIX/bin/liquid-cli" >/dev/null <<'EOF'
#!/bin/sh
exec podman exec bull-regtest-liquid elements-cli -datadir=config "$@"
EOF
sudo tee "$PREFIX/bin/mine" >/dev/null <<'EOF'
#!/bin/sh
# Mines N blocks (default 1) to the faucet's wallet and prints the height.
cli() { /opt/scuttle/bin/chain-cli "$@"; }
cli loadwallet miner >/dev/null 2>&1 || cli createwallet miner >/dev/null 2>&1 || true
cli -rpcwallet=miner generatetoaddress "${1:-1}" "$(cli -rpcwallet=miner getnewaddress)" >/dev/null
cli getblockcount
EOF
sudo tee "$PREFIX/bin/liquid-mine" >/dev/null <<'EOF'
#!/bin/sh
# Mines N Liquid blocks (default 1) to the faucet's wallet.
cli() { /opt/scuttle/bin/liquid-cli "$@"; }
cli loadwallet miner >/dev/null 2>&1 || cli createwallet miner >/dev/null 2>&1 || true
cli -rpcwallet=miner generatetoaddress "${1:-1}" "$(cli -rpcwallet=miner getnewaddress)" >/dev/null
cli getblockcount
EOF
sudo chmod 755 "$PREFIX/bin/chain-cli" "$PREFIX/bin/liquid-cli" "$PREFIX/bin/mine" "$PREFIX/bin/liquid-mine"
# Faucet funds: 101 blocks for spendable bitcoin, and Liquid's from the
# stack's funded wallet.
if [ "$("$PREFIX/bin/chain-cli" -rpcwallet=miner getbalance 2>/dev/null || echo 0)" = "0.00000000" ] \
	|| ! "$PREFIX/bin/chain-cli" -rpcwallet=miner getbalance >/dev/null 2>&1; then
	"$PREFIX/bin/mine" 101 >/dev/null
fi
"$PREFIX/bin/liquid-mine" 1 >/dev/null
lbal=$("$PREFIX/bin/liquid-cli" -rpcwallet=miner getbalance | jq -r .bitcoin)
if awk "BEGIN{exit !($lbal < 100)}"; then
	"$PREFIX/bin/liquid-cli" -rpcwallet=bull-regtest sendtoaddress \
		"$("$PREFIX/bin/liquid-cli" -rpcwallet=miner getnewaddress)" 1 >/dev/null || true
fi

env_line="Environment=XDG_RUNTIME_DIR=/run/user/$USER_ID PATH=$HOME/.local/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
for chain in "" liquid-; do
	sudo tee "/etc/systemd/system/scuttle-${chain}miner.service" >/dev/null <<EOF
[Unit]
Description=scuttle regtest ${chain:+Liquid }block
[Service]
Type=oneshot
User=$USER
$env_line
ExecStart=$PREFIX/bin/${chain}mine 1
EOF
	sudo tee "/etc/systemd/system/scuttle-${chain}miner.timer" >/dev/null <<EOF
[Unit]
Description=scuttle regtest ${chain:+Liquid }block every minute
[Timer]
OnBootSec=3min
OnUnitActiveSec=1min
[Install]
WantedBy=timers.target
EOF
done

log "faucet"
sudo install -m 755 "$HERE/../signet/devtools.py" "$PREFIX/bin/devtools"
sudo tee /etc/systemd/system/scuttle-devtools.service >/dev/null <<EOF
[Unit]
Description=scuttle test faucet and miner
After=bull-regtest-stack.service
[Service]
User=$USER
$env_line
Environment=NETWORK=regtest
Environment="DEVTOOLS_LNCLI=podman exec bull-regtest-cln-peer lightning-cli --regtest -N none -k"
ExecStart=/usr/bin/python3 $PREFIX/bin/devtools
Restart=on-failure
[Install]
WantedBy=multi-user.target
EOF

log "start on boot"
sudo tee /etc/systemd/system/bull-regtest-stack.service >/dev/null <<EOF
[Unit]
Description=BULL regtest stack with scuttle
Wants=network-online.target
After=network-online.target
[Service]
Type=oneshot
RemainAfterExit=yes
User=$USER
$env_line
Environment=BULL_REGTEST_POSD_URL=$BULL_REGTEST_POSD_URL BULL_REGTEST_POSD_BULL_URL=$BULL_REGTEST_POSD_BULL_URL
Environment=BULL_REGTEST_ESPLORA_URL=$BULL_REGTEST_ESPLORA_URL BULL_REGTEST_LIQUID_ESPLORA_URL=$BULL_REGTEST_LIQUID_ESPLORA_URL
ExecStart=/usr/bin/python3 $DIR/tools/regtest/stack.py up
ExecStop=/usr/bin/python3 $DIR/tools/regtest/stack.py down
TimeoutStartSec=1800
[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable bull-regtest-stack.service scuttle-devtools.service scuttle-miner.timer scuttle-liquid-miner.timer >/dev/null
sudo systemctl restart scuttle-devtools.service
sudo systemctl start scuttle-miner.timer scuttle-liquid-miner.timer

log "Cloudflare Tunnel"
tunnel=$(sudo awk '/^tunnel:/ {print $2}' /etc/cloudflared/config.yml)
sudo tee /etc/cloudflared/config.yml >/dev/null <<EOF
tunnel: $tunnel
credentials-file: /etc/cloudflared/$tunnel.json
ingress:
  - hostname: $NAME.$DOMAIN
    path: ^/dev(/.*)?$
    service: http://127.0.0.1:3090
  - hostname: $NAME.$DOMAIN
    service: http://127.0.0.1:3080
  - hostname: $NAME-bull.$DOMAIN
    service: http://127.0.0.1:3080
  - hostname: $NAME-ark.$DOMAIN
    service: https://127.0.0.1:8445
    originRequest:
      http2Origin: true
      noTLSVerify: true
  - hostname: $NAME-esplora.$DOMAIN
    service: http://127.0.0.1:30000
  - hostname: $NAME-liquid.$DOMAIN
    service: http://127.0.0.1:30001
  - hostname: $NAME-faucet.$DOMAIN
    service: http://127.0.0.1:3090
  - hostname: $NAME-wallet.$DOMAIN
    service: http://127.0.0.1:8448
  - hostname: $NAME-console.$DOMAIN
    service: http://127.0.0.1:8447
  - hostname: $NAME-exchange.$DOMAIN
    service: http://127.0.0.1:8088
  # Boltz's WebSocket is its sidecar's own server; production serves it at
  # /v2/ws on the API host, so route that path there.
  - hostname: $NAME-boltz.$DOMAIN
    path: ^/v2/ws$
    service: http://127.0.0.1:9004
  - hostname: $NAME-boltz.$DOMAIN
    service: http://127.0.0.1:9001
  - hostname: $NAME-crash.$DOMAIN
    service: http://127.0.0.1:8097
  - hostname: $NAME-explorer.$DOMAIN
    service: http://127.0.0.1:5000
  - hostname: $NAME-liquid-explorer.$DOMAIN
    service: http://127.0.0.1:5001
  # Electrum over WebSocket (Nigiri's websocat bridge), for browser wallets.
  - hostname: $NAME-electrum-ws.$DOMAIN
    service: http://127.0.0.1:50003
  # The two Payjoin mailrooms (directory and OHTTP relay each, relaying for
  # each other). They serve TLS with the stack's own certificate.
  - hostname: $NAME-payjoin-a.$DOMAIN
    service: https://127.0.0.1:8040
    originRequest:
      noTLSVerify: true
  - hostname: $NAME-payjoin-b.$DOMAIN
    service: https://127.0.0.1:8041
    originRequest:
      noTLSVerify: true
  - service: http_status:404
EOF
sudo cloudflared tunnel --config /etc/cloudflared/config.yml ingress validate >/dev/null
sudo systemctl restart scuttle-cloudflared

log "Tor onion service: RecoverBull"
# The app reaches RecoverBull over Tor, as in production.
sudo DEBIAN_FRONTEND=noninteractive apt-get install -yq tor >/dev/null
printf '%s\n' "HiddenServiceDir /var/lib/tor/recoverbull/" \
	"HiddenServicePort 80 127.0.0.1:8098" "HiddenServicePort 3000 127.0.0.1:8098" \
	| sudo tee /etc/tor/torrc.d-recoverbull >/dev/null
grep -q '^%include /etc/tor/torrc.d-recoverbull' /etc/tor/torrc \
	|| echo '%include /etc/tor/torrc.d-recoverbull' | sudo tee -a /etc/tor/torrc >/dev/null
sudo systemctl enable tor >/dev/null 2>&1 || true
sudo systemctl restart tor
for _ in $(seq 1 30); do sudo test -s /var/lib/tor/recoverbull/hostname && break; sleep 1; done

log "Tailscale Funnel: Electrum with TLS"
sudo tailscale funnel --bg --tls-terminated-tcp=443 tcp://127.0.0.1:50000 >/dev/null
sudo tailscale funnel --bg --tls-terminated-tcp=8443 tcp://127.0.0.1:50001 >/dev/null

host=$(tailscale status --self --json | jq -r '.Self.DNSName | rtrimstr(".")')
echo
echo "Route any new names from a machine with a cloudflared login for $DOMAIN:"
for h in boltz crash explorer liquid-explorer electrum-ws payjoin-a payjoin-b console; do echo "  cloudflared tunnel route dns $tunnel $NAME-$h.$DOMAIN"; done
echo "posd       $BULL_REGTEST_POSD_URL  (Bull: $BULL_REGTEST_POSD_BULL_URL)"
echo "Ark        https://$NAME-ark.$DOMAIN"
echo "Esplora    https://$NAME-esplora.$DOMAIN  (Liquid: https://$NAME-liquid.$DOMAIN)"
echo "Electrum   ssl://$host:443  (Liquid: ssl://$host:8443)"
echo "Boltz      https://$NAME-boltz.$DOMAIN   crash-cache https://$NAME-crash.$DOMAIN"
echo "RecoverBull http://$(sudo cat /var/lib/tor/recoverbull/hostname) (Tor)"
echo "crash-cache DSN (local): $(cat "${BULL_REGTEST_STATE:-$HOME/.local/share/bull-regtest}/crash-cache-dsn" 2>/dev/null)"
