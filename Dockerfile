# Multi-stage Dockerfile for the cLawyer agent (cloud deployment).
#
# Build:
#   docker build --platform linux/amd64 -t clawyer:latest .
#
# Run:
#   docker run --env-file .env -p 3000:3000 clawyer:latest

# Stage 1: Build
FROM rust:1.92-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev cmake gcc g++ \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add wasm32-wasip2 \
    && cargo install wasm-tools

WORKDIR /app

# Copy manifests first for layer caching
COPY Cargo.toml Cargo.lock ./

# Copy source, build script, tests, and supporting directories
COPY build.rs build.rs
COPY src/ src/
COPY tests/ tests/
COPY migrations/ migrations/
COPY registry/ registry/
COPY channels-src/ channels-src/
COPY wit/ wit/

RUN cargo build --release --bin clawyer

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/clawyer /usr/local/bin/clawyer
COPY --from=builder /app/migrations /app/migrations

# Non-root user
RUN useradd -m -u 1000 -s /bin/bash clawyer
USER clawyer

EXPOSE 3000

ENV RUST_LOG=clawyer=info

ENTRYPOINT ["clawyer"]
