# The edge

nginx in front of the services we run publicly: posd and captaind's public
API. One template (`render.py`) for every environment, and an environment
file per deployment (`regtest.toml`; production gets its own). Liquid, Boltz,
explorers and the other test helpers of the regtest network are not ours to
run in production and do not go through the edge.

## What it does

posd (both brands):
- **WAF:** ModSecurity with the OWASP Core Rule Set 3.3 (Debian's
  `modsecurity-crs`), blocking, paranoia level 1, anomaly threshold 5.
  `modsec/exclusions.conf` lifts only what posd's own fields need:
  - SQL and command-injection rules don't apply to a sale's memo or a payer's
    comment;
  - the remote-file-inclusion rules don't apply to a Bolt Card's link.
- **Rate limits, per client:**

  | Route | Limit |
  |---|---|
  | General | 20/s, burst 60 |
  | Sign-in, pairing and store registration | 30/min, burst 10 |
  | LNURL callbacks (each makes an invoice) | 60/min, burst 20 |

- **The operator API** (`/api/v1/admin/`) is reachable only from `admin_allow`,
  on top of posd's token.
- **Errors in the shape clients parse:** posd's `{"error": …}`, and LNURL's
  `{"status":"ERROR"}` with CORS on the LNURL routes.
- Bodies are capped at 256 KB.

captaind (gRPC and gRPC-Web):
- **An allowlist of its public services:** `ArkService`, `MailboxService`,
  health. Anything else, such as admin or integration services, or junk, is
  answered at the edge.
- **Rate limits, per client:**
  - 30/s, burst 120, overall;
  - 2/s, burst 20, on the methods that make captaind work per call (Lightning
    receive, board and arkoor cosigns, offboard, xpub recovery).
- **Body caps per method:**
  - 16 KB for fixed-size calls (status checks, invoices, subscriptions; real
    ones are under 1 KB);
  - 4 MB, captaind's own gRPC message limit, for the methods listed in
    `ARK_LARGE`, whose requests grow with the wallet (VTXO lists,
    transactions, signatures).
  - A method captaind adds later gets the small cap until it is listed, so
    it fails loudly ("request too large") rather than silently getting the
    big allowance.
- **HTTP/2 streams per connection:** 32. A wallet holds two open and makes a
  few calls at a time. On regtest it is 256, because cloudflared carries
  every client over a few connections.
- **Errors as gRPC statuses**, which clients read: 8 rate limited or too
  large, 12 unknown method, 14 unavailable.
- **No WAF:** its rules cannot read protobuf.

Everywhere:
- TLS 1.2+ with modern ciphers and no session tickets.
- HSTS, `nosniff`, and no `Server` header.
- Tight header and body timeouts, and connection caps per client.
- JSON access logs in `/var/log/nginx/edge-access.log`, with the request id,
  upstream timing, the limit outcome and the gRPC status.
- WAF blocks in `/var/log/nginx/edge-waf.log`.
- Unknown host names are refused at the TLS handshake.
- `limit_exempt` lists clients that skip the rate limits (operators, load
  tests). The WAF still applies to them.

## Regtest

`contrib/deploy/regtest/host.sh` installs it.
- **Listening:** on `127.0.0.1:9443`, behind cloudflared.
- **Certificate:** from a CA of the deployment, which cloudflared checks
  (`caPool`, `originServerName`).
- **Client address:** taken from `CF-Connecting-IP`, trusted from loopback
  only.

## Production checklist

The same template, with an environment file of its own.

1. **Listening and client address:** listen on the public address (`listen =
   "0.0.0.0:443"`). Drop `real_ip_from` and `real_ip_header`: clients connect
   directly.
2. **Certificate:** a public certificate (certbot) for `tls_cert`/`tls_key`,
   renewed by its timer. Add a server on port 80 that only redirects and
   answers ACME challenges.
3. **Upstreams:** the services' Tailscale addresses. Check with `tailscale
   ping` that the paths are direct (not relayed), and restrict the edge to
   exactly these ports in the Tailscale ACLs.
4. **Lists:** `admin_allow` is the tailnet; `limit_exempt` is the tailnet
   only.
5. **Firewall** (nftables): 80 and 443 in, plus Tailscale's UDP port. SSH
   only over Tailscale.
6. **Lightning:** CLN's peer port (9735) and, if published, Electrum go
   through an nginx `stream` block to their hosts over Tailscale, with
   `limit_conn`. They are not HTTP and get no WAF.
7. **Unattended security updates.** The edge holds no secrets beyond its TLS
   key.
8. **Volumetric DDoS:** nothing on the VM stops a flood of the link. That
   needs the provider's protection or is an accepted risk.

Tune the limits from `edge-access.log` before launch. Many customers behind
one carrier NAT share an address, which per-client limits count together.
