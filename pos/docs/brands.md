# Brands

A brand is how a store's terminals look and where its Lightning address
lives. posd has none built in: each is a `[brands.<key>]` section of the
config and, optionally, a directory of files.

```toml
[brands.example]
name = "Example"                   # "Open Example on your phone"
app_name = "Example POS"           # the installed app; the default is "<name> POS"
public_url = "https://pay.example.com"
theme_color = "#edece1"            # browser chrome and install splash, light mode
theme_color_dark = "#0e0e0e"       # and dark mode
assets = "/etc/posd/brands/example"
admin_token = "…"                  # the brand's operator API, optional
```

The key is 1 to 32 lowercase letters, digits and `-`. `default` is the
built-in neutral brand's. Owners pick the key when they register a store.

## The asset directory

Every file is optional. posd serves each at `/brands/<key>/<file>`, and the
neutral brand's file of that name for a file the directory does not have.

| File | What it is |
|---|---|
| `brand.css` | Sets the terminal's tokens on `:root` |
| `mark.svg` or `mark.png` | The brand's mark, in the header, on the pairing page and in the loading ring |
| `spinner.svg` | A loading animation, in place of the ring around the mark |
| `icon-192.png`, `icon-512.png` | The install icons, full bleed |
| `*.woff2`, `*.woff` | Fonts `brand.css` loads, by relative URL |

posd serves no other file.

## brand.css

posd links `brand.css` after the terminal's own stylesheet, so it sets the
`--pos-*` tokens the terminal styles itself with. The neutral values, and the
full list, are at the top of `web/src/lib/app.css`. A brand sets the ones it
changes, in light mode on `:root` and in dark mode in a
`@media (prefers-color-scheme: dark)` block.

| Tokens | For |
|---|---|
| `--pos-page`, `--pos-surface`, `--pos-muted`, `--pos-line` | Backgrounds and rules |
| `--pos-ink`, `--pos-ink-soft` | Text |
| `--pos-accent`, `--pos-on-accent` | The main button, links and the loading ring |
| `--pos-ok`, `--pos-warn`, `--pos-err` and their `-bg` | States |
| `--pos-paid`, `--pos-paid-ink` | The paid screen, white on `--pos-paid` |
| `--pos-confetti` | The paid screen's confetti, a comma-separated list |
| `--pos-font-body`, `--pos-font-display`, `--pos-display-weight`, `--pos-display-boost` | Type; a condensed display face sets the boost above 1 |
| `--pos-case` | `uppercase` for headings and buttons in capitals |
| `--pos-tracking-display`, `--pos-tracking-label`, `--pos-tracking-button` | Letter spacing |
| `--pos-radius-md`, `--pos-radius-lg` | Corners |

The mark has the class `brand-mark`, so a brand can shape it, for example
round a square image with `.brand-mark { border-radius: 9999px; }`.

## Which brand a page is in

- A pairing page, `/pair/<code>`, and its install manifest are in the brand
  of the code's store.
- Every other page, and `/manifest.webmanifest`, is in the brand of the
  domain it was asked for. On a domain of no brand, the neutral brand,
  unless the terminal remembers the brand of its store.
- A paired terminal changes to its store's brand when the store's is
  another, and remembers it.
