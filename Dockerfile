# syntax=docker/dockerfile:1.4

# Stage 1: Build Rust app on platform-specific image
FROM --platform=${BUILDPLATFORM} rust:bullseye AS builder

RUN apt-get update && apt-get install -y \
    build-essential bash pkg-config libssl-dev libpq-dev curl git \
    --no-install-recommends && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app

COPY Cargo.toml Cargo.lock ./
COPY ./entity/Cargo.toml ./entity/Cargo.toml
COPY ./entity_api/Cargo.toml ./entity_api/Cargo.toml
COPY ./migration/Cargo.toml ./migration/Cargo.toml
COPY ./service/Cargo.toml ./service/Cargo.toml
COPY ./web/Cargo.toml ./web/Cargo.toml
COPY . .

RUN cargo build --release --workspace

# Stage 2: Minimal runtime image using non-root user
FROM debian:bullseye-slim

RUN apt-get update && apt-get install -y bash && rm -rf /var/lib/apt/lists/*

RUN useradd -m -s /bin/bash appuser
WORKDIR /app

COPY --from=builder /usr/src/app/target/release/refactor_platform_rs .
COPY --from=builder /usr/src/app/target/release/migration .
COPY --from=builder /usr/src/app/target/release/seed_db .

COPY entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh && chown -R appuser:appuser /app /entrypoint.sh

USER appuser

EXPOSE 8000

ENTRYPOINT ["/entrypoint.sh"]