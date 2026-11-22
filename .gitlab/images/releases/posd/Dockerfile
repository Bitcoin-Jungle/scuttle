# posd, the point-of-sale service, with its terminal web app embedded.
#
# Build posd first, with the web app: `just build-pos-web` then
# `cargo build --release -p bark-pos`, or `nix build .#bark-pos`, and copy
# the binary to ./posd-linux-x86_64.
#
# posd needs a Postgres database, the Ark server, a private barkd and a
# chain source of its own; this image runs posd only. Mount the config at
# /etc/posd/posd.toml, or set every value with POSD__ environment variables,
# and brand directories wherever the config's `assets` point.
FROM --platform=linux/amd64 debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    apt-get clean && rm -rf /var/lib/apt/lists/*

RUN useradd --system --home /nonexistent --shell /usr/sbin/nologin posd
COPY ./posd-linux-x86_64 /usr/local/bin/posd
RUN chmod a+x /usr/local/bin/posd

USER posd
ENV POSD_CONFIG=/etc/posd/posd.toml
EXPOSE 3080

CMD ["/usr/local/bin/posd", "start"]
