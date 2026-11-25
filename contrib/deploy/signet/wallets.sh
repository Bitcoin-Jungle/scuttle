#!/usr/bin/env bash
# Drives a public signet or regtest deployment from your own machine: two local bark
# wallets (a store owner and a customer), the owner API and terminals.
#
# Wallets
#   create owner|customer             new wallet
#   balance owner|customer            Ark balance
#   onchain-balance owner|customer    on-chain balance
#   faucet <wallet> [btc]             test coins on-chain (default 0.01)
#   board <wallet>                    move all on-chain coins into Ark
#   mine [n]                          mine n blocks now (default 1)
#   bark <wallet> <args...>           any bark command
#
# Owner
#   register "<name>" [CRC] [brand] [lnname]
#                                     register the owner's store; brand is
#                                     bitcoinjungle (default) or bullbitcoin,
#                                     lnname its Lightning address name
#   store                             show the store
#   store-set <key>=<value>...        name, currency, brand, onchain_min_sat,
#                                     lightning_address_name ("" removes it)
#   pair                              a single-use pairing link for a till
#   devices                           list paired terminals
#   revoke <device-id>                revoke a terminal
#   owner-sales [n]                   the store's latest sales
#   owner-csv [from] [to]             CSV export (dates YYYY-MM-DD)
#
# Terminal (a till run from this script; the PWA is the real one)
#   till-pair <code|url> [name]       pair a till here (default name: till)
#   till [name]                       terminal info
#   rate                              the store's bitcoin price
#   sell <amount> [memo]              new sale; amount is fiat ("6250",
#                                     "12.50") or sats ("20000sat")
#   sale [id]                         show a sale (default: the last one)
#   wait [id] [state]                 wait until the sale leaves state
#                                     (default: open)
#   cancel [id]                       cancel an open sale
#   sales [n]                         the till's latest sales
#   csv [from] [to]                   CSV export from the till
#   qr [id]                           print the sale's QR (needs qrencode)
#
# Customer
#   pay <wallet> [id] [lightning|onchain]
#                                     pay a sale (default: the last one, by
#                                     lightning, which is free inside the
#                                     Ark). onchain pays from the wallet's
#                                     on-chain coins; mine 1 to confirm.
#   pay-uri <wallet> <uri|invoice>    pay a copied QR or invoice, default way
#   lnaddress <wallet> <name> <sats>  pay the store's Lightning address
#
# Use TILL=<name> to pick a till and NETWORK=regtest for the regtest
# deployment (scuttle-regtest.vibecode.casa). State lives in $SIGNET_HOME (default
# ~/.scuttle-$NETWORK). The faucet, mining and the store's on-chain wallet use
# the deployment's public test helpers at https://<host>/dev/.
set -euo pipefail

NETWORK="${NETWORK:-signet}"
if [ "$NETWORK" = regtest ]; then
	# Published by cloudflare.sh: one host per service.
	HOST="${SIGNET_HOST:-scuttle-regtest.vibecode.casa}"
	ARK="https://${HOST%%.*}-ark.${HOST#*.}"
	ESPLORA="https://${HOST%%.*}-esplora.${HOST#*.}"
else
	# Published by public.sh over Tailscale Funnel: one host, three ports.
	HOST="${SIGNET_HOST:-scuttle-$NETWORK.tailc408a.ts.net}"
	ARK="https://$HOST:8443"
	ESPLORA="https://$HOST:10000"
fi
POSD="https://$HOST"
HOME_DIR="${SIGNET_HOME:-$HOME/.scuttle-$NETWORK}"
REPO="$(cd "$(dirname "$0")/../../.." && pwd)"
BARK="${BARK:-$REPO/target/debug/bark}"
TILL="${TILL:-till}"

mkdir -p "$HOME_DIR/tills"

die() { echo "error: $*" >&2; exit 1; }
wallet_dir() {
	case "$1" in owner|customer) echo "$HOME_DIR/$1" ;; *) die "wallet is owner or customer" ;; esac
}
bark() { local d; d=$(wallet_dir "$1"); shift; "$BARK" --datadir "$d" -q "$@" 2>/dev/null; }
dev() { # endpoint json
	curl -sS -f -X POST "$POSD/dev/$1" -H 'Content-Type: application/json' -d "$2" \
		|| die "the test helper /dev/$1 failed"
}
mine() { dev mine "{\"blocks\": ${1:-1}}" | jq -r .height; }

# Calls posd. Prints the body; on an error status prints it and fails.
api() { # method path [token] [json]
	local out status
	out=$(curl -sS -w '\n%{http_code}' -X "$1" "$POSD$2" ${3:+-H "Authorization: Bearer $3"} \
		${4:+-H 'Content-Type: application/json' -d "$4"})
	status=${out##*$'\n'}
	out=${out%$'\n'*}
	if [ "$status" -ge 400 ]; then
		echo "posd $1 $2: $status $out" >&2
		return 1
	fi
	printf '%s\n' "$out"
}

# The owner logs in by signing a challenge with the store's address. The
# token lasts 24 hours and is cached.
owner_token() {
	local cache="$HOME_DIR/owner-token" address challenge sig token
	# GNU stat, then BSD stat (macOS).
	mtime=$(stat -c %Y "$cache" 2>/dev/null || stat -f %m "$cache" 2>/dev/null || echo 0)
	if [ -f "$cache" ] && [ "$(( $(date +%s) - mtime ))" -lt 80000 ]; then
		cat "$cache"; return
	fi
	address=$(cat "$HOME_DIR/store-address" 2>/dev/null) || die "register the store first"
	challenge=$(api POST /api/v1/owner/challenge "" "$(jq -n --arg a "$address" '{ark_address: $a}')" | jq -r .challenge)
	sig=$(bark owner message sign "$challenge" "$address" | jq -r .signature)
	token=$(api POST /api/v1/owner/login "" "$(jq -n --arg a "$address" --arg c "$challenge" --arg s "$sig" \
		'{ark_address: $a, challenge: $c, signature: $s}')" | jq -r .token)
	(umask 077; echo "$token" > "$cache")
	echo "$token"
}

till_token() {
	cat "$HOME_DIR/tills/$TILL" 2>/dev/null || die "no till '$TILL'; run till-pair first"
}

sale_id() {
	if [ -n "${1:-}" ]; then echo "$1"; else cat "$HOME_DIR/last-sale" 2>/dev/null || die "no sale yet"; fi
}

get_sale() { api GET "/api/v1/sales/$1" "$(till_token)"; }

show_sale() {
	jq '{id, state, amount_sat, fiat_amount, currency, memo, received_sat, paid_rail, paid_at,
		expires_at, device_name, onchain_available, onchain_uri, onchain_received_sat, onchain_pending_sat}'
}

# A query parameter of a BIP 321 URI.
uri_param() { # uri name
	tr '?&' '\n\n' <<<"$1" | sed -n "s/^$2=//p" | head -1
}

cmd="${1:-}"; shift || true
case "$cmd" in

# Wallets

create)
	d=$(wallet_dir "${1:-}")
	[ -e "$d" ] && die "$d exists"
	"$BARK" create --datadir "$d" --"$NETWORK" --ark "$ARK" --esplora "$ESPLORA" >/dev/null 2>&1 \
		|| die "bark create failed, see $d/debug.log"
	echo "created $1: $(bark "$1" address)"
	;;
balance)
	bark "${1:?wallet}" balance | jq -c '{spendable_sat, pending_board_sat, pending_lightning_send_sat, pending_arkoor_send_sat}'
	;;
onchain-balance)
	bark "${1:?wallet}" onchain balance | jq -c '{confirmed_sat, trusted_pending_sat, untrusted_pending_sat}'
	;;
faucet)
	who="${1:?wallet}"; amount="${2:-0.01}"
	addr=$(bark "$who" onchain address | jq -r .address)
	dev faucet "$(jq -n --arg a "$addr" --argjson b "$amount" '{address: $a, btc: $b}')" \
		| jq -r --arg w "$who" '"sent \(.btc) BTC to \($w), txid \(.txid), confirmed at height \(.height)"'
	;;
board)
	who="${1:?wallet}"
	# Esplora takes a few seconds to index a new block.
	for i in $(seq 1 30); do
		[ "$(bark "$who" onchain balance | jq .confirmed_sat)" -gt 0 ] && break
		sleep 2
	done
	bark "$who" board --all >/dev/null || die "board failed: no confirmed on-chain coins?"
	# Boards need 3 confirmations on regtest, 1 on signet.
	if [ "$NETWORK" = regtest ]; then mine 3 >/dev/null; else mine 1 >/dev/null; fi
	for i in $(seq 1 30); do
		[ "$(bark "$who" balance | jq .pending_board_sat)" = 0 ] && break
		sleep 2
	done
	bark "$who" balance | jq -c '{spendable_sat, pending_board_sat}'
	;;
mine)
	echo "height $(mine "${1:-1}")"
	;;
bark)
	who="${1:?wallet}"; shift
	"$BARK" --datadir "$(wallet_dir "$who")" "$@"
	;;

# Owner

register)
	name="${1:?store name}"; currency="${2:-CRC}"; brand="${3:-bitcoinjungle}"; lnname="${4:-}"
	[ -f "$HOME_DIR/store-address" ] && die "already registered: $(cat "$HOME_DIR/store-address")"
	address=$(bark owner address)
	# The host app keeps the store's on-chain account. Here it is a wallet on
	# the deployment's bitcoind, named after the store.
	sw=$(dev store-wallet "$(jq -n --arg n "$name" '{name: $n}')")
	wallet=$(jq -r .wallet <<<"$sw")
	descriptor=$(jq -r .onchain_descriptor <<<"$sw")
	ts=$(date +%s)
	msg=$(printf 'posd registration v1\nname:%s\nark_address:%s\nonchain_descriptor:%s\nsilent_payment_address:%s\ncurrency:%s\nbrand:%s\nlightning_address_name:%s\ntimestamp:%s' \
		"$name" "$address" "$descriptor" "" "$currency" "$brand" "$lnname" "$ts")
	sig=$(bark owner message sign "$msg" "$address" | jq -r .signature)
	api POST /api/v1/stores "" "$(jq -n --arg n "$name" --arg a "$address" \
		--arg d "$descriptor" --arg c "$currency" --arg br "$brand" --arg ln "$lnname" --argjson t "$ts" --arg s "$sig" \
		'{registration: {name: $n, ark_address: $a, onchain_descriptor: $d, currency: $c, brand: $br, lightning_address_name: $ln, timestamp: $t}, signature: $s}')" \
		| jq -c .store
	echo "$address" > "$HOME_DIR/store-address"
	echo "$wallet" > "$HOME_DIR/store-onchain-wallet"
	echo "on-chain sales go to the test wallet '$wallet' on the deployment"
	;;
store)
	api GET /api/v1/owner/store "$(owner_token)" | jq .
	;;
store-set)
	[ $# -gt 0 ] || die "give key=value pairs"
	body='{}'
	for kv in "$@"; do
		k=${kv%%=*}; v=${kv#*=}
		case "$k" in
			onchain_min_sat) body=$(jq --arg k "$k" --argjson v "$v" '.[$k] = $v' <<<"$body") ;;
			name|currency|brand|lightning_address_name) body=$(jq --arg k "$k" --arg v "$v" '.[$k] = $v' <<<"$body") ;;
			*) die "unknown key $k" ;;
		esac
	done
	api PATCH /api/v1/owner/store "$(owner_token)" "$body" | jq .
	;;
pair)
	api POST /api/v1/owner/invites "$(owner_token)" | jq -r '"\(.url)\n(single use, expires \(.expires_at))"'
	;;
devices)
	api GET /api/v1/owner/devices "$(owner_token)" | jq -r '.devices[] | "\(.id)\t\(.name)\t\(.created_at)\t\(if .revoked_at then "revoked" else "active" end)"'
	;;
revoke)
	api DELETE "/api/v1/owner/devices/${1:?device id}" "$(owner_token)" >/dev/null
	echo "revoked device $1"
	;;
owner-sales)
	api GET "/api/v1/owner/sales?limit=${1:-10}" "$(owner_token)" \
		| jq -r '.sales[] | "\(.id)\t\(.state)\t\(.amount_sat) sat\t\(.fiat_amount // "-") \(.currency // "")\t\(.paid_rail // "-")\t\(.device_name // "lnaddress")\t\(.memo // "")"'
	;;
owner-csv)
	q=""; [ -n "${1:-}" ] && q="from=$1"; [ -n "${2:-}" ] && q="$q&to=$2"
	api GET "/api/v1/owner/sales.csv?$q" "$(owner_token)"
	;;

# Terminal

till-pair)
	code="${1:?pairing code or link}"; code=${code##*/pair/}; name="${2:-$TILL}"
	out=$(api POST /api/v1/pair "" "$(jq -n --arg c "$code" --arg n "$name" '{code: $c, device_name: $n}')")
	(umask 077; jq -r .token <<<"$out" > "$HOME_DIR/tills/$name")
	echo "paired till '$name' to $(jq -r .store.name <<<"$out")"
	;;
till)
	TILL="${1:-$TILL}"
	api GET /api/v1/terminal "$(till_token)" | jq .
	;;
rate)
	api GET /api/v1/rate "$(till_token)" | jq -c .
	;;
sell)
	amount="${1:?amount}"; memo="${2:-}"
	if [[ "$amount" == *sat ]]; then
		body=$(jq -n --argjson a "${amount%sat}" --arg m "$memo" '{amount_sat: $a} + (if $m == "" then {} else {memo: $m} end)')
	else
		# A fiat amount names the currency it is in, as the terminal does.
		currency=$(api GET /api/v1/terminal "$(till_token)" | jq -r .store.currency)
		body=$(jq -n --arg a "$amount" --arg c "$currency" --arg m "$memo" \
			'{fiat_amount: $a, currency: $c} + (if $m == "" then {} else {memo: $m} end)')
	fi
	sale=$(api POST /api/v1/sales "$(till_token)" "$body")
	jq -r .id <<<"$sale" > "$HOME_DIR/last-sale"
	show_sale <<<"$sale"
	;;
sale)
	get_sale "$(sale_id "${1:-}")" | show_sale
	;;
wait)
	id=$(sale_id "${1:-}"); from="${2:-open}"
	# The API long-polls for 25 s at a time.
	for i in $(seq 1 24); do
		sale=$(api GET "/api/v1/sales/$id?wait=$from" "$(till_token)")
		[ "$(jq -r .state <<<"$sale")" != "$from" ] && break
	done
	show_sale <<<"$sale"
	;;
cancel)
	api POST "/api/v1/sales/$(sale_id "${1:-}")/cancel" "$(till_token)" | show_sale
	;;
sales)
	api GET "/api/v1/sales?limit=${1:-10}" "$(till_token)" \
		| jq -r '.sales[] | "\(.id)\t\(.state)\t\(.amount_sat) sat\t\(.fiat_amount // "-") \(.currency // "")\t\(.paid_rail // "-")\t\(.memo // "")"'
	;;
csv)
	q=""; [ -n "${1:-}" ] && q="from=$1"; [ -n "${2:-}" ] && q="$q&to=$2"
	api GET "/api/v1/sales.csv?$q" "$(till_token)"
	;;
qr)
	command -v qrencode >/dev/null || die "install qrencode"
	get_sale "$(sale_id "${1:-}")" | jq -r .lightning_invoice | qrencode -t ansiutf8
	;;

# Customer

pay)
	who="${1:?wallet}"; id=$(sale_id "${2:-}"); method="${3:-lightning}"
	sale=$(get_sale "$id")
	case "$method" in
	lightning)
		# A wallet of this Ark server pays the invoice inside the Ark, free.
		inv=$(jq -r .lightning_invoice <<<"$sale")
		before=$(bark "$who" balance | jq .spendable_sat)
		bark "$who" send "$inv" >/dev/null || die "payment failed, see $(wallet_dir "$who")/debug.log"
		after=$(bark "$who" balance | jq .spendable_sat)
		echo "fee paid: $(( before - after - $(jq -r .amount_sat <<<"$sale") )) sat"
		;;
	onchain)
		# As the cashier picking on-chain: the sale gets its address now.
		[ "$(jq -r .onchain_available <<<"$sale")" = true ] \
			|| die "no on-chain: the sale is below the store's on-chain minimum"
		sale=$(api POST "/api/v1/sales/$id/onchain" "$(till_token)")
		addr=$(jq -r '.onchain_address // empty' <<<"$sale")
		[ -n "$addr" ] || die "posd gave no on-chain address"
		bark "$who" onchain send "$addr" "$(jq -r .amount_sat <<<"$sale") sats" | jq -c . \
			|| die "on-chain payment failed: does the wallet have on-chain coins? (faucet)"
		echo "sent; the sale is in progress until 'mine 1'"
		;;
	*) die "method is lightning or onchain" ;;
	esac
	echo "paid $id by $method"
	;;
pay-uri)
	who="${1:?wallet}"; uri="${2:?uri or invoice}"
	if [[ "$uri" == bitcoin:* ]]; then
		inv=$(uri_param "$uri" lightning)
		[ -n "$inv" ] || die "no invoice in the URI"
		bark "$who" send "$inv" >/dev/null || die "payment failed"
	else
		bark "$who" send "$uri" >/dev/null || die "payment failed"
	fi
	echo "paid"
	;;
lnaddress)
	who="${1:?wallet}"; name="${2:?name}"; sats="${3:?sats}"
	bark "$who" send "$name@$HOST" "$sats sats" >/dev/null || die "payment failed, see $(wallet_dir "$who")/debug.log"
	echo "paid $sats sats to $name@$HOST"
	;;

*)
	sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
	;;
esac
