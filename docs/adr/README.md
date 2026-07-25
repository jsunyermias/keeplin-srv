# Architecture decision records

This is the `keeplin-srv` decision registry. The canonical ADR framework and template live in
[`jsunyermias/keeplin/docs/adr`](https://github.com/jsunyermias/keeplin/tree/main/docs/adr)
because `keeplin-core` owns the shared model, wire, and format contracts.

Use the canonical
[`0000-template.md`](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0000-template.md)
for every ADR. Cross-repository decisions are accepted and versioned in `keeplin`; this registry
links them rather than copying their reasoning. Decisions that affect only server internals live
in this directory and use this repository's own next unused four-digit number.

The lifecycle is `proposed` → `accepted` or `rejected`, with later accepted decisions marking old
ones `superseded`. `proposed` blocks implementation. Only the maintainer accepts or rejects an
ADR. Once accepted, the decision body is immutable historical record; replace it with a new ADR
rather than rewriting it to fit later code. Issues and acceptance PRs link the ADR in both
directions.

## Canonical cross-repository decisions

| ADR | Status | Server impact |
|---|---|---|
| [0001 — Current synchronization delivery semantics](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0001-current-sync-delivery.md) | proposed (retrospective) | records durable journal/cursor behavior and the unacknowledged end-to-end loss windows |
| [0002 — Shared domain model and server projections](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0002-shared-domain-model.md) | proposed (retrospective) | imports canonical core types and materializes queryable PostgreSQL projections |
| [0003 — Versioned persistent formats and forward migrations](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0003-versioned-persistence.md) | proposed (retrospective) | append-only PostgreSQL migrations, backup and recovery boundary |

## Server-specific decisions

No server-specific ADR has been accepted yet. The first expected use is
[keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75): it must decide between
transactional materialization and a durable projection queue before implementation begins.

When adding a local ADR, register it here with status, scope, issue, acceptance PR, and any
canonical or companion ADR in `keeplin`.
