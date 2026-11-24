#!/usr/bin/env bash
# Runs a web wallet on this deployment: bark-web, Second's reference UI, with
# a barkd of its own (this repository's, not the release bark-web bundles, so
# it speaks this Ark server's protocol), with its password login on. Run after
# setup.sh; publish port 8447 with cloudflare.sh (the <name>-wallet host).
#
#   BARK_WEB=<checkout of bark-web> ./contrib/deploy/signet/webwallet.sh
#
# Bitcoin Jungle's fork (github.com/Bitcoin-Jungle/bark-web, branch scuttle)
# adds store management against this posd; upstream bark-web ignores those
# settings.
#
# The login password is in /etc/scuttle/webwallet/ui_password. The wallet
# itself is created in the browser on first visit. It is one shared wallet:
# UI_AUTH=false drops the login for open testing, and then anyone can use,
# reveal or reset it.
set -euo pipefail

BARK_WEB="${BARK_WEB:?a bark-web checkout}"
UI_AUTH="${UI_AUTH:-true}"
# The Esplora wallets read the chain from: setup.sh's electrs, or the BULL
# stack's on a regtest host (contrib/deploy/regtest/host.sh: 127.0.0.1:30000).
CHAIN_SOURCE="${CHAIN_SOURCE:-http://127.0.0.1:3002}"
DATA=/var/lib/scuttle/webwallet
ETC=/etc/scuttle/webwallet
eval "$(sudo cat /etc/scuttle/deploy.env)"   # NETWORK

echo "== barkd"
sudo install -d -o scuttle -g scuttle -m 700 "$DATA"
sudo tee /etc/systemd/system/scuttle-webwallet-barkd.service >/dev/null <<EOF
[Unit]
Description=scuttle barkd for the web wallet
After=scuttle-captaind.service
[Service]
User=scuttle
Environment=RUST_LOG=info
ExecStart=/opt/scuttle/bin/barkd --datadir $DATA --port 4000
Restart=on-failure
RestartSec=5
[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable --now scuttle-webwallet-barkd.service
for i in $(seq 1 30); do sudo test -f "$DATA/auth_token" && break; sleep 1; done

echo "== login secrets"
sudo install -d -o root -g scuttle -m 750 "$ETC"
for f in ui_password ui_session_secret; do
	if ! sudo test -f "$ETC/$f"; then
		openssl rand -hex 16 | sudo tee "$ETC/$f" >/dev/null
		sudo chown root:scuttle "$ETC/$f"
		sudo chmod 640 "$ETC/$f"
	fi
done

echo "== bark-web images"
# Its nginx expects the compose service names; here everything is on the
# host network, on loopback ports.
sed -e 's/listen 8080;/listen 127.0.0.1:8447;/' \
	-e 's#http://barkd:4000#http://127.0.0.1:4000#' \
	-e 's#http://api:4001#http://127.0.0.1:4001#' \
	"$BARK_WEB/docker/nginx.conf" > "$BARK_WEB/docker/nginx.scuttle.conf"
sed 's#docker/nginx.conf#docker/nginx.scuttle.conf#' "$BARK_WEB/docker/bark-web.Dockerfile" \
	> "$BARK_WEB/docker/bark-web.scuttle.Dockerfile"
sudo docker build -q -t scuttle-bark-web -f "$BARK_WEB/docker/bark-web.scuttle.Dockerfile" "$BARK_WEB"
sudo docker build -q -t scuttle-bark-web-api -f "$BARK_WEB/docker/bark-web-api.Dockerfile" "$BARK_WEB"

echo "== services"
uid=$(id -u scuttle); gid=$(id -g scuttle)
sudo tee /etc/systemd/system/scuttle-webwallet-api.service >/dev/null <<EOF
[Unit]
Description=scuttle bark-web API
Requires=docker.service scuttle-webwallet-barkd.service
After=docker.service scuttle-webwallet-barkd.service
[Service]
ExecStartPre=-/usr/bin/docker rm -f scuttle-webwallet-api
ExecStart=/usr/bin/docker run --rm --name scuttle-webwallet-api --net=host --user $uid:$gid \\
	--mount type=bind,source=$DATA,target=/wallet-data/.bark,readonly \\
	--mount type=bind,source=$ETC,target=/auth,readonly \\
	-e HOST=127.0.0.1 -e PORT=4001 -e BARKD_URL=http://127.0.0.1:4000 -e WALLET_DIR=/wallet-data/.bark \\
	-e WALLET_DATA_PATH=$DATA/ -e BARK_NETWORK=$NETWORK \\
	-e ARK_SERVER=http://127.0.0.1:3535 -e CHAIN_SOURCE=$CHAIN_SOURCE \\
	-e UI_AUTH=$UI_AUTH -e UI_PASSWORD_FILE=/auth/ui_password -e UI_SESSION_SECRET_FILE=/auth/ui_session_secret \\
	-e POSD_URL=http://127.0.0.1:3080 -e "POS_BRANDS=bitcoinjungle:Bitcoin Jungle,bullbitcoin:Bull Bitcoin" \\
	-e POS_CURRENCIES=$(sudo grep -oP '^currencies = \{\K[^}]*' /etc/scuttle/posd.toml | grep -oP '[A-Z]{3}(?= =)' | paste -sd,) \\
	scuttle-bark-web-api
ExecStop=/usr/bin/docker stop scuttle-webwallet-api
Restart=on-failure
[Install]
WantedBy=multi-user.target
EOF
sudo tee /etc/systemd/system/scuttle-webwallet-web.service >/dev/null <<EOF
[Unit]
Description=scuttle bark-web UI
Requires=docker.service scuttle-webwallet-api.service
After=docker.service scuttle-webwallet-api.service
[Service]
ExecStartPre=-/usr/bin/docker rm -f scuttle-webwallet-web
ExecStart=/usr/bin/docker run --rm --name scuttle-webwallet-web --net=host scuttle-bark-web
ExecStop=/usr/bin/docker stop scuttle-webwallet-web
Restart=on-failure
[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable scuttle-webwallet-api.service scuttle-webwallet-web.service
sudo systemctl restart scuttle-webwallet-api.service scuttle-webwallet-web.service
for i in $(seq 1 30); do curl -sf http://127.0.0.1:8447/api/config >/dev/null 2>&1 && break; sleep 1; done
systemctl is-active scuttle-webwallet-barkd scuttle-webwallet-api scuttle-webwallet-web
echo "web wallet on 127.0.0.1:8447; login password: sudo cat $ETC/ui_password"
