# `docker-compose.yml` — local PostgreSQL

## Purpose

A one-service Compose file that brings up the PostgreSQL the server (and the test suite) needs for
local development. It is **not** a production deployment descriptor — it runs only the database, with
default credentials and a named volume, so you can `cargo run` / `cargo test` against it.

## What it defines

| Item | Value |
|------|-------|
| service | `postgres` (`postgres:16-alpine`), container `keeplin-srv-postgres` |
| credentials | `keeplin` / `keeplin`, database `keeplin` |
| port | `5432:5432` (matches `.env.example`'s `DATABASE_URL`) |
| volume | `keeplin-srv-pgdata` → `/var/lib/postgresql/data` (data survives `down`) |
| healthcheck | `pg_isready -U keeplin -d keeplin` every 5s |

## Usage

```
docker compose up -d          # start Postgres
cargo test --workspace        # sqlx::test spins throwaway DBs inside this instance
docker compose down           # stop (data kept in the named volume)
docker compose down -v        # stop and wipe the volume
```

## Notes & gotchas

- **Development only.** For production, run a managed/hardened Postgres with real credentials,
  TLS, and backups — see the README's operator checklist. Do not ship these defaults.
- `sqlx::test` needs create-database rights; the `keeplin` superuser here has them.
- The volume is named, so `up`/`down` cycles keep data; use `down -v` to reset.

## Related files

- `.env.example.md` — the `DATABASE_URL` that points at this instance.
- `.github/workflows/ci.yml.md` — the CI service container mirroring this setup.
- `crates/keeplin-srv/src/main.md` — how the server connects and pools.
