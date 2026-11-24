# scuttle's servers and wallet daemon in one image, built from source:
# captaind and watchmand (the Ark server, with expiry payouts), barkd and
# bark (scuttle's wallet; upstream bark cannot use a payout-mode server) and
# posd (point of sale and Lightning addresses, its terminal app embedded).
# Each service picks its binary as the command. The brands' assets for posd
# are in /usr/share/scuttle/brands.
#
# Build from the repository root:
#
#   podman build -f contrib/docker/Dockerfile -t scuttle .
#
# BULL's regtest stack (tools/regtest in bullbitcoin-mobile) builds it from a
# pinned commit. Base images are pinned by digest.

FROM docker.io/library/node:24-trixie-slim@sha256:4d00acb4bcc42c454c4eb801526f7ad7f772ea3bd4fd168c5cc2e384abcbb9d4 AS web
WORKDIR /src/pos/web
COPY pos/web/package.json pos/web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY pos/web/ ./
RUN npm run build

FROM docker.io/library/rust:1.94-trixie@sha256:652612f07bfbbdfa3af34761c1e435094c00dde4a98036132fca28c7bb2b165c AS build
RUN apt-get update \
	&& apt-get install -y --no-install-recommends protobuf-compiler clang libclang-dev \
	&& rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
COPY --from=web /src/pos/web/dist /src/pos/web/dist
# Thin LTO: most of full LTO's gain at a fraction of the link time.
ENV CARGO_PROFILE_RELEASE_LTO=thin POS_WEB_DIST=/src/pos/web/dist
RUN cargo build --release --locked -p bark-server -p bark-cli -p bark-pos \
	&& mkdir /out \
	&& cp target/release/captaind target/release/watchmand target/release/barkd \
		target/release/bark target/release/posd /out/

FROM docker.io/library/debian:trixie-slim@sha256:8aa5a7bf5a1955bddbe7d74fcdd0b91ab30e74362a8b73af672b0481b35b44a0
RUN apt-get update \
	&& apt-get install -y --no-install-recommends ca-certificates curl \
	&& rm -rf /var/lib/apt/lists/*
COPY --from=build /out/ /usr/local/bin/
COPY contrib/deploy/brands /usr/share/scuttle/brands
EXPOSE 3535 3000 3080
