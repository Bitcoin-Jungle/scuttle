# posd

posd is a point-of-sale and Lightning address service for Ark wallets.
Staff pair a phone or tablet with a store, install the terminal as a web
app, key in an amount and show a QR code for Lightning or on-chain. Every
payment goes to the store owner's wallet, which does not need to be online.
posd holds no bitcoin, and a terminal cannot send.

| Rail | The payer uses | Paid when |
|---|---|---|
| Lightning | Any Lightning wallet, including an Ark wallet on the same Ark server | barkd delivers the claim to the owner's mailbox |
| On-chain | Any bitcoin wallet, at or above the store minimum | 1 confirmation (bitcoind or Esplora); "in progress" before that |

A wallet on the same Ark server pays the Lightning invoice without leaving
the Ark: the server settles it internally and charges its intra-Ark fees
(`[fees.intra_ark]` in captaind). The owner receives VTXOs from the
server's pool, so their expiry does not depend on the payer's VTXOs.

The terminal shows one rail's QR code at a time, Lightning first, so the
code stays sparse enough for cheap phones to scan off a low-resolution
screen: the bare invoice or a BIP 21 address and amount, in upper case for
the QR's alphanumeric mode. The API also returns the BIP 321 URI of both
(`payment_request`) for clients that want one code.

The API is in [docs/api.md](docs/api.md), with the generated OpenAPI document
in [openapi.json](openapi.json).

## Brands and Lightning addresses

A store has terminals, a Lightning address, or both; a person's Lightning
address is a store with no terminals. The owner's wallet picks the store's
brand when it registers it, by its key in the config. One posd serves
several brands, each on its own domain from the `[brands]` config, for
example `pay.example.com` and `pay.example.org`. A store's
Lightning address and pairing links are on its brand's domain, and each
domain has its own names. A brand's operator can reserve names and give
them to verified users, for example when users move from an older service
(see "Operator" in [docs/api.md](docs/api.md)).

## Brands are configuration

The brand also sets the look of the store's terminals: its name, colours,
type, mark, loading animation and install icons. None of it is in the code.
A brand is a `[brands.<key>]` section of the config and a directory of
files, which posd serves and writes into each page. A brand without files
looks like the built-in neutral one. [docs/brands.md](docs/brands.md) says
what a brand directory holds; `contrib/deploy/brands/` has two.

## Languages

The terminal is in English, Spanish and French. It follows the device's
languages, falling back to English, and the staff can pick a language in
Settings, which the device remembers. Amounts and dates are formatted for the
language, with the device's regional variant where it matches (`es-CR`).

The text lives in message catalogs under `web/src/lib/i18n/`: `en.ts` is the
source. To add a language, copy it to `<code>.ts`, translate the values and
register the catalog in `LOCALES` in `index.svelte.ts`. A catalog with a
missing or extra key does not type-check, and `npm test` checks that every
catalog keeps the `{placeholders}` of the English text.

## What posd needs

- The Ark server's public address. Run it in expiry payout mode, so that the
  VTXOs of an owner who stays offline are paid out to them on-chain instead
  of swept (see `docs/expiry-payouts.md` in the repository root).
- A private barkd. It issues the forwarding invoices and claims them to the
  owner's Ark address. It needs no funds.
- A chain source for on-chain sales: a bitcoind, where posd creates a
  watch-only wallet, or an Esplora API.
- A price source for fiat amounts: the Bull Bitcoin price API, any JSON
  price API (`source = "json"` with a URL and a JSON pointer), or a fixed
  price on test networks. It also decides the currencies a store can use.
- A Postgres database for posd.

## Build and run

```sh
just build-pos-web           # builds pos/web/dist, which posd embeds
cargo build --release -p bark-pos
posd --config posd.toml start
```

With nix, `nix build .#bark-pos` builds both. `.gitlab/images/releases/posd/`
has a container image of the binary.

`posd.default.toml` documents every setting. `posd check-config` checks a
config and prints it.

## Owner setup

The owner's wallet registers the store once. It signs the registration
message (`pos/src/registration.rs`, also in `docs/api.md`) with bark's
ordinary message signing (`Wallet::sign_message`, or `POST
/api/v1/message/sign` on barkd). No wallet code is specific to the point of
sale. The registration carries:
- the Ark address that receives the payments;
- the public descriptor of the on-chain account that the host app keeps;
- the store currency;
- the key of the store's brand.

The wallet signs it with the key of the Ark address. The owner then logs in
by signing a challenge, creates pairing links for the terminals, and can
revoke a terminal at any time.

## Tests

```sh
just int-pos      # end-to-end tests; posd runs inside the test process
cargo test -p bark-pos
cd pos/web && npm test && npm run check
```

## Known limits

- **Lightning address:** the Ark server cannot put a description hash in an
  invoice. Wallets that enforce the LUD-06 metadata hash refuse Lightning
  address invoices from posd.
- **Fees:** the owner pays the Ark server's Lightning receive fee on a sale
  paid from outside the Ark, and its intra-Ark receive fee on a sale paid
  from the same Ark server.
- **On-chain reorgs:** a sale is paid at 1 confirmation. A payment whose
  block is replaced leaves the sale `paid` with less in `received_sat`.
