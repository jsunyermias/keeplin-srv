# Architecture decision records

This is the `keeplin-srv` decision registry. The canonical ADR framework and template live in
[`jsunyermias/keeplin/docs/adr`](https://github.com/jsunyermias/keeplin/tree/main/docs/adr)
because `keeplin-core` owns the shared model, wire, and format contracts.

Use the canonical
[`0000-template.md`](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0000-template.md)
for every ADR. Cross-repository decisions are accepted and versioned in `keeplin`; this registry
links them rather than copying their reasoning. Decisions that affect only server internals live
in this directory and use this repository's own next unused four-digit number. Because numbering is
per repository, a bare number is ambiguous across the two registries: always qualify a reference
with its repository, as in `keeplin ADR 0002` or `keeplin-srv ADR 0001`.

The lifecycle is `proposed` → `accepted` or `rejected`, with later accepted decisions marking old
ones `superseded`. `proposed` blocks implementation. Only the maintainer accepts or rejects an
ADR. `(retrospective)` is a qualifier that may accompany any status and records that the ADR
documents already-implemented behavior rather than proposing new behavior; it does not weaken the
blocking effect of `proposed`. Once accepted, the decision body is immutable historical record;
replace it with a new ADR rather than rewriting it to fit later code. Issues and acceptance PRs
link the ADR in both directions.

## Canonical cross-repository decisions

| ADR | Status | Server impact |
|---|---|---|
| [0001 — Current synchronization delivery semantics](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0001-current-sync-delivery.md) | accepted (retrospective) | records durable journal/cursor behavior and the unacknowledged end-to-end loss windows |
| [0002 — Shared domain model and server projections](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0002-shared-domain-model.md) | accepted (retrospective) | imports canonical core types and materializes queryable PostgreSQL projections |
| [0003 — Versioned persistent formats and forward migrations](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0003-versioned-persistence.md) | accepted (retrospective) | append-only PostgreSQL migrations, backup and recovery boundary |
| [0004 — Deterministic convergence and a stagnation brake for the review loop](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0004-review-loop-convergence.md) | accepted | binds this repository's review loop identically: same checker, ledger, stagnation brake and stall record ([keeplin-srv#104](https://github.com/jsunyermias/keeplin-srv/pull/104)). Its storage mechanism is amended by keeplin ADR 0005 |
| [0005 — Loop history lives outside the pull-request body](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0005-loop-history-outside-the-pull-request-body.md) | proposed | same amendment here: loop history moves to CI-written check runs so an editing agent cannot reset the stagnation brake. Requires `checks: write` on the `converge` job in this repository too |

## Server-specific decisions

No server-specific ADR has been accepted yet. The first expected use is
[keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75): it must decide between
transactional materialization and a durable projection queue before implementation begins.

When adding a local ADR, register it here with status, scope, issue, acceptance PR, and any
canonical or companion ADR in `keeplin`.
