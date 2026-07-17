# `ci.yml` — continuous integration

## Purpose

The GitHub Actions workflow that gates every push to `main` / `claude/**` and every PR into `main`.
It runs the full workspace check, the real-Postgres integration suite, lint, and a dependency audit.
Green CI is the merge bar for this repo.

## When it runs

- **push** to `main` and any `claude/**` branch.
- **pull_request** targeting `main`.

## The `test` job

Runs on `ubuntu-latest` with a **real PostgreSQL 16 service container** (not a mock): the
integration tests use `sqlx::test`, which creates one throwaway database per test from
`DATABASE_URL`. The service exposes `5432:5432` and is gated on `pg_isready` health checks before
the steps run.

| Step | What it enforces |
|------|------------------|
| `./scripts/check-docs.sh` | every `.rs` has a companion `.md`, and every companion carries a `## Graph context` section (the two-layer navigation model; see README "Navigating this repo") |
| `cargo fmt --check --all` | formatting is committed |
| `cargo check --workspace` | the workspace compiles |
| `cargo test --workspace` | unit + integration tests pass (against the PG service) |
| `cargo clippy --workspace --all-targets -- -D warnings` | zero clippy warnings |
| `cargo audit` | no known-vulnerable dependencies |

Caching is via `Swatinem/rust-cache@v2`; the toolchain is stable with `clippy` + `rustfmt`.

## Notes & gotchas

- `DATABASE_URL` here points at the service container's superuser (`keeplin:keeplin`); `sqlx::test`
  needs create-database rights, which that role has.
- `-D warnings` makes clippy findings **fail** the build — treat a clippy note as a required fix.
- `cargo audit` is installed per-run (`--locked`); a new advisory can turn CI red without any code
  change, which is intended (it surfaces a dependency to bump).

## Related files

- `../../crates/keeplin-srv/tests/integration.md` / `collab.md` — the suites this runs.
- `../../docker-compose.yml.md` — the equivalent Postgres for local runs.
- `../../.env.example.md` — the `DATABASE_URL` shape mirrored here.
