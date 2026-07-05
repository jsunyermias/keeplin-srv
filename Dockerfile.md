# `Dockerfile` — reproducible server image

## Purpose

A multi-stage build that produces a small, self-contained runtime image for keeplin-srv. It exists
so the server can be deployed reproducibly (the same binary everywhere) instead of being built ad-hoc
on a host. Pairs with the pinned `keeplin-core` git dependency (`crates/keeplin-srv/Cargo.toml`),
which fixes exactly which upstream commit the image is built from.

## Stages

| Stage | Base | What it does |
|-------|------|--------------|
| `builder` | `rust:1-bookworm` | installs the C/bindgen toolchain, copies the workspace, `cargo build --release -p keeplin-srv` |
| `runtime` | `debian:bookworm-slim` | installs CA roots, creates an unprivileged user, copies just the binary |

## Why these build dependencies

`keeplin-core` pulls in **libsql** (bundled SQLite), which compiles C and generates bindings, so the
builder needs `clang`/`libclang` (for bindgen) and `cmake` on top of the Rust image's own C compiler.
These are build-only; the runtime image does not carry them.

## Why the runtime image is tiny

- The database **migrations are embedded into the binary at compile time** (`sqlx::migrate!`), so the
  runtime image ships no migration files — the binary applies them itself at startup.
- sqlx uses `runtime-tokio-rustls`, so there is no OpenSSL runtime dependency; only `ca-certificates`
  is installed (for any outbound TLS).
- The release profile sets `strip = true` and `lto = true`, so the binary is small and standalone.

## Runtime contract

- Runs as the unprivileged `keeplin` user (uid 10001), never root.
- Listens on `PORT` (default `3000`), which is `EXPOSE`d.
- Reads all configuration from environment variables (see `.env.example` / `config.md`);
  `DATABASE_URL` and `JWT_SECRET` are required.

## Usage

```
docker build -t keeplin-srv .
docker run --rm -p 3000:3000 \
  -e DATABASE_URL=postgres://user:pass@host:5432/keeplin \
  -e JWT_SECRET=$(openssl rand -hex 32) \
  keeplin-srv
```

Or the whole stack (Postgres + server) via `docker compose up --build` (see `docker-compose.md`).

## Notes & gotchas

- `rust:1-bookworm` tracks the latest stable Rust; pin it to a specific version if you need
  bit-for-bit reproducibility of the toolchain too.
- Put a TLS-terminating reverse proxy in front in production and leave `RATE_LIMIT_PER_MIN=0`
  (the in-process limiter keys on the peer IP — see `ratelimit.md`).
- The build fetches the pinned `keeplin-core` commit over the network; a fully air-gapped build would
  need a vendored copy.

## Related files

- `crates/keeplin-srv/Cargo.toml` — the pinned `keeplin-core` `rev` the image is built from.
- `docker-compose.md` — the Postgres + server stack for local/demo runs.
- `.env.example.md` — the environment variables the container reads.
