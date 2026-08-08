# 0005 — Who must join the serializable protocol

- Status: proposed
- Date: 2026-08-08
- Decision owners: maintainer of `jsunyermias/keeplin-srv`
- Scope: keeplin-srv
- Issue: [keeplin-srv#147](https://github.com/jsunyermias/keeplin-srv/issues/147)
- Acceptance PR: link once the ADR is accepted
- Supersedes: [ADR 0002](0002-authorization-mutation-atomicity.md) in part — its eight-handler
  enumeration and its deferral of non-HTTP entry points, both only as they concern the serializable
  participant set. Everything else in ADR 0002 stands.
- Superseded by: none

## Context and problem

`Verified at`: `keeplin-srv@14640f6`.

Accepted [ADR 0002](0002-authorization-mutation-atomicity.md) requires authorization and the
operation it authorizes to be one transactionally consistent decision, and puts eight HTTP mutation
handlers under `SERIALIZABLE` because accepted [ADR 0001](0001-note-moves-and-share-provenance.md)'s
written invariants commit their behaviour. Phase 2 of its implementation
([keeplin-srv#147](https://github.com/jsunyermias/keeplin-srv/issues/147)) built that, and building it
exposed a contradiction inside ADR 0002 that had no way of being noticed before.

### The fact that forces this decision, verified rather than reasoned

**A `SERIALIZABLE` transaction does not observe, and is not aborted by, a concurrent
`READ COMMITTED` writer.** PostgreSQL takes predicate locks only for serializable transactions, so a
read/write antidependency is recorded only when both participants are serializable, and the dangerous
structure that triggers `40001` cannot form with one.

Verified on the PostgreSQL 16 that CI uses, with two sessions:

| Step | Session A (`SERIALIZABLE`) | Session B (`READ COMMITTED`) |
|---|---|---|
| 1 | reads the share table: `0` rows | |
| 2 | | inserts a share row, commits |
| 3 | re-reads inside its transaction: **still `0`** | |
| 4 | **commits successfully — no `40001`** | |

The consequence is precise and uncomfortable: an in-transaction guard re-read at `SERIALIZABLE` can
be **staler** than the same re-read at `READ COMMITTED` would have been. Isolation protects the
serializable transaction from other serializable transactions and from nothing else.

The one exception is not SSI: if the serializable transaction *writes* a row that a concurrent
transaction of any isolation level updated after its snapshot, PostgreSQL raises `40001` through its
ordinary row-version check. That covers write/write collisions and not the guard reads this decision
is about.

### The writers ADR 0002 leaves outside, and what they touch

Two exist, both found by independent review of phase 2 and both verified against the tree.

**The synchronization path.** `Store::upsert_notebook` and `Store::delete_notebook` are called from
exactly one place in the crate — `sync.rs:300` and `sync.rs:310`, inside `materialize`, at
`READ COMMITTED`. No HTTP handler writes a notebook row at all. Notebook rows feed
`resolve_notebook_access_on`, the inherited branch of `resolve_note_access_on`, and
`inherited_note_principals_on`, which filters on the notebook's live row. A notebook deletion
materialized while one of the eight is mid-transaction can change owner access to `NotFound`,
inherited access to `Forbidden`, alter the move-out principal set, or invalidate destination write
authority — and the handler commits on the earlier view.

**`delete_account`.** It calls `Store::delete_user`, which runs `DELETE FROM users` on the pool
(`store.rs:377`), outside any serializable boundary. Foreign keys cascade to `notes`, `note_shares`,
`notebooks` and `notebook_shares`. It deletes the requester's own account, which sounds self-contained
and is not: when a grantee deletes their account, their `note_shares` rows vanish while another
user's move guard is mid-transaction deciding whether that same grantee loses access. It is an HTTP
handler, inside ADR 0002's own stated audit boundary, and it is not among the eight.

An exhaustive search found no others: `collab.rs` writes no share, ownership or notebook row, and the
maintenance tasks in `main.rs` write none of these tables.

### The contradiction

ADR 0002 says both of these, and they cannot both hold:

> Every transaction that can read or write the state participating in one of those invariants must
> join the `SERIALIZABLE` protocol; making only the move transaction serializable would not establish
> the invariant against a `READ COMMITTED` share writer.

> **Not decided** — Authorization atomicity for non-HTTP entry points, including
> collaboration/WebSocket paths. A later audit may extend the rule through a separate decision if
> their mutation model requires it.

The synchronization path participates and is deferred. `delete_account` participates and is simply
absent from an enumeration ADR 0002 calls complete. Phase 2 cannot resolve this by choosing an
interpretation, because ADR 0002 is accepted; that is the error
[ADR 0004](0004-sync-quota-refusal.md) was rejected for making.

## Forces and requirements

- The invariant ADR 0002 exists to establish must actually hold, not hold against a subset of writers
  chosen by which module the writer lives in.
- Whatever is decided must be **detectable**: a future writer of this state must fail a check rather
  than quietly join the set of writers nobody enumerated.
- ADR 0002's re-verification rule, refusal shapes, retry bound, `503` on exhaustion and notice
  ordering are unchanged. This decision moves the participant set and nothing else.
- The synchronization path's throughput matters: it is a fan-in from every device, and it is not an
  interactive request that can be told to retry.
- Phase 3's read paths are out of scope. `REPEATABLE READ` reads take no predicate locks either, and
  what that means for them belongs to phase 3's own analysis.

## Threat model

**Asset.** ADR 0001's invariants, in particular invariant 6: a move must not silently remove a
controlled principal's access.

**Adversary.** Not necessarily an adversary at all, which is what makes this worth deciding. The
synchronization writer is any device of any user syncing an ordinary notebook deletion. The
`delete_account` writer is a user closing their own account. Neither requires intent, timing skill or
privilege; both are the product working normally.

**Consequence.** A move, share, ejection or transfer commits against an authorization view that a
concurrently committed change had already invalidated, with no error raised anywhere.

**Out of scope.** Collaboration line editing, which writes none of this state; and read paths, which
are phase 3.

## Options considered

### Option 1 — Keep ADR 0002's enumeration and record the gap

Narrow what phase 2 claims, document the two writers as a known limit, open an issue.

Rejected by the maintainer, and the reason is worth recording rather than the rejection alone: the
gap is not an edge. It is reachable by syncing a notebook deletion, which is ordinary use, so a
documented limit would be a documented defect.

### Option 2 — Bring both writers into the serializable protocol, and enumerate participants

Add `delete_account` to the enumerated set, making it nine. Put the synchronization path's notebook
writes under the same protocol. Make the participant set structural so a new writer fails a check.

### Option 3 — An anchor row locked by every participant

Have every guard transaction and every writer take `SELECT … FOR UPDATE` on a common row, forcing
write/write collisions and therefore `40001` regardless of isolation.

Not adopted, but it is the strongest alternative and is recorded properly rather than dismissed. It
would close the window without changing any isolation level, and it composes with writers this
decision has not foreseen. Its costs: it introduces a second serialization mechanism alongside the
one ADR 0002 chose, it requires every participant to agree on the anchor — which is the same
enumeration problem in a different shape — and the anchor becomes a per-notebook or per-user
bottleneck on paths that have none today. It also collides with the lock-domain contract
[ADR 0003](0003-per-user-quota-serialization.md) requires, since `lock_note_order` already shares the
advisory-lock space.

### Option 4 — `REPEATABLE READ` for the writers instead of `SERIALIZABLE`

Not expressible as a fix: `REPEATABLE READ` takes no predicate locks either, so it changes nothing
about what the guard transaction observes.

## Decision and justification

> This ADR is `proposed`. It records a recommendation and does not authorize implementation. Only
> the maintainer may accept or reject it.

**Proposed decision: adopt Option 2.**

**Part one — the enumerated set becomes nine.** `delete_account` joins `update_note`, `delete_note`,
`create_share`, `delete_share`, `transfer_ownership`, `create_notebook_share`,
`delete_notebook_share` and `transfer_notebook`. It is an HTTP mutation inside ADR 0002's own audit
boundary whose cascade revokes authority that ADR 0001's invariants govern; its absence was an
omission in the enumeration rather than a scoping choice, because ADR 0002 never states a reason for
excluding it.

**Part two — the synchronization path's notebook writes join the protocol.** `materialize`'s calls to
`upsert_notebook` and `delete_notebook` execute in a serializable transaction with the same retry
bound. This is the part that extends ADR 0002 into a non-HTTP entry point, which that ADR reserved
for a separate decision; this is that decision, and it is deliberately narrow: **only the notebook
writes, only because they feed authorization guards.** Nothing else in `materialize` changes, and
this ADR does not decide anything about the rest of the synchronization path, which remains
keeplin-srv#75's ground.

**Part three — the participant set is structural.** A test enumerates every writer of the state the
enumerated guards read — note rows, note shares, notebook rows, notebook shares and ownership — and
fails when a writer appears that is not in the set or does not join the protocol. The handler
inventory added in phase 1 detects a new *handler*; nothing today detects a new *writer*, which is
exactly how both of these arrived unnoticed.

**Part four — what remains true and unclaimed.** A serializable transaction is still unprotected
against any writer outside the set. That is a property of PostgreSQL, not of this decision, and part
three is what keeps the set honest rather than a claim that the set is permanently complete.

The invariants proposed are:

1. Every enumerated participant — the nine handlers and the synchronization path's notebook writes —
   executes its authorization re-verification and its write in one `SERIALIZABLE` transaction.
2. No writer of note rows, note shares, notebook rows, notebook shares or ownership executes outside
   the protocol.
3. Invariant 2 is established by a check that fails when a new writer appears, not by inspection.
4. ADR 0002's re-verification rule, its `403`/`MoveBlocked` refusal shapes, its three-attempt bound,
   its `503` on exhaustion and its notice ordering are unchanged.
5. Retry exhaustion on the synchronization path does not return `503`, because that path is not an
   HTTP request. Its representation is decided below.

### The synchronization path's exhaustion, decided rather than left

ADR 0002's `503` is an HTTP answer and `materialize` has no response. On exhaustion the batch's
notebook write is **not applied**, the failure is recorded in telemetry, and the change remains in the
journal. This is the existing behaviour for any materialization failure — ADR 0004 fact 3 — so this
decision adds no new loss mode; it makes the existing one reachable by contention as well as by error.

That is stated as a cost rather than hidden: a notebook deletion can fail to materialize under
contention and nothing retries it, which is keeplin-srv#75's defect and not a new one.

### Costs, stated rather than implied

**The synchronization path gains serializable transactions and a retry loop it does not have.** It is
a fan-in from every device of every user. This decision does not measure that cost, and the
verification plan requires it to be measured rather than assumed.

**Nine is not obviously final.** Part three exists precisely because this decision does not trust its
own enumeration.

**A second decision now governs part of ADR 0002.** A reader of ADR 0002's enumeration must read this
one too. That is the cost of correcting an accepted decision rather than rewriting it, and the
registry records it.

### Not decided

- Anything about phase 3's read paths. `REPEATABLE READ` takes no predicate locks either, and what
  that implies for authorized reads is phase 3's analysis to do.
- The rest of the synchronization path's materialization semantics, which remain keeplin-srv#75's.
- Whether the anchor-row mechanism of option 3 is a better long-term answer. It stays available.

## Consequences and risks

- The invariant ADR 0002 claims becomes true against every writer this repository has today.
- The synchronization path can now fail to apply a notebook write under contention, with the same
  silence as any other materialization failure.
- A new writer of guarded state fails a check instead of joining silently.
- `delete_account` acquires a retry loop and can return `503`, which it cannot today.

## Compatibility, migration, and rollback

No schema change, no migration, no wire or format change. `keeplin-core` is untouched and its pin does
not move. `delete_account` gains `503` as a possible response under contention, which is a new status
on that endpoint and the only client-visible change.

Rollback returns to ADR 0002's eight and reopens the window this decision closes.

## Verification plan

| # | Evidence | Kind | What fails if the decision is violated |
|---|---|---|---|
| 1 | A deterministic interleaving test pauses one of the nine after its serializable snapshot, materializes a notebook deletion through the synchronization path, resumes, and requires either replay against the deletion or the ordinary current-state refusal | negative, forced interleaving | **Fails on the current tree.** That is the point: it is the defect this decision exists to close |
| 2 | The same for a `delete_account` cascade that removes an authorizing share while one of the nine is paused | negative, forced interleaving | Fails if the cascade can revoke authority a paused transaction still believes in |
| 3 | A structural writer inventory enumerates every writer of note rows, note shares, notebook rows, notebook shares and ownership, and fails when one is not in the protocol | structural | Fails if a new writer can be added outside the set, which is how both known writers arrived |
| 4 | Replacing the inventory's derivation with a hand-written list makes row 3 pass while a planted writer is uncovered | mutation | Fails if the inventory checks that a list exists rather than that it is derived |
| 5 | Downgrading any one participant to `READ COMMITTED` fails a test | mutation | Fails if the protocol is asserted by spelling rather than by behaviour |
| 6 | The synchronization path retries `40001` within the same bound and, on exhaustion, applies no notebook write, emits telemetry, and returns no HTTP status | failure injection | Fails if ADR 0002's `503` is copied onto a path that is not a request, or if a partial write survives |
| 7 | `delete_account` under injected `40001` retries within the bound and returns `503` only on exhaustion, with the account not deleted | failure injection | Fails if the cascade partially commits or exhaustion is hidden |
| 8 | ADR 0002's existing evidence — the move interleaving, byte-equivalent refusal, rollback, replay, retry bound and exhaustion tests — still passes unchanged | regression | Fails if widening the set changed the behaviour ADR 0002 established |
| 9 | A measurement of the synchronization path's throughput and latency before and after, recorded in the pull request | operational | Fails if the cost of serializing a fan-in path is assumed rather than observed |
| 10 | `./scripts/check-docs.sh` passes with every changed source companion synchronized | documentation | Fails if implementation and documentation diverge |
| 11 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` pass | repository | Fails on behavioral regressions, warnings, or formatting drift |

Rows 4 and 5 are mutation evidence, and rows 1 and 2 require a forced rendezvous rather than hoping
for an interleaving, for the reason [keeplin-srv#138](https://github.com/jsunyermias/keeplin-srv/issues/138)
records: during ADR 0002's phase 1 a test named for exactly the property it was meant to establish
passed for a full review round while that property was violated.

Row 1 is expected to fail before implementation. A verification plan whose rows all pass on the
current tree is describing the present rather than deciding anything.

## Equivalent decision in the other repository

None. `keeplin` has neither this HTTP layer nor PostgreSQL, and the decision changes no shared wire,
format or `keeplin-core` surface.
