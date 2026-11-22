# Point-of-sale brands

The brand bundles of this deployment's posd. Each directory is a brand's
`assets` in the posd config:

```toml
[brands.bitcoinjungle]
name = "Bitcoin Jungle"
public_url = "https://pay.bitcoinjungle.app"
theme_color = "#edece1"
theme_color_dark = "#0e0e0e"
assets = "/etc/posd/brands/bitcoinjungle"

[brands.bullbitcoin]
name = "Bull Bitcoin"
public_url = "https://pay.bull-wallet.com"
theme_color = "#f5f0e8"
theme_color_dark = "#161512"
assets = "/etc/posd/brands/bullbitcoin"
```

What a bundle can hold is in `pos/docs/brands.md`.
