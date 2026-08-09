# SCC daemon image (docs/DEPLOYMENT_AND_INFRA.md §3): read-only repo mount
# at /repo, writable state at /data (mount an SCC volume).
FROM rust:1.97-bookworm AS builder
WORKDIR /build
COPY crates crates
COPY Cargo.toml Cargo.lock ./
RUN cargo build --release -p scc-cli

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/scc /usr/local/bin/scc
VOLUME ["/data"]
ENV SCC_STATE_DIR=/data
WORKDIR /repo
EXPOSE 7777
ENTRYPOINT ["scc"]
CMD ["serve"]
