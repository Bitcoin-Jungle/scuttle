#!/usr/bin/env bash
# Serves bark-web in WASM mode: each visitor's wallet runs in their browser,
# with its seed in their browser's storage, so every tester has a wallet,
# store and Lightning address of their own. The page talks to the Ark server
# (gRPC-Web) and Esplora directly, and to posd through /posd/ on its own
# origin. Run after setup.sh and cloudflare.sh; publish port 8448 with
# cloudflare.sh (the <name>-wallet host).
#
#   BARK_WEB=<checkout of Bitcoin-Jungle/bark-web, branch scuttle> DOMAIN=vibecode.casa \
#     ./contrib/deploy/signet/webwallet-wasm.sh
#
# The checkout must carry vendor/bark-wasm, scuttle's bark built for the
# browser (its scripts/build-scuttle-wasm.sh): the npm release is upstream
# bark, which a server in expiry payout mode refuses.
set -euo pipefail

BARK_WEB="${BARK_WEB:?a bark-web checkout with vendor/bark-wasm}"
DOMAIN="${DOMAIN:?the domain cloudflare.sh published under}"
eval "$(sudo cat /etc/scuttle/deploy.env)"   # NETWORK
NAME="${NAME:-scuttle-$NETWORK}"
SITE=/opt/scuttle/webwallet-wasm
[ -f "$BARK_WEB/vendor/bark-wasm/package.json" ] || { echo "no vendor/bark-wasm in $BARK_WEB" >&2; exit 1; }

currencies=$(sudo grep -oP '^currencies = \{\K[^}]*' /etc/scuttle/posd.toml | grep -oP '[A-Z]{3}(?= =)' | paste -sd,)

echo "== bundle"
out=$(mktemp -d)
sudo docker run --rm -v "$BARK_WEB:/src:ro" -v "$out:/out" \
	-e BARK_NETWORK="$NETWORK" \
	-e ARK_SERVER="https://$NAME-ark.$DOMAIN" \
	-e CHAIN_SOURCE="https://$NAME-esplora.$DOMAIN" \
	-e "POS_BRANDS=bitcoinjungle:Bitcoin Jungle,bullbitcoin:Bull Bitcoin" \
	-e POS_CURRENCIES="$currencies" \
	docker.io/library/node:22-alpine sh -c \
	'cp -r /src /work && cd /work && npm ci --silent && npm run build:wasm >/dev/null && cp -r dist/. /out/'
sudo rm -rf "$SITE"
sudo install -d -m 755 "$SITE"
sudo cp -r "$out"/. "$SITE"/
sudo rm -rf "$out"

echo "== nginx"
sudo tee /etc/nginx/sites-available/scuttle-webwallet-wasm >/dev/null <<EOF
# bark-web in WASM mode. The wallet holds its seed in the page, so framing it
# is denied by header (the bundle's CSP meta tag cannot carry frame-ancestors).
server {
	listen 127.0.0.1:8448;
	root $SITE;
	index index.html;
	add_header Content-Security-Policy "frame-ancestors 'none'" always;
	add_header X-Frame-Options "DENY" always;
	add_header X-Content-Type-Options "nosniff" always;
	add_header Referrer-Policy "no-referrer" always;

	location = /index.html {
		add_header Content-Security-Policy "frame-ancestors 'none'" always;
		add_header X-Frame-Options "DENY" always;
		add_header X-Content-Type-Options "nosniff" always;
		add_header Referrer-Policy "no-referrer" always;
		add_header Cache-Control "no-store, no-cache, must-revalidate" always;
		try_files \$uri =404;
	}
	location /assets/ {
		add_header Content-Security-Policy "frame-ancestors 'none'" always;
		add_header X-Frame-Options "DENY" always;
		add_header X-Content-Type-Options "nosniff" always;
		add_header Referrer-Policy "no-referrer" always;
		add_header Cache-Control "public, max-age=31536000, immutable";
		try_files \$uri =404;
	}
	# posd on this origin, so the browser needs no CORS from posd.
	location /posd/ {
		proxy_pass http://127.0.0.1:3080/;
		proxy_read_timeout 60s;
	}
	location / {
		try_files \$uri \$uri/ /index.html;
	}
}
EOF
sudo ln -sf /etc/nginx/sites-available/scuttle-webwallet-wasm /etc/nginx/sites-enabled/scuttle-webwallet-wasm
sudo nginx -t 2>/dev/null
sudo systemctl reload nginx
curl -sf -o /dev/null http://127.0.0.1:8448/ && echo "WASM web wallet on 127.0.0.1:8448"
