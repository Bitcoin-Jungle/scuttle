# posd HTTP API

The machine-readable contract is [`pos/openapi.json`](../openapi.json),
generated from the handlers (posd also serves it at `/api/v1/openapi.json`,
and `posd openapi` prints it). A unit test fails when the committed copy is
stale; `just dump-posd-openapi` rewrites it. This guide covers what a schema
cannot: the signed message formats, the payment rails and the flows.

posd is the point-of-sale and Lightning address service. It serves the
terminal PWA, the owner API and LNURL. A store has terminals, a Lightning
address, or both: a person's Lightning address is a store with no terminals.

One posd serves several brands, each on its own domain, set in the config
(`[brands.<key>]`, see [brands.md](brands.md)). A store's `brand` is the key
of one: it sets the look of its terminals and the domain of its Lightning
address and pairing links.

All bodies are JSON. Enum values use kebab-case. Amounts in sats are
integers. Fiat amounts are decimal strings in the store currency, for
example `"12.50"`. Times are RFC 3339 strings in UTC.

There are four callers, each with its own credential:

| Caller | Credential | Section |
|---|---|---|
| A paired terminal | device token from pairing | Terminal |
| A store's owner wallet | owner token from a signed login | Owner |
| A brand's own services | the brand's `admin_token` | Operator |
| Any payer | none | LNURL |

Errors return a non-2xx status with `{"error": "<message>"}`:

| Status | Meaning |
|---|---|
| 400 | The request is not valid |
| 401 | The token is missing, not valid, or revoked |
| 404 | The resource does not exist for this caller |
| 409 | The request conflicts with the current state |
| 429 | Too many unpaid Lightning address invoices are open |
| 502 | An upstream service (Ark server, barkd, price feed) failed |

## Payment rails

| Rail | `paid_rail` | Payer | How posd knows it is paid |
|---|---|---|---|
| Lightning | `lightning` | Any Lightning wallet, including an Ark wallet on the same Ark server | barkd delivered the claim to the owner's mailbox |
| On-chain | `onchain` | Any bitcoin wallet | 1 confirmation |

A Bolt Card pays the Lightning invoice: see Bolt Cards under Terminal.

The terminal shows one rail's QR code at a time, which stays sparse enough
for any phone to scan. posd gives no combined BIP 321 URI: wallets that only
know BIP 21 mishandle one.

- `lightning_invoice` is the forwarding invoice. A wallet on the same Ark
  server pays it without leaving the Ark, at the server's intra-Ark fees.
- `onchain_address` is `null` until the cashier picks on-chain: then
  `POST /api/v1/sales/<id>/onchain` gives the sale an unused address from the
  store's descriptor. `onchain_available` says whether it can: a terminal's
  sale of at least the store's `onchain_min_sat`. Making the address only
  when it may be paid keeps the descriptor from filling with addresses no
  payer saw, which a wallet restored from seed would have to scan past.
  No two sales get the same address, even of two stores that registered the
  same descriptor. A sale also skips an address that already received a
  payment posd can see, such as one of the wallet's own receives: with
  Esplora any payment, with bitcoind one from the last two hours, as older
  ones never count. The wallet must still keep a dedicated account for the
  descriptor: a receive address it hands out later could match a sale. The
  owner's store shows `next_onchain_index`, how far posd has gone into it.
- `onchain_uri` is the address and the full amount as a BIP 21 URI,
  `bitcoin:<address>?amount=<btc>`. It keeps the full amount after a partial
  payment, which `onchain_received_sat` and `onchain_pending_sat` show.

## Types

### Sale

```json
{
  "id": "f1a01eb4e3e74b15220ac2be24fc38e0",
  "created_at": "2026-11-20T15:04:05Z",
  "expires_at": "2026-11-20T15:14:05Z",
  "state": "open",
  "amount_sat": 35000,
  "fiat_amount": "12.50",
  "currency": "CRC",
  "btc_price": "62500000.00",
  "memo": "2 coffees",
  "lightning_invoice": "lnbc…",
  "onchain_available": true,
  "onchain_address": "bc1p…",
  "onchain_uri": "bitcoin:bc1p…?amount=0.00035",
  "onchain_received_sat": 0,
  "onchain_pending_sat": 0,
  "paid_at": null,
  "paid_rail": null,
  "received_sat": 0,
  "device_name": "Front till"
}
```

- `state` is `open`, `in-progress`, `paid`, `expired` or `canceled`.
  - `in-progress` means an on-chain payment is in the mempool. The sale stays
    `in-progress` past `expires_at` until the payment confirms.
  - `paid` is final. `received_sat` can be higher than `amount_sat` when the
    payer overpaid or paid more than one rail.
  - `expired` and `canceled` are final too, but posd keeps recording payments
    that arrive later in `received_sat`: an on-chain payment to an address a
    payer saw, for a week by default. A Lightning payment that barkd
    delivers after the sale expired, as after a posd outage, still makes it
    `paid`: the invoice expired with the sale, so it was paid in time.
- `paid_rail` is `lightning`, `onchain` or `null`.
- `onchain_address` and `onchain_uri` are `null` until the cashier picks
  on-chain.
- `onchain_received_sat` is confirmed and `onchain_pending_sat` in the
  mempool, both to the on-chain address. On an `open` sale, a non-zero sum is
  a partial payment: the payer still owes the rest.
- `fiat_amount`, `currency` and `btc_price` are `null` when the sale was keyed
  in sats.
- `device_name` is `null` for a Lightning address payment.

### Store

```json
{
  "name": "Corner Café",
  "currency": "CRC",
  "currency_decimals": 2,
  "onchain_min_sat": 50000,
  "min_sale_sat": 2,
  "lightning_address": "cornercafe@pay.example.com",
  "brand": "example"
}
```

- `currency_decimals` is how many decimal places a sale in `currency` may
  have.
- `min_sale_sat` is the smallest sale. bark makes no Lightning invoice that
  the Ark server's receive fee would leave empty: at 0.20%, rounded up to a
  whole sat, a 1-sat invoice leaves nothing, so the minimum is 2. It
  follows the server's fees. A smaller sale is refused with `400`.
- `lightning_address` is `null` when the store has none. Its domain is the
  host of the brand's `public_url`.
- `brand` is the key of the store's brand. It sets the terminal's look and
  the domain of the Lightning address and pairing links. The pairing page
  `/pair/<code>` and its manifest are in the brand of the code's store, so an
  installed terminal gets the brand's name and icon. Other pages are in the
  brand of the domain they are asked for.

## Terminal

A terminal sends `Authorization: Bearer <device token>`.

### Pairing

- `GET /pair/<code>` serves the PWA. The page links the manifest
  `/pair/<code>/manifest.webmanifest`, whose `start_url` is `/pair/<code>`, so
  the installed app opens with the code.
- `POST /api/v1/pair` with `{"code": "<code>", "device_name": "Front till"}`
  returns `{"token": "<device token>", "store": Store}`. A code works once.
  `404` means the code is unknown, used or expired.

### Endpoints

| Method and path | Body | Returns |
|---|---|---|
| `GET /api/v1/terminal` | | `{"store": Store, "device_name": "Front till"}` |
| `GET /api/v1/rate` | | `{"currency": "CRC", "btc_price": "62500000.00", "fetched_at": "…"}` |
| `POST /api/v1/sales` | `{"fiat_amount": "12.50", "currency": "CRC", "memo": "…"}` or `{"amount_sat": 35000, "memo": "…"}` | `Sale` |
| `GET /api/v1/sales/<id>` | | `Sale` |
| `GET /api/v1/sales/<id>?wait=<state>&onchain_sat=<n>` | | `Sale`, returned when the state is not `<state>`, when `onchain_received_sat + onchain_pending_sat` is not `<n>`, or after 25 s; `onchain_sat` is optional |
| `POST /api/v1/sales/<id>/onchain` | | `Sale` with its on-chain address, the same one on a repeat; `409` when the sale is not `open`, is a Lightning address payment, is below the store's `onchain_min_sat`, or the store has no unused address |
| `POST /api/v1/sales/<id>/bolt-card` | `{"link": "lnurlw://…"}` | `Sale`, still `open`: the card's service took the invoice; see Bolt Cards below |
| `POST /api/v1/sales/<id>/cancel` | | `Sale`; `409` unless the sale is `open`, or when a payment for it, or part of one, arrived |
| `GET /api/v1/sales?before=<id>&limit=<n>` | | `{"sales": [Sale]}`, newest first, at most 100 |
| `GET /api/v1/sales.csv?from=<date>&to=<date>` | | CSV, one row per sale |

`memo` is optional and at most 200 characters. `currency` is the currency
the terminal showed the amount in. posd takes a request without it, but a
terminal sends it: when it is not the store's, as after the owner changed
it, the sale is refused with `409` instead of charging the amount in
another currency.

A cancel first checks the sale's on-chain address, then asks barkd to
cancel the Lightning invoice. barkd forgets an unpaid invoice and posd stops
watching it. The Ark server does not cancel invoices, so the invoice stays
payable until it expires, and a payment made to it after the cancel is not
delivered to the owner. When a payment was made on either rail, the sale is
not canceled: the cancel is `409`, and the sale soon shows `in-progress` or
`paid`. Part of the amount paid on-chain also makes it `409`: the cashier
takes the rest, or lets the sale expire with the part recorded. When barkd
fails to cancel an unpaid invoice, the cancel is `502` and the sale stays
open.

`POST /api/v1/sales` takes an optional `Idempotency-Key` header: 1 to 64
letters, digits, `-` or `_`, unique per sale on the device. A request
repeated with the same key and body, as after a response lost on a poor
network, returns the sale of the first request instead of making another.
The same key with a different body is `409`.

The terminal follows a sale by repeating the `?wait=` request with the last
state and on-chain sum it saw. After any drop, the next request returns the
current sale at once, so no change, a partial payment included, is missed.

### Bolt Cards

A Bolt Card is an NFC card that holds an LNURL-withdraw link (LUD-03),
new on every tap. The terminal reads it with Web NFC, which only Chrome on
Android has, and sends the link as `link`; an `LNURL1…` in bech32 works
too. posd fetches the withdraw request, checks the sale's amount against
the card's limits, and calls the card's callback with the sale's
`lightning_invoice`. The card's service then pays it, and the sale is paid
like any other Lightning payment, with `paid_rail` `lightning`.

posd, not the terminal, talks to the card's service. A browser could reach
only services that allow it with CORS, and posd decides which invoice is
paid. As the link comes from the terminal, posd fetches only https URLs on
public addresses, checked again after DNS, and follows no redirects.
`[bolt_card] allow_private_hosts` lifts that on test networks.

| Status | Meaning |
|---|---|
| `200` | The card's service took the invoice; the payment arrives a moment later |
| `400` | `link` is not a withdraw link, or not one posd may fetch |
| `409` | The sale is not `open`, or expired |
| `422` | The card refused: `error` is the service's reason, or the card's limits when the amount is outside them |
| `502` | The card's service could not be reached or did not answer JSON |

`from` and `to` are dates (`2026-11-20`, in UTC) or RFC 3339 times. A date as
`to` includes that whole day.

The CSV columns are: `id, created_at, paid_at, state, paid_rail, amount_sat,
received_sat, fiat_amount, currency, btc_price, memo, device_name`.

## Owner

The owner is the Ark address that registered the store. The owner's wallet
signs with its Ark key, using bark's message signing (`bark/message` tag,
BIP 340 Schnorr over SHA-256).

### Registration

`POST /api/v1/stores`, with no token:

```json
{
  "registration": {
    "name": "Corner Café",
    "ark_address": "ark1…",
    "onchain_descriptor": "tr([fingerprint/86'/0'/1']xpub…/0/*)",
    "silent_payment_address": "sp1q…",
    "currency": "CRC",
    "brand": "example",
    "lightning_address_name": "cornercafe",
    "timestamp": 1795190400
  },
  "signature": "<hex>"
}
```

The signed message is these lines, joined by `\n`, with no trailing newline:

```text
posd registration v1
name:<name>
ark_address:<ark_address>
onchain_descriptor:<onchain_descriptor>
silent_payment_address:<silent_payment_address>
currency:<currency>
brand:<brand>
lightning_address_name:<lightning_address_name>
timestamp:<timestamp>
```

`lightning_address_name` is the name of the store's Lightning address on
its brand's domain (`cornercafe` for `cornercafe@pay.example.com`), or
empty for none, in which case the line is `lightning_address_name:` and the
field may be left out of the JSON. A name has 1 to 64 lowercase letters,
digits, `.`, `-` and `_`.

`silent_payment_address` is the owner's BIP 352 address (`sp1q…`, `tsp1q…`
on test networks), which the store's Lightning address offers to payers that
ask for it (see LNURL). It is empty for none, like `lightning_address_name`.
posd checks that it is a version 0 address for its network with two valid
keys; it never pays to it or scans for it.

`currency` is one that posd's price source quotes, which the config lists
(`[price] currencies`). With the Bull Bitcoin source they are `USD`, `CAD`,
`MXN`, `CRC`, `EUR`, `ARS`, `AUD` and `COP`. A store's `currency_decimals`
says how many decimal places its sales may have.

`onchain_descriptor` must be a dedicated account of the owner's wallet,
which the wallet gives out for nothing else. See Payment rails.

posd refuses the registration with `400` when:
- The signature does not verify for `ark_address`.
- The timestamp is more than 10 minutes away from its clock.
- The address is not for its Ark server.
- The descriptor is not a public, ranged descriptor.
- The silent payment address is not a version 0 address for posd's network.
- The currency is not one of those.
- The posd does not serve the brand `brand`.
- The Lightning address name breaks the rule above.

and with `409` when:
- A store is already registered for `ark_address`.
- The Lightning address name is taken, reserved or held on the brand's
  domain.

The response is `{"store": OwnerStore}`: the `Store`, and

```json
{
  "ark_address": "ark1…",
  "onchain_descriptor": "tr([fingerprint/86'/0'/1']xpub…/0/*)",
  "next_onchain_index": 12,
  "silent_payment_address": "sp1q…"
}
```

`next_onchain_index` is the next index posd gives a sale from the descriptor.
Every index below it may have been shown to a payer, so a wallet that
restores the account scans at least that far, and gives none of those
addresses out for its own receives.

### Login

1. `POST /api/v1/owner/challenge` with `{"ark_address": "ark1…"}` returns
   `{"challenge": "<text>"}`, valid for 5 minutes. Sign the text as it is:
   its last line, `mac:<hex>`, is posd's HMAC of the lines above it, so posd
   stores nothing for a challenge.
2. `POST /api/v1/owner/login` with `{"ark_address": "ark1…", "challenge":
   "<text>", "signature": "<hex>"}` returns `{"token": "<owner token>",
   "expires_at": "…"}`. The token lasts 24 hours. A challenge works once:
   posd records its nonce at login.

### Endpoints

The owner sends `Authorization: Bearer <owner token>`.

| Method and path | Body | Returns |
|---|---|---|
| `GET /api/v1/owner/store` | | `OwnerStore` |
| `PATCH /api/v1/owner/store` | any of `name`, `currency`, `brand`, `onchain_min_sat`, `lightning_address_name` (`""` removes it) | `OwnerStore`; `400` for a brand this posd does not serve, `409` for a name taken, reserved or held on the store's domain |
| `POST /api/v1/owner/invites` | | `{"code": "…", "url": "https://…/pair/<code>", "expires_at": "…"}`; valid for 24 hours |
| `GET /api/v1/owner/devices` | | `{"devices": [{"id": 1, "name": "…", "created_at": "…", "revoked_at": null}]}` |
| `DELETE /api/v1/owner/devices/<id>` | | `204` |
| `GET /api/v1/owner/sales?before=<id>&limit=<n>` | | `{"sales": [Sale]}` |
| `GET /api/v1/owner/sales.csv?from=<date>&to=<date>` | | CSV |

### Held names

A Lightning address name a store gives up, by a rename, by removing its
address or by moving to another brand, is held for that store for 90 days.
Payers may have saved the old address: while it is held, it pays nobody
(the pay request is `404`) and no other store can take it, not even from the
operator. The store that gave it up can take it back. After the hold, anyone
can.

## Operator

A brand with an `admin_token` in the config has an operator API, called with
`Authorization: Bearer <admin_token>`. The operator works on its own brand's
domain only. It is for the brand's own services, for example to move users
of an older service: the service checks who a user is, then gives the user's
old name to the store the user's wallet registered.

| Method and path | Body | Returns |
|---|---|---|
| `POST /api/v1/admin/reserved-names` | `{"names": ["cornercafe", …]}`, at most 10,000 | `{"reserved": <names newly reserved>}` |
| `POST /api/v1/admin/lightning-addresses` | `{"ark_address": "ark1…", "name": "cornercafe"}` | `Store`; `404` if no store of the brand has the address, `409` if another store has or holds the name |

A reserved name cannot be taken at registration or by an owner's `PATCH`
(`409`), on that brand's domain only. The operator assigns it, reserved or
not. A store keeps an assigned name through later changes, but cannot move
it onto a domain where it is reserved. The name the store had is held for it
like any other it gives up.

## LNURL

A store with a `lightning_address_name` answers LUD-06 and LUD-16 on its
brand's domain. Each domain has its own names: `cafe@pay.example.com`
and `cafe@pay.example.org` can be two stores. posd tells the domains
apart by the request's `Host`, so a proxy in front of it must pass the
original host, or set `X-Forwarded-Host` with `http.trust_forwarded_host`.
A posd with one brand answers on any host. LNURL errors have the LUD-06
body, `{"status": "ERROR", "reason": "…"}`, so the payer's wallet shows the
reason.
The LNURL endpoints send `Access-Control-Allow-Origin: *` and answer CORS
preflights, so a wallet running in a browser on another site can use them
(LUD-01). The other endpoints send no CORS headers.

- `GET /.well-known/lnurlp/<name>` returns the pay request. `minSendable` is
  the store's `min_sale_sat` in msat and `maxSendable` is set in the posd
  config.
  `commentAllowed` is 200 (LUD-12): a payer's comment becomes the sale's
  memo, which the owner sees in the sales list and CSV. A longer one is
  refused.
  `payment_methods` lists the LUD-22 methods besides Lightning: `ARK`, and
  `SP` when the owner registered a silent payment address.
- `GET /api/v1/lnurlp/<name>/callback?amount=<msat>[&comment=<text>]` creates a sale with no
  device and returns `{"pr": "<forwarding invoice>", "routes": []}`. Asking
  for an invoice needs no credentials, so a store has at most 25 unpaid ones
  open at once, and a posd 500: past that the callback is `429` until some
  are paid or expire.
- `GET /api/v1/lnurlp/<name>/callback?amount=<msat>&payment_method=<methods>`
  (LUD-22) takes methods from `payment_methods`, comma-separated, most
  preferred first, and answers the first one the store offers with the
  owner's address: `{"ARK": {"address": "ark1…"}}` or `{"SP": {"address":
  "sp1q…"}}`. The payer pays the callback's amount to it directly. With no
  method the store offers, the answer is an invoice, so a payer tells the
  answers apart by their shape. These payments never pass posd: they are
  not in the store's sales.
