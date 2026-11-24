#!/usr/bin/env bash
# End-to-end check of a deployment. It registers a store from a bark
# wallet, pairs a terminal, and has two sales paid: one by the external
# Lightning node and one by a second bark wallet of the same Ark server,
# which pays inside the Ark for free. The owner's wallet stays closed while
# the sales are paid.
#
# Run after setup.sh and fund.sh. It creates its wallets under ~/smoke and
# reuses them on the next run.
set -euo pipefail

DATA=/var/lib/scuttle
PREFIX=/opt/scuttle
POSD=http://127.0.0.1:3080
WORK="$HOME/smoke"
mkdir -p "$WORK"
eval "$(sudo cat /etc/scuttle/secrets.env)"

eval "$(sudo cat /etc/scuttle/deploy.env)"   # NETWORK, RPC_PORT
cli() { sudo -u scuttle "$PREFIX/bin/chain-cli" "$@"; }
mine() { sudo -u scuttle "$PREFIX/bin/mine" "$1" >/dev/null; }
lncli() { sudo docker exec "scuttle-$1" lightning-cli --network="$NETWORK" --lightning-dir=/data/.lightning "${@:2}"; }
bark() { local w=$1; shift; "$PREFIX/bin/bark" --datadir "$WORK/$w" -q "$@" 2>/dev/null; }
api() { # method path [token] [json]
	curl -sf -X "$1" "$POSD$2" ${3:+-H "Authorization: Bearer $3"} \
		${4:+-H 'Content-Type: application/json' -d "$4"}
}
new_wallet() {
	[ -d "$WORK/$1" ] && return
	"$PREFIX/bin/bark" create --datadir "$WORK/$1" --"$NETWORK" --ark http://127.0.0.1:3535 \
		--bitcoind "http://127.0.0.1:$RPC_PORT" --bitcoind-user scuttle --bitcoind-pass "$BITCOIND_RPC_PASS" \
		>/dev/null 2>&1
}
wait_state() { # token id state
	for i in $(seq 1 90); do
		state=$(api GET "/api/v1/sales/$2" "$1" | jq -r .state)
		[ "$state" = "$3" ] && return 0
		sleep 2
	done
	echo "sale $2 is $state, expected $3" >&2
	return 1
}

echo "== owner registers the store"
new_wallet owner
# The store is the address, so keep using the first one.
[ -f "$WORK/owner-address" ] || bark owner address > "$WORK/owner-address"
address=$(cat "$WORK/owner-address")
# The host app's on-chain account, here a watch-only descriptor from bitcoind.
cli -named createwallet wallet_name=owner-onchain >/dev/null 2>&1 || true
descriptor=$(cli -rpcwallet=owner-onchain listdescriptors \
	| jq -r '[.descriptors[] | select(.desc | startswith("tr(")) | select(.internal == false)][0].desc')
ts=$(date +%s)
msg=$(printf 'posd registration v1\nname:%s\nark_address:%s\nonchain_descriptor:%s\nsilent_payment_address:%s\ncurrency:%s\nbrand:%s\nlightning_address_name:%s\ntimestamp:%s' \
	"Smoke Cafe" "$address" "$descriptor" "" CRC bitcoinjungle "" "$ts")
sig=$(bark owner message sign "$msg" "$address" | jq -r .signature)
reg=$(jq -n --arg a "$address" --arg d "$descriptor" --argjson t "$ts" --arg s "$sig" \
	'{registration: {name: "Smoke Cafe", ark_address: $a, onchain_descriptor: $d, currency: "CRC", brand: "bitcoinjungle", timestamp: $t}, signature: $s}')
api POST /api/v1/stores "" "$reg" >/dev/null || echo "(store already registered)"
challenge=$(api POST /api/v1/owner/challenge "" "{\"ark_address\": \"$address\"}" | jq -r .challenge)
login_sig=$(bark owner message sign "$challenge" "$address" | jq -r .signature)
owner=$(api POST /api/v1/owner/login "" "$(jq -n --arg a "$address" --arg c "$challenge" --arg s "$login_sig" \
	'{ark_address: $a, challenge: $c, signature: $s}')" | jq -r .token)

echo "== terminal pairs"
code=$(api POST /api/v1/owner/invites "$owner" | jq -r .code)
till=$(api POST /api/v1/pair "" "{\"code\": \"$code\", \"device_name\": \"Smoke till\"}" | jq -r .token)

echo "== sale 1, paid over Lightning by the external node"
sale=$(api POST /api/v1/sales "$till" '{"amount_sat": 21000, "memo": "smoke lightning"}')
id=$(echo "$sale" | jq -r .id)
lncli cln-external xpay "$(echo "$sale" | jq -r .lightning_invoice)" >/dev/null
wait_state "$till" "$id" paid
api GET "/api/v1/sales/$id" "$till" | jq -c '{state, paid_rail, received_sat}'

echo "== sale 2, paid inside the Ark by a wallet of the same server"
new_wallet payer
if [ "$(bark payer balance | jq '.spendable_sat')" -lt 100000 ]; then
	cli -rpcwallet=miner sendtoaddress "$(bark payer onchain address | jq -r .address)" 0.01 >/dev/null
	mine 1
	bark payer onchain sync >/dev/null 2>&1 || true
	bark payer board --all >/dev/null
	mine 2
	for i in $(seq 1 60); do
		[ "$(bark payer balance | jq '.spendable_sat')" -ge 100000 ] && break
		sleep 2
	done
fi
before=$(bark payer balance | jq '.spendable_sat')
sale=$(api POST /api/v1/sales "$till" '{"amount_sat": 30000, "memo": "smoke intra-ark"}')
id=$(echo "$sale" | jq -r .id)
bark payer send "$(echo "$sale" | jq -r .lightning_invoice)" >/dev/null
wait_state "$till" "$id" paid
api GET "/api/v1/sales/$id" "$till" | jq -c '{state, paid_rail, received_sat}'
fee=$(( before - $(bark payer balance | jq '.spendable_sat') - 30000 ))
echo "payer fee: $fee sat"
[ "$fee" -eq 0 ] || { echo "an intra-Ark payment must be free, the payer paid $fee sat" >&2; exit 1; }

echo "== the owner's wallet opens and holds the funds"
bark owner sync >/dev/null 2>&1 || true
bark owner balance | jq -c '{spendable_sat}'
