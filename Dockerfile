# syntax=docker/dockerfile:1.4

# Build-args for your cross toolchain image & Rust target
ARG BASE_IMAGE=ghcr.io/rust-cross/rust-musl-cross:x86_64-musl
ARG TARGET_TRIPLE=x86_64-unknown-linux-musl

############################################################
# 0) Base: musl cross toolchain + pkg-config & OpenSSL dev #
#           + cargo-chef for dependency caching            #
############################################################

FROM ${BASE_IMAGE} AS chef-base

# Install system dependencies with retry logic
RUN apt-get update && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Install cargo-chef with memory-conscious settings
ENV CARGO_NET_RETRY=10 \
    CARGO_NET_GIT_FETCH_WITH_CLI=true \
    CARGO_HTTP_TIMEOUT=300

RUN cargo install cargo-chef --locked

WORKDIR /usr/src/app

############################################################
# 1) Planner: generate recipe.json of your workspace deps  #
############################################################

FROM chef-base AS chef-plan

COPY Cargo.toml Cargo.lock ./
COPY src ./src

COPY entity/Cargo.toml     entity/Cargo.toml
COPY entity/src            entity/src
COPY entity_api/Cargo.toml entity_api/Cargo.toml
COPY entity_api/src        entity_api/src
COPY migration/Cargo.toml  migration/Cargo.toml
COPY migration/src         migration/src
COPY service/Cargo.toml    service/Cargo.toml
COPY service/src           service/src
COPY web/Cargo.toml        web/Cargo.toml
COPY web/src               web/src
COPY domain/Cargo.toml     domain/Cargo.toml
COPY domain/src            domain/src

RUN cargo chef prepare --recipe-path recipe.json

############################################################
# 2) Cooker: compile all dependencies for musl target      #
############################################################

FROM chef-base AS chef-cook
ARG TARGET_TRIPLE

# Memory-conscious environment settings
ENV CARGO_BUILD_JOBS=1 \
    CARGO_NET_RETRY=10

COPY --from=chef-plan /usr/src/app/recipe.json recipe.json
COPY Cargo.toml Cargo.lock ./
COPY entity/Cargo.toml     entity/Cargo.toml
COPY entity_api/Cargo.toml entity_api/Cargo.toml
COPY migration/Cargo.toml  migration/Cargo.toml
COPY service/Cargo.toml    service/Cargo.toml
COPY web/Cargo.toml        web/Cargo.toml
COPY domain/Cargo.toml     domain/Cargo.toml

RUN cargo chef cook --release \
    --target ${TARGET_TRIPLE} \
    --recipe-path recipe.json

############################################################
# 3) Builder: compile only your binaries                   #
############################################################

FROM chef-base AS builder
ARG TARGET_TRIPLE
ARG BUILDKIT_INLINE_CACHE
ARG CARGO_INCREMENTAL
ARG RUSTFLAGS

ENV CARGO_INCREMENTAL=${CARGO_INCREMENTAL:-0} \
    CARGO_BUILD_JOBS=1 \
    RUSTFLAGS=${RUSTFLAGS}

WORKDIR /usr/src/app
COPY --from=chef-cook /usr/src/app/target target
COPY --from=chef-plan /usr/src/app/recipe.json recipe.json

COPY Cargo.toml Cargo.lock ./
COPY src        ./src
COPY entity     ./entity
COPY entity_api ./entity_api
COPY migration  ./migration
COPY service    ./service
COPY web        ./web
COPY domain     ./domain

RUN cargo build --release \
    --target ${TARGET_TRIPLE} \
    -p refactor_platform_rs \
    -p migration

############################################################
# 4) Runtime: minimal Debian-slim with bash & non-root     #
############################################################

FROM debian:bullseye-slim AS runtime
ARG TARGET_TRIPLE

LABEL \
    org.opencontainers.image.title="Refactor Platform RS" \
    org.opencontainers.image.description="A Sea-ORM-powered Rust workspace (multi-arch, cached builds)" \
    org.opencontainers.image.source="https://github.com/refactor-group/refactor-platform-rs" \
    org.opencontainers.image.licenses="GPL-3.0-only" \
    org.opencontainers.image.authors="Levi McDonough <levimcdonough@gmail.com>"

RUN apt-get update \
    && apt-get install -y --no-install-recommends bash \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -u 1001 -s /bin/bash appuser

WORKDIR /app
RUN chown appuser:appuser /app

COPY --from=builder /usr/src/app/target/${TARGET_TRIPLE}/release/refactor_platform_rs .
COPY --from=builder /usr/src/app/target/${TARGET_TRIPLE}/release/migration ./migrationctl

RUN mkdir -p /app/migration/src
COPY --from=builder /usr/src/app/migration/src/refactor_platform_rs.sql /app/migration/src/

COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh \
    && chown -R appuser:appuser /app /entrypoint.sh

USER appuser
EXPOSE 4000
ENTRYPOINT ["/entrypoint.sh"]