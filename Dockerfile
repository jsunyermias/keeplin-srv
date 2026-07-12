# syntax=docker/dockerfile:1

# ── Builder ──────────────────────────────────────────────────────────────────
# Compiles a release binary. keeplin-core (a git dependency) builds bundled
# SQLite via libsql and generates bindings, so the build needs a C toolchain
# plus clang/libclang (bindgen) and cmake.
FROM rust:1-bookworm AS builder
RUN apt-get update \
    && apt-get install -y --no-install-recommends clang cmake pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY . .
# The keeplin-core git dependency (pinned by rev in Cargo.toml) is fetched here.
RUN cargo build --release -p keeplin-srv

# ── Runtime ──────────────────────────────────────────────────────────────────
# The database migrations are embedded into the binary at compile time
# (sqlx::migrate!), so the runtime image needs only the binary and CA roots.
FROM debian:bookworm-slim AS runtime
# `curl` is included only for the container HEALTHCHECK below.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home keeplin
COPY --from=builder /build/target/release/keeplin-srv /usr/local/bin/keeplin-srv
USER keeplin
ENV PORT=3000
EXPOSE 3000
# Readiness probe: `/ready` does a DB round-trip and returns 503 if Postgres is unreachable,
# so an orchestrator does not route traffic to an instance that can only error (issue #36).
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS "http://localhost:${PORT}/ready" || exit 1
ENTRYPOINT ["/usr/local/bin/keeplin-srv"]
