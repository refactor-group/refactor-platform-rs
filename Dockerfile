# syntax=docker/dockerfile:1.4

# ┌───────────────────────────────────────────┐
# │ 0) Plan & cache your workspace with Chef │
# └───────────────────────────────────────────┘

# Stage "chef-plan": generate a recipe.json of all your crates + versions
FROM ghcr.io/lukemathwalker/cargo-chef:latest AS chef-plan
WORKDIR /usr/src/app

# copy only manifests first, to leverage layer caching
COPY Cargo.toml Cargo.lock ./
COPY ./entity/Cargo.toml ./entity/Cargo.toml
COPY ./entity_api/Cargo.toml ./entity_api/Cargo.toml
COPY ./migration/Cargo.toml ./migration/Cargo.toml
COPY ./service/Cargo.toml ./service/Cargo.toml
COPY ./web/Cargo.toml ./web/Cargo.toml

# create recipe.json
RUN cargo chef prepare --recipe-path recipe.json

# Stage "chef-cook": fetch & compile all your deps (no sources yet)
FROM ghcr.io/lukemathwalker/cargo-chef:latest AS chef-cook
WORKDIR /usr/src/app
COPY --from=chef-plan /usr/src/app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# ┌───────────────────────────────────────────┐
# │ 1) Builder on Tonistiigi's multi-arch rs │
# └───────────────────────────────────────────┘

FROM --platform=${BUILDPLATFORM} tonistiigi/rs:debian AS builder

# Declare the GitHub Actions build-args so they're no longer "unknown"
ARG BUILDKIT_INLINE_CACHE
ARG CARGO_INCREMENTAL
ARG RUSTFLAGS

# (Optional) Expose the buildkit platform vars, if you want to use them
ARG TARGETPLATFORM
ARG BUILDPLATFORM

# Export into the container environment for any RUN/cargo commands
ENV CARGO_INCREMENTAL=${CARGO_INCREMENTAL} \
    RUSTFLAGS=${RUSTFLAGS} \
    TARGETPLATFORM=${TARGETPLATFORM} \
    BUILDPLATFORM=${BUILDPLATFORM}


# install libs for Sea-ORM, etc.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
    build-essential bash pkg-config libssl-dev libpq-dev curl git \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

# reuse compiled deps from chef-cook
COPY --from=chef-cook /usr/src/app/target target
# bring in recipe in case you need cargo-chef metadata
COPY --from=chef-plan /usr/src/app/recipe.json recipe.json

# copy all your source
COPY . .

# final build (only your code, deps are cached!)
RUN cargo build --release -p refactor_platform_rs -p migration

# debug listing (optional)
RUN echo "LIST OF CONTENTS" && ls -lahR /usr/src/app
# ┌───────────────────────────────────────────┐
# │ 2) Runtime — your existing Debian slim   │
# └───────────────────────────────────────────┘

FROM --platform=${BUILDPLATFORM} debian:bullseye-slim AS runtime

# Install Bash to support entrypoint.sh
RUN apt-get update && apt-get install -y bash && rm -rf /var/lib/apt/lists/*

# non-root user ensuring user/group IDs match the host
# (this is important for file permissions, e.g. when using volumes)
RUN useradd -m -u 1001 -s /bin/bash appuser
WORKDIR /app
RUN chown appuser:appuser /app

# binaries
COPY --from=builder /usr/src/app/target/release/refactor_platform_rs .
COPY --from=builder /usr/src/app/target/release/migration ./migrationctl

# migrations SQL
RUN mkdir -p /app/migration/src
COPY --from=builder /usr/src/app/migration/src/refactor_platform_rs.sql /app/migration/src/

# entrypoint
COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh \
    && chown -R appuser:appuser /app /entrypoint.sh

USER appuser
EXPOSE 4000
ENTRYPOINT ["/entrypoint.sh"]
