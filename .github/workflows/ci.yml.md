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
runs. The `checks: read` scope lets the canary locate its own check run but grants no mutation
ability. The canary separately requires a successful GET and a non-empty check ID, then attempts
to patch that run and requires exactly HTTP 403; a failed lookup cannot masquerade as a passing
denial, and any successful PATCH fails CI. Body edits retrigger the workflow, so completing or
removing review evidence — and editing the review ledger — is reflected in the required check
without a new commit. The separate default-branch `review-loop-evaluator.yml` consumes completed
runs.

## The `test` job

Runs on `ubuntu-latest` with a **real PostgreSQL 16 service container** (not a mock): the
integration tests use `sqlx::test`, which creates one throwaway database per test from
`DATABASE_URL`. The service exposes `5432:5432` and is gated on `pg_isready` health checks before
the steps run.

| Step | What it enforces |
|------|------------------|
| API `GET` plus `PATCH` canary (pull requests only) | the pull-request token cannot rewrite check runs: a successful check-run lookup and HTTP 403 from the mutation attempt; lookup failure, missing ID, or a successful mutation fails CI |
| `actions/github-script@v7` (non-draft pull requests only) | either an independent review with evidence, or a complete maintainer waiver whose exact PR is recorded in the changed `docs/review-debt.md` |
| `actions/setup-python@v5` (`3.12`) | the standard-library runtime used by deterministic companion verification |
| `./scripts/check-docs.sh` | every `.rs` has a structurally valid companion, every Rust fence is exactly faithful to source, and the generated context manifest is current |
| `python3 -m unittest discover -s scripts/tests -p 'test_*.py'` | syntax fixtures, drift/error detection, fence-only sync and reproducible context packs |
| `cargo fmt --check --all` | formatting is committed |
| `cargo test --workspace` | unit + integration tests pass (against the PG service) |
| `cargo clippy --workspace --all-targets -- -D warnings` | zero clippy warnings (`--all-targets` also subsumes `cargo check`, so no separate check step) |
| `cargo audit` | no known-vulnerable dependencies (the tool is installed as a prebuilt binary via `taiki-e/install-action@v2`, not compiled from source) |
| `node --test` over both governance suites with the job's read-only `GITHUB_TOKEN` (**runs last**, so a governance regression does not abort the job before docs, cargo and audit have reported) | the reviewed and maintainer-waiver paths, and the convergence, recurrence, advisory and stagnation paths, trusted-evaluator isolation, verified disposal, a real exhaustive collaborator enumeration with the evaluator credential, and the bounded-journal limitation, including negative cases |

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

### Trusted convergence

The former head-controlled `converge` job is removed. Implements
[keeplin ADR 0008](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md),
identically to `keeplin`. The default-branch
[`review-loop-evaluator.yml`](review-loop-evaluator.yml) workflow is the authoritative
evaluator after this unprivileged workflow completes. Only `Check, Test & Lint` and
`Knowledge graph up to date` count, and each must positively report `success`.

**Why the evaluator is a separate default-branch workflow.** Convergence asserts "the required
checks are green", and a step inside `test` cannot make that claim: it runs before
`cargo test --workspace` against the PostgreSQL service, Clippy and audit, and while `graph` is
still going. It also cannot make the claim *trustworthily*, because everything in this workflow
is head-controlled. The evaluator's definition comes from the default branch and it never checks
out, executes or interpolates head content.

The evaluator publishes a check run named `Review loop converged`. That name is a
branch-protection required-check identifier, exactly like `graph`'s: it does not enforce
anything until it is added to the required-check list in Settings -> Branches, and it must not
be added there until the evaluator has reported at least once, or every pull request blocks on
a check nobody reports.

| State | Meaning | Check |
|-------|---------|-------|
| `converged` | Required checks green and no reified finding open | passes |
| `awaiting-checks` | Nothing blocks, but a required check has not finished | fails |
| `converging` | The blocking set is non-empty but shrinking | fails |
| `escalated` | The loop state repeated, or the blocking set has not shrunk for `REVIEW_LOOP_STAGNATION_LIMIT` rounds | fails, and demands a `docs/review-stalls.md` `## Open` row naming this pull request and every current blocker |
| `malformed` | The ledger or round log contradicts the observed state | fails |
| `history-unverifiable` | The journal, ledger or a disposal authorization failed verification | fails |
| `fork-refused` | The pull request comes from a fork, so evidence is partial | fails |

A body with no `## Review ledger` section is round zero, not malformed — ADR 0004's migration
contract, carried forward by 0008. The loop-state hash is SHA-256 over canonical JSON containing
the normalized changed-file tuples, the sorted open reified finding IDs and the sorted red check
names, so a delimiter byte inside a filename or a check-run name cannot make two different
states collide.

The evaluator is a floor beneath `Check pull-request review governance`, never a substitute for
an independent reviewer. Fork pull requests deliberately fail closed rather than evaluate partial
journal evidence.

## Related files

- `../scripts/check-review-loop.js` — the convergence and stagnation evaluator.
- `../scripts/check-review-governance.js` — the independent-review and waiver evaluator.
- `review-loop-evaluator.yml` — the default-branch authoritative evaluator.
- `../../docs/review-stalls.md` — the durable record of escalated loops.
- `../../crates/keeplin-srv/tests/integration.md` / `collab.md` — the suites this runs.
- `../../docker-compose.yml.md` — the equivalent Postgres for local runs.
- `../../.env.example.md` — the `DATABASE_URL` shape mirrored here.
