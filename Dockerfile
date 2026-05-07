FROM rust:1.88-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked --bin sifs

FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive \
    NODE_MAJOR=22 \
    MCP_PROXY_VERSION=6.4.3 \
    SIFS_CACHE_DIR=/data/sifs-cache

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl git gnupg \
    && mkdir -p /etc/apt/keyrings \
    && curl -fsSL https://deb.nodesource.com/gpgkey/nodesource-repo.gpg.key \
        | gpg --dearmor -o /etc/apt/keyrings/nodesource.gpg \
    && echo "deb [signed-by=/etc/apt/keyrings/nodesource.gpg] https://deb.nodesource.com/node_${NODE_MAJOR}.x nodistro main" \
        > /etc/apt/sources.list.d/nodesource.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends nodejs \
    && npm install -g "mcp-proxy@${MCP_PROXY_VERSION}" \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/*

COPY --from=builder /app/target/release/sifs /usr/local/bin/sifs

RUN mkdir -p /data/sifs-cache \
    && sifs --version

WORKDIR /workspace

CMD ["mcp-proxy", "--", "sifs", "mcp", "--cache-dir", "/data/sifs-cache"]
