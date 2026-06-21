# syntax=docker/dockerfile:1
# ── Build stage ──────────────────────────────────────────────────────────────
# Rust edition 2024 needs >= 1.85; `rust:1-bookworm` tracks the latest 1.x.
FROM rust:1-bookworm AS builder
WORKDIR /app

# Compile against the committed offline query cache (no live DB needed at build).
ENV SQLX_OFFLINE=true

COPY Cargo.toml Cargo.lock ./
COPY .sqlx ./.sqlx
COPY migrations ./migrations
COPY src ./src

RUN cargo build --release && strip target/release/backend

# ── Runtime stage ────────────────────────────────────────────────────────────
# Slim Debian + CA certs + DB client tools. The in-cluster control plane connects
# DIRECTLY to workspace DB services for backup/restore/query/rotation — no pod exec,
# no kubectl. Redis backup/restore is protocol-level (per-key DUMP/RESTORE); SQL/Mongo
# stream via their clients; the Kubernetes API is reached with the kube crate.
FROM debian:bookworm-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      ca-certificates curl gnupg \
      postgresql-client default-mysql-client redis-tools \
 # MongoDB database tools (mongodump/mongorestore) + shell (mongosh).
 && curl -fsSL https://pgp.mongodb.com/server-7.0.asc | gpg --dearmor -o /usr/share/keyrings/mongodb.gpg \
 && echo "deb [signed-by=/usr/share/keyrings/mongodb.gpg] https://repo.mongodb.org/apt/debian bookworm/mongodb-org/7.0 main" > /etc/apt/sources.list.d/mongodb.list \
 && apt-get update \
 && apt-get install -y --no-install-recommends mongodb-database-tools mongodb-mongosh \
 && apt-get purge -y curl gnupg && apt-get autoremove -y \
 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/backend /usr/local/bin/hermes-backend

# Object storage (PVC) and DB backups (PVC) are mounted here at runtime.
RUN mkdir -p /var/www/hermes /var/lib/hermes/backups

ENV PORT=3000
EXPOSE 3000
CMD ["hermes-backend"]
