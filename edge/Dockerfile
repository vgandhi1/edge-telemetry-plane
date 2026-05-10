FROM rust:1-bookworm AS builder
WORKDIR /build
COPY proto /build/proto
COPY edge /build/edge
WORKDIR /build/edge
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/edge/target/release/edge-gateway \
     /build/edge/target/release/edge-sync \
     /usr/local/bin/
