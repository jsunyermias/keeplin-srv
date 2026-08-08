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
| [0003 — Making per-user quotas hold](0003-per-user-quota-serialization.md) | accepted | enforcement at every path that creates a counted object, plus a per-user advisory lock so the check and the write it authorizes are mutually exclusive | [keeplin-srv#142](https://github.com/jsunyermias/keeplin-srv/issues/142), [keeplin-srv#145](https://github.com/jsunyermias/keeplin-srv/issues/145) | [keeplin-srv#143](https://github.com/jsunyermias/keeplin-srv/pull/143) |
| [0005 — Who must join the serializable protocol](0005-serializable-participant-set.md) | **accepted**; supersedes `0002` in part (its eight-handler enumeration and its deferral of non-HTTP entry points, only as they concern the serializable participant set) | widens the participant set to nine handlers plus the synchronization path's notebook writes, and makes the set structural, after phase 2 established that a `SERIALIZABLE` transaction neither observes nor is aborted by a concurrent `READ COMMITTED` writer | [keeplin-srv#147](https://github.com/jsunyermias/keeplin-srv/issues/147) | [keeplin-srv#148](https://github.com/jsunyermias/keeplin-srv/pull/148) |
| [0006 — Making a journaled change reach its projection](0006-durable-projection.md) | accepted | a durable projection job written inside the journal transaction, claimed under a lease and retried, with a retention interlock so pruning cannot erase a job's own input, plus a reconciliation command; a journaled change is projected or visibly dead-lettered. Answers the exhaustion sentence `0005` defers, and forecloses no option of `0004` | [keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75) | [keeplin-srv#148](https://github.com/jsunyermias/keeplin-srv/pull/148) |
| [0004 — How the synchronization path refuses an over-quota change](0004-sync-quota-refusal.md) | rejected | proposed admission control before the journal insert; rejected because it could not hold `0003`'s invariants 2 and 3 on this path and would have superseded them the day they were accepted. The question moves to `keeplin-srv#75`'s ADR, which inherits this document's eleven verified facts, its four-mechanism analysis and its verification rows | [keeplin-srv#145](https://github.com/jsunyermias/keeplin-srv/issues/145) | none — rejected in [keeplin-srv#146](https://github.com/jsunyermias/keeplin-srv/pull/146) |

`0001` has no canonical or companion ADR in `keeplin`: the capability model, both share tables and
every function it names are local to this repository, and it moves no shared wire or format
surface.

`0002` has no canonical or companion ADR in `keeplin`: `keeplin` has neither this HTTP layer nor
PostgreSQL, and the decision changes no shared wire, format or `keeplin-core` surface.

`0003` has no canonical or companion ADR in `keeplin`: per-user quotas are a server concept with no
equivalent write path in `keeplin`, and the decision moves no shared wire, format or `keeplin-core`
surface.

`0005` is accepted and supersedes `0002` in part: its enumeration of eight handlers and
its deferral of non-HTTP entry points, both only as they concern which transactions must be
serializable. `0002`'s re-verification rule, refusal shapes, retry bound, `503` on exhaustion and
notice ordering are untouched. The partial supersession is recorded here because `0002` states both
that every participant must join the protocol and that non-HTTP entry points are deferred, and phase
2 of its implementation established that the synchronization path is a participant. `0005` has no
canonical or companion ADR in `keeplin`, which has neither this HTTP layer nor PostgreSQL.

`0004` is rejected and kept rather than withdrawn. It proposed deciding the synchronization path's
quota refusal ahead of `keeplin-srv#75`, and two review rounds established that doing so would have
required superseding `0003`'s invariants 2 and 3 on that path the day they were accepted, would have
depended on a fix to `handle_incoming` that quota did not cause, and would still have left the bound
sequential. A rejected ADR whose analysis is discarded costs the next decision the same work twice,
so the record stays: its eleven facts were each read out of the tree and five of them were wrong in
an earlier draft.

`0006` is accepted and is that decision for
[keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75). It rejects the single
transaction the issue itself recommended, on the ground that no acknowledgement exists and the client
never re-sends, so a rolled-back journal append loses a batch irrecoverably. The issue's bar is a
queue only if a queue is demonstrably needed; the demonstration is in the decision, and it is that
`put_resource_blob` carries no last-writer guard while multiple replicas are a documented deployment,
so concurrent appliers are unsafe without a lease. It inherits `0004`'s verified facts and answers the exhaustion sentence `0005` defers.
It does **not** decide the synchronization path's quota representation, which stays with
[keeplin-srv#145](https://github.com/jsunyermias/keeplin-srv/issues/145); it records that a durable
queue is what makes that decision expressible. `0006` has no canonical or companion ADR in `keeplin`:
the journal, the projections and the worker are local to this repository.

When adding a local ADR, register it here with status, scope, issue, acceptance PR, and any
canonical or companion ADR in `keeplin`.
