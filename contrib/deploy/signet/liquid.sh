#!/usr/bin/env bash
# Adds a Liquid regtest chain to a regtest deployment, for wallets that hold
# L-BTC next to bitcoin and Ark (the BULL app): elementsd, its Esplora and
# Electrum index (electrs, Liquid build), a miner timer, and the helper
# /opt/scuttle/bin/liquid-cli. No peg-ins and no Boltz: a standalone chain.
#
# Images and chain parameters are those of Nigiri, which the BULL app's own
# regtest stack runs, so the policy asset (L-BTC) is the same:
# 5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225.
#
# Listens on loopback only: RPC 18884, Esplora 3003, Electrum 50002. Publish
# Esplora with cloudflare.sh (the <name>-liquid host) and Electrum with
# Tailscale Funnel (TLS), since Cloudflare Tunnel carries no raw TCP:
#
#   sudo tailscale funnel --bg --tls-terminated-tcp=8443 tcp://127.0.0.1:50002
#
# Run after setup.sh:  ./contrib/deploy/signet/liquid.sh
set -euo pipefail

ELEMENTS_IMAGE=ghcr.io/vulpemventures/elements@sha256:47a1bcc8acd1b390a737f1c4004194feac2e60015e9feabc9e66f3e057847f1a
ELECTRS_IMAGE=ghcr.io/vulpemventures/electrs-liquid@sha256:d801d1ab07e7a3379462b9d09cf5bb4fada1ced67b001a1a2de31e027c2f6055
POLICY_ASSET=5ac9f65c0efcc4775e0baec4ec03abdde22473cd3cf33c0419ca290e0751b225
DATA=/var/lib/scuttle
ETC=/etc/scuttle
PREFIX=/opt/scuttle
LIQUID_RPC_PORT=18884
eval "$(sudo cat "$ETC/deploy.env")"   # NETWORK
[ "$NETWORK" = regtest ] || { echo "Liquid runs beside regtest only" >&2; exit 1; }
log() { echo "== $*"; }
uid=$(id -u scuttle)
gid=$(id -g scuttle)

log "secrets"
if ! sudo grep -q '^LIQUID_RPC_PASS=' "$ETC/secrets.env"; then
	echo "LIQUID_RPC_PASS=$(openssl rand -hex 24)" | sudo tee -a "$ETC/secrets.env" >/dev/null
fi
eval "$(sudo cat "$ETC/secrets.env")"

log "images"
sudo docker pull -q "$ELEMENTS_IMAGE" >/dev/null
sudo docker pull -q "$ELECTRS_IMAGE" >/dev/null

log "elementsd"
sudo install -d -o scuttle -g scuttle -m 750 "$DATA/liquid" "$DATA/electrs-liquid"
# Nigiri's chain parameters (cmd/nigiri/resources/elements.conf, v0.5.17),
# the relay policy of BULL's stack, and this deployment's own credentials.
sudo tee "$DATA/liquid/elements.conf" >/dev/null <<EOF
chain=liquidregtest
txindex=1
server=1
listen=0

[liquidregtest]
rpcport=$LIQUID_RPC_PORT
rpcbind=127.0.0.1
rpcallowip=127.0.0.1
rpcuser=scuttle
rpcpassword=$LIQUID_RPC_PASS
pchmessagestart=12345678
blockfilterindex=1
validatepegin=0
initialfreecoins=2100000000000000
fallbackfee=0.000001
minrelaytxfee=0.00000100
acceptdiscountct=1
con_dyna_deploy_start=0
con_dyna_deploy_signal=1
con_nminerconfirmationwindow=1
con_nrulechangeactivationthreshold=1
con_taproot_signal_start=0
EOF
sudo chown scuttle:scuttle "$DATA/liquid/elements.conf"
sudo chmod 600 "$DATA/liquid/elements.conf"

sudo tee /etc/systemd/system/scuttle-liquid.service >/dev/null <<EOF
[Unit]
Description=scuttle Liquid regtest elementsd
Requires=docker.service
After=docker.service
[Service]
ExecStartPre=-/usr/bin/docker rm -f scuttle-liquid
ExecStart=/usr/bin/docker run --rm --name scuttle-liquid --net=host --user $uid:$gid \\
	--mount type=bind,source=$DATA/liquid,target=/data \\
	--entrypoint elementsd $ELEMENTS_IMAGE -datadir=/data -conf=/data/elements.conf
ExecStop=/usr/bin/docker stop -t 60 scuttle-liquid
Restart=on-failure
[Install]
WantedBy=multi-user.target
EOF

# elements-cli inside the container, as the scuttle user, against this node.
sudo tee "$PREFIX/bin/liquid-cli" >/dev/null <<EOF
#!/bin/sh
exec docker exec scuttle-liquid elements-cli -datadir=/data -conf=/data/elements.conf "\$@"
EOF
sudo chmod 755 "$PREFIX/bin/liquid-cli"
lcli() { sudo -u scuttle "$PREFIX/bin/liquid-cli" "$@"; }

sudo systemctl daemon-reload
sudo systemctl enable scuttle-liquid.service >/dev/null
sudo systemctl restart scuttle-liquid.service
for _ in $(seq 1 60); do lcli getblockcount >/dev/null 2>&1 && break; sleep 1; done
asset=$(lcli getsidechaininfo | jq -r .pegged_asset)
[ "$asset" = "$POLICY_ASSET" ] || { echo "policy asset is $asset, expected $POLICY_ASSET" >&2; exit 1; }

log "wallet miner"
# The chain starts with 21M L-BTC in anyone-can-spend outputs: claim them
# into the miner wallet, which funds the faucet.
lcli -named createwallet wallet_name=miner >/dev/null 2>&1 || lcli loadwallet miner >/dev/null 2>&1 || true
if [ "$(lcli getblockcount)" -eq 0 ]; then
	lcli -rpcwallet=miner rescanblockchain >/dev/null
	lcli -rpcwallet=miner -named sendtoaddress address="$(lcli -rpcwallet=miner getnewaddress)" amount=20000000 subtractfeefromamount=true >/dev/null
	lcli -rpcwallet=miner generatetoaddress 101 "$(lcli -rpcwallet=miner getnewaddress)" >/dev/null
fi
echo "miner holds $(lcli -rpcwallet=miner getbalance | jq -r .bitcoin) L-BTC at height $(lcli getblockcount)"

log "miner timer"
# One block a minute, like the bitcoin miner.
sudo tee "$PREFIX/bin/liquid-mine" >/dev/null <<EOF
#!/bin/sh
# Mines N Liquid blocks (default 1) to the miner wallet.
cli() { $PREFIX/bin/liquid-cli "\$@"; }
cli -rpcwallet=miner generatetoaddress "\${1:-1}" "\$(cli -rpcwallet=miner getnewaddress)"
EOF
sudo chmod 755 "$PREFIX/bin/liquid-mine"
sudo tee /etc/systemd/system/scuttle-liquid-miner.service >/dev/null <<EOF
[Unit]
Description=scuttle Liquid regtest block
Requires=scuttle-liquid.service
After=scuttle-liquid.service
[Service]
Type=oneshot
User=scuttle
ExecStart=$PREFIX/bin/liquid-mine 1
EOF
sudo tee /etc/systemd/system/scuttle-liquid-miner.timer >/dev/null <<EOF
[Unit]
Description=scuttle Liquid regtest block every minute
[Timer]
OnBootSec=2min
OnUnitActiveSec=1min
[Install]
WantedBy=timers.target
EOF

log "electrs (Liquid)"
sudo tee /etc/systemd/system/scuttle-electrs-liquid.service >/dev/null <<EOF
[Unit]
Description=scuttle Liquid Esplora and Electrum index
Requires=docker.service scuttle-liquid.service
After=docker.service scuttle-liquid.service
[Service]
EnvironmentFile=$ETC/secrets.env
ExecStartPre=-/usr/bin/docker rm -f scuttle-electrs-liquid
ExecStart=/bin/sh -c 'exec /usr/bin/docker run --rm --name scuttle-electrs-liquid --net=host --user $uid:$gid \\
	--mount type=bind,source=$DATA/electrs-liquid,target=/db \\
	--mount type=bind,source=$DATA/liquid,target=/daemon,readonly \\
	--entrypoint /build/electrs $ELECTRS_IMAGE \\
	--parent-network regtest --network liquidregtest --jsonrpc-import \\
	--daemon-dir /daemon --daemon-rpc-addr 127.0.0.1:$LIQUID_RPC_PORT \\
	--cookie "scuttle:\$LIQUID_RPC_PASS" --db-dir /db \\
	--http-addr 127.0.0.1:3003 --electrum-rpc-addr 127.0.0.1:50002 \\
	--monitoring-addr 127.0.0.1:4225 --cors "*"'
ExecStop=/usr/bin/docker stop -t 30 scuttle-electrs-liquid
Restart=on-failure
[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable scuttle-electrs-liquid.service scuttle-liquid-miner.timer >/dev/null
sudo systemctl restart scuttle-electrs-liquid.service
sudo systemctl start scuttle-liquid-miner.timer
for _ in $(seq 1 60); do
	[ "$(curl -sf http://127.0.0.1:3003/blocks/tip/height || true)" = "$(lcli getblockcount)" ] && break
	sleep 2
done
echo "Liquid Esplora at height $(curl -sf http://127.0.0.1:3003/blocks/tip/height)"
