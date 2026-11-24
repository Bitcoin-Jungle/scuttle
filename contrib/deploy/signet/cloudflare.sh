#!/usr/bin/env bash
# Publishes a deployment on the internet through a Cloudflare Tunnel, as an
# alternative to public.sh (Tailscale Funnel). Run on the VM after setup.sh.
#
#   CREDENTIALS=<tunnel id>.json DOMAIN=vibecode.casa NAME=scuttle-regtest \
#     ./contrib/deploy/signet/cloudflare.sh
#
# CREDENTIALS is the file `cloudflared tunnel create` wrote. The tunnel's
# DNS records are created separately with `cloudflared tunnel route dns`.
#
#   https://$NAME.$DOMAIN/           posd for Bitcoin Jungle stores
#   https://$NAME.$DOMAIN/dev/       test faucet, mining and store wallets
#   https://$NAME-faucet.$DOMAIN/    the same, at the root
#   https://$NAME-wallet.$DOMAIN/    bark-web, when webwallet.sh has set it up
#   https://$NAME-exchange.$DOMAIN/  BULL's dummy exchange, when exchange.sh has set it up
#   https://$NAME-bull.$DOMAIN/      posd for Bull Bitcoin stores
#   https://$NAME-ark.$DOMAIN        captaind gRPC, the Ark server for wallets
#   https://$NAME-esplora.$DOMAIN    Esplora (electrs)
#
# The names are one level below the domain, because Cloudflare's Universal
# SSL certificate covers only *.$DOMAIN.
#
# cloudflared proxies HTTP/2 only to an origin with TLS, and captaind serves
# gRPC without TLS. So the Ark server goes through a local nginx that
# terminates TLS with a self-signed certificate. The zone's gRPC setting
# (Network → gRPC) must be on for Cloudflare to pass gRPC, and cloudflared
# must use its http2 transport: over QUIC, the default, the tunnel drops
# HTTP trailers, so gRPC clients never see grpc-status.
set -euo pipefail

CREDENTIALS="${CREDENTIALS:?the tunnel credentials file}"
DOMAIN="${DOMAIN:?the Cloudflare zone, e.g. vibecode.casa}"
NAME="${NAME:-scuttle-$(sudo sed -n 's/^NETWORK=//p' /etc/scuttle/deploy.env)}"
POS="$NAME.$DOMAIN"
BULL="$NAME-bull.$DOMAIN"
ARK="$NAME-ark.$DOMAIN"
ESPLORA="$NAME-esplora.$DOMAIN"
FAUCET="$NAME-faucet.$DOMAIN"
WALLET="$NAME-wallet.$DOMAIN"
EXCHANGE="$NAME-exchange.$DOMAIN"
TUNNEL=$(jq -r .TunnelID "$CREDENTIALS")
TLS=/etc/scuttle/tls

echo "== cloudflared"
if ! command -v cloudflared >/dev/null; then
	sudo install -d -m 755 /usr/share/keyrings
	curl -fsSL https://pkg.cloudflare.com/cloudflare-main.gpg | sudo tee /usr/share/keyrings/cloudflare-main.gpg >/dev/null
	echo "deb [signed-by=/usr/share/keyrings/cloudflare-main.gpg] https://pkg.cloudflare.com/cloudflared any main" \
		| sudo tee /etc/apt/sources.list.d/cloudflared.list >/dev/null
	sudo apt-get update -q >/dev/null
	sudo DEBIAN_FRONTEND=noninteractive apt-get install -yq cloudflared >/dev/null
fi

echo "== nginx, TLS in front of captaind's gRPC"
sudo DEBIAN_FRONTEND=noninteractive apt-get install -yq nginx >/dev/null
sudo install -d -m 750 -o root -g www-data "$TLS"
if ! sudo test -f "$TLS/origin.crt"; then
	sudo openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 3650 \
		-subj "/CN=$ARK" -keyout "$TLS/origin.key" -out "$TLS/origin.crt" 2>/dev/null
	sudo chgrp www-data "$TLS/origin.key" "$TLS/origin.crt"
	sudo chmod 640 "$TLS/origin.key" "$TLS/origin.crt"
fi
sudo rm -f /etc/nginx/sites-enabled/default
sudo tee /etc/nginx/sites-available/scuttle-grpc-origin >/dev/null <<EOF
# The Ark server gRPC API, for cloudflared.
server {
	listen 127.0.0.1:8445 ssl;
	http2 on;
	ssl_certificate $TLS/origin.crt;
	ssl_certificate_key $TLS/origin.key;
	location / {
		grpc_pass grpc://127.0.0.1:3535;
		# Mailbox and round streams stay open.
		grpc_read_timeout 1h;
		grpc_send_timeout 1h;
	}
}
EOF
sudo ln -sf /etc/nginx/sites-available/scuttle-grpc-origin /etc/nginx/sites-enabled/scuttle-grpc-origin
sudo nginx -t 2>/dev/null
sudo systemctl enable --now nginx
sudo systemctl reload nginx

echo "== test helpers"
sudo install -m 755 "$(dirname "$0")/devtools.py" /opt/scuttle/bin/devtools
sudo tee /etc/systemd/system/scuttle-devtools.service >/dev/null <<UNIT
[Unit]
Description=scuttle test faucet and miner
After=scuttle-bitcoind.service
[Service]
User=scuttle
ExecStart=/usr/bin/python3 /opt/scuttle/bin/devtools
Restart=on-failure
[Install]
WantedBy=multi-user.target
UNIT
sudo systemctl daemon-reload
sudo systemctl enable scuttle-devtools.service
sudo systemctl restart scuttle-devtools.service

echo "== tunnel $TUNNEL"
sudo install -d -m 750 /etc/cloudflared
sudo install -m 600 "$CREDENTIALS" "/etc/cloudflared/$TUNNEL.json"
sudo tee /etc/cloudflared/config.yml >/dev/null <<EOF
tunnel: $TUNNEL
credentials-file: /etc/cloudflared/$TUNNEL.json
ingress:
  - hostname: $POS
    path: ^/dev(/.*)?$
    service: http://127.0.0.1:3090
  - hostname: $POS
    service: http://127.0.0.1:3080
  - hostname: $BULL
    service: http://127.0.0.1:3080
  - hostname: $ARK
    service: https://127.0.0.1:8445
    originRequest:
      http2Origin: true
      noTLSVerify: true
  - hostname: $ESPLORA
    service: http://127.0.0.1:3002
  - hostname: $FAUCET
    service: http://127.0.0.1:3090
  - hostname: $WALLET
    service: http://127.0.0.1:8447
  - hostname: $EXCHANGE
    service: http://127.0.0.1:8088
  - service: http_status:404
EOF
sudo cloudflared tunnel --config /etc/cloudflared/config.yml ingress validate
sudo tee /etc/systemd/system/scuttle-cloudflared.service >/dev/null <<EOF
[Unit]
Description=scuttle Cloudflare Tunnel
After=network-online.target
Wants=network-online.target
[Service]
ExecStart=/usr/bin/cloudflared --no-autoupdate --config /etc/cloudflared/config.yml tunnel --protocol http2 run
Restart=on-failure
RestartSec=5
[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable scuttle-cloudflared.service
sudo systemctl restart scuttle-cloudflared.service

echo
echo "Point DNS at the tunnel, from a machine with a cloudflared login for $DOMAIN:"
for h in "$POS" "$BULL" "$ARK" "$ESPLORA" "$FAUCET" "$WALLET" "$EXCHANGE"; do echo "  cloudflared tunnel route dns $TUNNEL $h"; done
echo "Then give posd its public URLs:"
echo "  NETWORK=… PUBLIC_URL=https://$POS BULL_PUBLIC_URL=https://$BULL ./contrib/deploy/signet/setup.sh && sudo systemctl restart scuttle-posd"
echo "Wallets: --ark https://$ARK --esplora https://$ESPLORA"
