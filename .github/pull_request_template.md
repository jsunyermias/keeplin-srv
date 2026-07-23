<!--
  Fill in the sections below. Delete any that genuinely do not apply, but do not
  delete the checklist — CI enforces most of it, and a reviewer will look for it.
-->

## Summary

<!-- What does this PR change, and why? One short paragraph. -->

## Linked issues

<!-- e.g. "Resolves #128" / server-side of a keeplin change. If none, say so and why. -->

## Type of change

- [ ] Feature
- [ ] Bug fix
- [ ] Refactor (no behaviour change)
- [ ] Docs / companion-only
- [ ] Chore / tooling

## What changed

<!-- Bullet the concrete changes: migrations, store, sync, http, … -->

## Database & compatibility

- [ ] New migrations are **forward-only** and idempotent (`ADD COLUMN IF NOT EXISTS`, `CREATE … IF NOT EXISTS`); existing migrations are never edited after being applied.
- [ ] New `NOT NULL` columns carry a `DEFAULT` so existing rows stay valid without a table rewrite.
- [ ] Every migration `.sql` has its companion `.md`.
- [ ] Any `SELECT` feeding a `sqlx::FromRow` struct includes all of that struct's columns (a missing column fails the row decode at runtime, not at compile time).

## Contract

- [ ] Every touched `.rs` has its companion `.md` updated **verbatim** (block-complete v2.3.1): one `// md:` marker per block, one Coverage-checklist row per marker, no elided fences, no non-`// md:` comments in the `.rs`.
- [ ] `scripts/check-docs.sh` passes clean.
- [ ] If this PR consumes a new `keeplin-core` API, the `keeplin-core` git `rev` in `Cargo.toml` is bumped to a commit whose own CI is green.
- [ ] No stray references to "pizarra" in touched code.

## Verification

- [ ] `cargo fmt --check --all` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo test --workspace` green against Postgres (`sqlx::test` integration tests included).
- [ ] Tests added or updated for the new behaviour.
- [ ] `graphify update .` run and the refreshed `graphify-out/` committed (code changes only). CI (`scripts/check-graph.sh`) fails if the graph is stale; enable the auto-refresh hook once with `git config core.hooksPath .githooks`. Requires `pip install graphifyy==0.9.25`.

<!-- Paste anything a reviewer should know that the diff doesn't show:
     manual testing done, follow-ups deferred, known limitations. -->
