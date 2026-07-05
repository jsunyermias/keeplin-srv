# `docker-compose.yml` — Postgres + server stack

## Purpose

A Compose file for local/demo runs: a **PostgreSQL** service the server (and the test suite) needs,
plus an optional **server** service that builds the `Dockerfile` and runs against that Postgres. It
is **not** a production deployment descriptor — credentials and `JWT_SECRET` are dev defaults.

## What it defines

| Item | Value |
|------|-------|
| service `postgres` | `postgres:16-alpine`, container `keeplin-srv-postgres` |
| credentials | `keeplin` / `keeplin`, database `keeplin` |
| port | `5432:5432` (matches `.env.example`'s `DATABASE_URL`) |
| volume | `keeplin-srv-pgdata` → `/var/lib/postgresql/data` (data survives `down`) |
| healthcheck | `pg_isready -U keeplin -d keeplin` every 5s |
| service `server` | built from `Dockerfile`; waits for Postgres health; `3000:3000`; `DATABASE_URL` points at the `postgres` service |

## Usage

```
# Just the database (for `cargo run` / `cargo test` on the host):
docker compose up -d postgres
cargo test --workspace        # sqlx::test spins throwaway DBs inside this instance

# The whole stack (build + run the server too):
docker compose up --build

docker compose down           # stop (data kept in the named volume)
docker compose down -v        # stop and wipe the volume
```

## Notes & gotchas

- **Dev/demo only.** For production, run a managed/hardened Postgres with real credentials, TLS and
  backups, override `JWT_SECRET`, and put a TLS reverse proxy in front — see the README checklist.
  Do not ship these defaults.
- The `server` service's `DATABASE_URL` uses the `postgres` service hostname, not `localhost`.
- `sqlx::test` needs create-database rights; the `keeplin` superuser here has them.
- The volume is named, so `up`/`down` cycles keep data; use `down -v` to reset.

## Related files

- `Dockerfile.md` — the image the `server` service builds.
- `.env.example.md` — the `DATABASE_URL` that points at this instance.
- `.github/workflows/ci.yml.md` — the CI service container mirroring this setup.
- `crates/keeplin-srv/src/main.md` — how the server connects and pools.
