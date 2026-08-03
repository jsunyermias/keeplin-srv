# `ci.yml` — continuous integration

## Purpose

The GitHub Actions workflow that gates every push to `main` / `claude/**` and every PR into
`main`. It enforces pull-request review governance, runs the full workspace check, the
real-Postgres integration suite, lint, and a dependency audit. Green CI is the merge bar
for this repo.

## When it runs

- **push** to `main` and any `claude/**` branch.
- **pull_request** targeting `main` on `opened`, `synchronize`, `reopened`, `edited`, and
  `ready_for_review`.

The workflow has read-only access to repository contents, pull-request metadata and check
runs. The `checks: read` scope exists for the review-loop convergence step, which reads the
head commit's check runs to build the set of red required checks; without it that set would
read as empty and a failing pull request could be declared converged. Body edits retrigger the
workflow, so completing or removing review evidence — and editing the review ledger — is
reflected in the required check without a new commit.

## The `test` job

Runs on `ubuntu-latest` with a **real PostgreSQL 16 service container** (not a mock): the
integration tests use `sqlx::test`, which creates one throwaway database per test from
`DATABASE_URL`. The service exposes `5432:5432` and is gated on `pg_isready` health checks before
the steps run.

| Step | What it enforces |
|------|------------------|
| `node --test` over `check-review-governance.test.js` and `check-review-loop.test.js` | the reviewed and maintainer-waiver paths, and the convergence, recurrence, advisory and stagnation paths, including negative cases |
| `actions/github-script@v7` (non-draft pull requests only) | either an independent review with evidence, or a complete maintainer waiver whose exact PR is recorded in the changed `docs/review-debt.md` |
| `actions/setup-python@v5` (`3.12`) | the standard-library runtime used by deterministic companion verification |
| `./scripts/check-docs.sh` | every `.rs` has a structurally valid companion, every Rust fence is exactly faithful to source, and the generated context manifest is current |
| `python3 -m unittest discover -s scripts/tests -p 'test_*.py'` | syntax fixtures, drift/error detection, fence-only sync and reproducible context packs |
| `cargo fmt --check --all` | formatting is committed |
| `cargo test --workspace` | unit + integration tests pass (against the PG service) |
| `cargo clippy --workspace --all-targets -- -D warnings` | zero clippy warnings (`--all-targets` also subsumes `cargo check`, so no separate check step) |
| `cargo audit` | no known-vulnerable dependencies (the tool is installed as a prebuilt binary via `taiki-e/install-action@v2`, not compiled from source) |

Caching is via `Swatinem/rust-cache@v2`; the toolchain is stable with `clippy` + `rustfmt`.

## The `graph` job

Runs on `ubuntu-latest` in parallel with `test` (no Rust toolchain or Postgres needed).
Builds LAYER 1 of the navigation model from the exact checked-out commit, validates its
focused corpus and reproducibility, and publishes the ignored output.

| Step | What it enforces |
|------|------------------|
| `actions/setup-python@v5` (`3.12`) + `pip install "graphifyy==0.9.25"` | the pinned extractor used for every CI artifact |
| `./scripts/check-graph.sh` (env `GRAPHIFY_REQUIRED=1`) | builds twice and verifies same-tree reproducibility, corpus exclusions, cross-file edges, domain hubs and report quality |
| `actions/upload-artifact@v4` | publishes the complete `graphify-out/` directory as `knowledge-graph-<commit SHA>` for 14 days, including hidden Graphify metadata |

## Notes & gotchas

- `DATABASE_URL` here points at the service container's superuser (`keeplin:keeplin`); `sqlx::test`
  needs create-database rights, which that role has.
- `-D warnings` makes clippy findings **fail** the build — treat a clippy note as a required fix.
- `cargo audit` is installed each run as a **prebuilt binary** (`taiki-e/install-action@v2`)
  rather than compiled from source, which keeps the step fast; a new advisory can turn CI red
  without any code change, which is intended (it surfaces a dependency to bump).

### `converge` — Review loop converged

Runs on `ubuntu-latest`, `needs: [test, graph]`, non-draft pull requests only. Implements
[keeplin ADR 0004](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0004-review-loop-convergence.md),
identically to `keeplin`.

**Why this is a job and not a step.** Convergence asserts "the required checks are green". A
step inside `test` cannot make that claim: it ran before `cargo test --workspace` against the
PostgreSQL service, Clippy and audit, and while `graph` was still going. `needs: [test, graph]`
makes the claim checkable; `always()` keeps the job reporting when a dependency fails, so
branch protection sees a red check rather than a skipped one.

The job name is a branch-protection required-check identifier, exactly like `graph`'s: adding
it to the workflow does not enforce it until it is added to the required-check list in
Settings -> Branches.

| State | Meaning | Check |
|-------|---------|-------|
| `converged` | Required checks green and no reified finding open | passes |
| `awaiting-checks` | Nothing blocks, but a required check has not finished | fails |
| `converging` | The blocking set is non-empty but shrinking | fails |
| `escalated` | The loop state repeated, or the blocking set has not shrunk for `REVIEW_LOOP_STAGNATION_LIMIT` rounds | fails, and demands a `docs/review-stalls.md` `## Open` row naming this pull request and every current blocker |
| `malformed` | The ledger or round log contradicts the observed state | fails |

A body with no `## Review ledger` section is round zero, not malformed — ADR 0004's migration
contract. The loop-state hash is `sha256(normalized diff : open reified finding IDs : red check
names)`, joined with `\x1e`/`\x1f` rather than commas because check-run names contain commas.

This job is a floor beneath `Check pull-request review governance`, never a substitute for it.
The two are conjunctive.

## Related files

- `../scripts/check-review-loop.js` — the convergence and stagnation evaluator.
- `../scripts/check-review-governance.js` — the independent-review and waiver evaluator.
- `../../docs/review-stalls.md` — the durable record of escalated loops.
- `../../crates/keeplin-srv/tests/integration.md` / `collab.md` — the suites this runs.
- `../../docker-compose.yml.md` — the equivalent Postgres for local runs.
- `../../.env.example.md` — the `DATABASE_URL` shape mirrored here.
