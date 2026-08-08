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
| [0004 — Deterministic convergence and a stagnation brake for the review loop](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0004-review-loop-convergence.md) | superseded via 0006 by 0008 | binds this repository's review loop identically: same checker, ledger, stagnation brake and stall record ([keeplin-srv#104](https://github.com/jsunyermias/keeplin-srv/pull/104)) |
| [0005 — Loop history lives outside the pull-request body](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0005-loop-history-outside-the-pull-request-body.md) | rejected | did not define a trusted writer; replaced by 0006 |
| [0006 — Trusted review-loop history](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0006-trusted-review-loop-history.md) | superseded by 0008 | accepted, then found unimplementable: a digest chain cannot detect deletion of its own newest record |
| [0007 — Trusted evaluator, verified disposal, and dual-store loop history](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0007-trusted-evaluator-and-dual-store-history.md) | rejected | its check-run immutability claim was false; rejected by two reviews before acceptance |
| [0008 — Trusted evaluator, verified disposal, and a bounded history claim](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0008-trusted-evaluator-verified-disposal-and-a-bounded-history-claim.md) | **accepted**; authenticity claim superseded by 0011 | the standing evaluator and disposal decision. [0011](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0011-bounded-journal-authenticity.md) narrows the authenticity claim while leaving its terminal-truncation bound standing |
| [0009 — Review governance is evaluated from the default branch](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0009-governance-evaluated-from-the-default-branch.md) | superseded by 0012 | extended 0008 and addressed F-025; [0012](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0012-default-branch-review-governance.md) preserves the direction while completing its required analysis and resolving the `ci.yml` contradiction |
| [0011 — Bound the review journal's authenticity claim](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0011-bounded-journal-authenticity.md) | accepted | binds both repositories to the same accidental-corruption and casual-editing claim; it explicitly does not defend against another repository workflow carrying the same App identity |
| [0012 — Evaluate review governance from the default branch](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0012-default-branch-review-governance.md) | accepted | keeps the `ci.yml` governance step as a nonauthoritative fast signal and makes default-branch evaluation authoritative, while preserving the bound that head-authored evidence remains unauthenticated; implementation is deferred to a separate pull request off `main` |
| [0013 — What an empty review journal may do](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0013-genesis-anchor-on-an-empty-journal.md) | accepted | permits an unauthenticated empty-journal genesis to evaluate while synthetic `GENESIS` remains open and reified, so convergence still requires verified authorization |

## Server-specific decisions

The server-specific decisions are registered below.

| ADR | Status | Scope | Issue | Acceptance PR |
|---|---|---|---|---|
| [0001 — Note moves and the provenance of note shares](0001-note-moves-and-share-provenance.md) | accepted | note/notebook permission surface: who may move a note, whether a move may alter its grants, and the named deployment-selected permission scheme that fixes those policy points | [keeplin-srv#110](https://github.com/jsunyermias/keeplin-srv/issues/110) | [keeplin-srv#121](https://github.com/jsunyermias/keeplin-srv/pull/121) |
| [0002 — Authorization and mutation atomicity](0002-authorization-mutation-atomicity.md) | accepted | server-wide rule joining each HTTP authorization check to the mutation it authorizes through transactional re-verification | [keeplin-srv#123](https://github.com/jsunyermias/keeplin-srv/issues/123) | [keeplin-srv#132](https://github.com/jsunyermias/keeplin-srv/pull/132) |
| [0003 — Making per-user quotas hold](0003-per-user-quota-serialization.md) | proposed | enforcement at every path that creates a counted object, plus a per-user advisory lock so the check and the write it authorizes are mutually exclusive | [keeplin-srv#142](https://github.com/jsunyermias/keeplin-srv/issues/142), [keeplin-srv#145](https://github.com/jsunyermias/keeplin-srv/issues/145) | none; proposed does not authorize implementation |

`0001` has no canonical or companion ADR in `keeplin`: the capability model, both share tables and
every function it names are local to this repository, and it moves no shared wire or format
surface.

`0002` has no canonical or companion ADR in `keeplin`: `keeplin` has neither this HTTP layer nor
PostgreSQL, and the decision changes no shared wire, format or `keeplin-core` surface.

A second local decision is expected for
[keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75): it must decide between
transactional materialization and a durable projection queue before implementation begins.

When adding a local ADR, register it here with status, scope, issue, acceptance PR, and any
canonical or companion ADR in `keeplin`.
