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
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --no-create-home keeplin
COPY --from=builder /build/target/release/keeplin-srv /usr/local/bin/keeplin-srv
USER keeplin
ENV PORT=3000
EXPOSE 3000
ENTRYPOINT ["/usr/local/bin/keeplin-srv"]
