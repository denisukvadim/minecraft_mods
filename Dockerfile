# syntax=docker/dockerfile:1

# ---- build stage -----------------------------------------------------------
FROM rust:1-slim-bookworm AS builder

# `utoipa-swagger-ui` downloads the Swagger UI archive at build time and falls
# back to the system `curl` (its `reqwest` feature is off), so the builder needs
# curl plus CA certificates for HTTPS.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Cache dependency compilation separately from the application sources.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY src ./src
COPY migrations ./migrations

# Touch the entry point so the real sources are always recompiled.
RUN touch src/main.rs && cargo build --release --locked

# ---- runtime stage ---------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 app

WORKDIR /app

COPY --from=builder /build/target/release/minecraftmods_backend /usr/local/bin/minecraftmods_backend

# Uploaded mods and the audit trail live outside the image and survive restarts.
RUN mkdir -p /app/data/mods && chown -R app:app /app/data
VOLUME ["/app/data"]

USER app

ENV HOST=0.0.0.0 \
    PORT=8080 \
    MODS_DIR=/app/data/mods

EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/minecraftmods_backend"]
