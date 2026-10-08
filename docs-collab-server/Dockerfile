# syntax=docker/dockerfile:1.4

# Stage 1: prepare dependency recipe
# Pinned by digest so a new Rust release cannot silently invalidate the layer
# cache; Dependabot opens a PR when the tag moves.
FROM lukemathwalker/cargo-chef:latest-rust-bookworm@sha256:2ee6e8edf0b91b5a7295071299596601710bccb243bd9735e0b9abf6e114582b AS chef
WORKDIR /usr/src/app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Stage 2: build dependencies (cached layer) then the collab binary
FROM chef AS builder
RUN apt-get update && apt-get install -y \
    build-essential bash pkg-config libssl-dev libpq-dev curl git \
    --no-install-recommends && rm -rf /var/lib/apt/lists/*
COPY --from=planner /usr/src/app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release -p docs-collab-server

# Stage 3: minimal runtime image
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates curl \
    --no-install-recommends && rm -rf /var/lib/apt/lists/*
RUN useradd -m -u 1001 -s /bin/bash appuser
WORKDIR /app
COPY --from=builder /usr/src/app/target/release/docs-collab-server /app/docs-collab-server
RUN chown -R appuser:appuser /app
USER appuser
EXPOSE 1234
ENTRYPOINT ["/app/docs-collab-server"]
