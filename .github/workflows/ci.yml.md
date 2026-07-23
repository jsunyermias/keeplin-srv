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
| `./scripts/check-docs.sh` | every `.rs` has a companion `.md` in the block-complete format: `## Graph context`, every `// md:` marker mirrored + unique, `## Coverage checklist` rows == marker count, no fence elision, no non-marker `.rs` comments (the two-layer navigation model; see README "Navigating this repo") |
| `cargo fmt --check --all` | formatting is committed |
| `cargo test --workspace` | unit + integration tests pass (against the PG service) |
| `cargo clippy --workspace --all-targets -- -D warnings` | zero clippy warnings (`--all-targets` also subsumes `cargo check`, so no separate check step) |
| `cargo audit` | no known-vulnerable dependencies (the tool is installed as a prebuilt binary via `taiki-e/install-action@v2`, not compiled from source) |

Caching is via `Swatinem/rust-cache@v2`; the toolchain is stable with `clippy` + `rustfmt`.

## The `graph` job

Runs on `ubuntu-latest` in parallel with `test` (no Rust toolchain or Postgres needed).
Enforces LAYER 1 of the navigation model: the committed `graphify-out/graph.json` must match
the code.

| Step | What it enforces |
|------|------------------|
| `actions/setup-python@v5` (`3.12`) + `pip install "graphifyy==0.9.25"` | the pinned graphify is available so extraction matches the version the committed graph was built with |
| `./scripts/check-graph.sh` (env `GRAPHIFY_REQUIRED=1`) | re-runs `graphify update .` and fails if the committed graph's code structure is stale; `GRAPHIFY_REQUIRED=1` turns a missing install into a hard failure rather than a silent skip |

## Notes & gotchas

- `DATABASE_URL` here points at the service container's superuser (`keeplin:keeplin`); `sqlx::test`
  needs create-database rights, which that role has.
- `-D warnings` makes clippy findings **fail** the build — treat a clippy note as a required fix.
- `cargo audit` is installed each run as a **prebuilt binary** (`taiki-e/install-action@v2`)
  rather than compiled from source, which keeps the step fast; a new advisory can turn CI red
  without any code change, which is intended (it surfaces a dependency to bump).

## Related files

- `../../crates/keeplin-srv/tests/integration.md` / `collab.md` — the suites this runs.
- `../../docker-compose.yml.md` — the equivalent Postgres for local runs.
- `../../.env.example.md` — the `DATABASE_URL` shape mirrored here.
