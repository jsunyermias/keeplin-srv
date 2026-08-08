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
`notebooks` and `notebook_shares`. It deletes the requester's own account, and it is an HTTP handler
inside ADR 0002's own stated audit boundary that is not among the eight.

**Deleting a grantee is fail-closed. Deleting a notebook owner is not**, and the difference is a
missing foreign key. `notebooks.user_id` cascades from `users` and `notebook_shares.notebook_id`
cascades from `notebooks`, so closing a notebook owner's account hard-deletes the notebook and every
grant through it. But `notes.notebook_id` is a bare `UUID` with **no foreign key at all**
(`migrations/0003_note_metadata.sql`), so notes owned by surviving users stay attached to a notebook
row that no longer exists.

The fail-open schedule follows:

1. Bob holds inherited write access to a note through Alice's notebook.
2. Bob's `update_note` transaction resolves that inherited access and takes its snapshot.
3. Alice closes her account; her notebook and its shares cascade away.
4. Bob resumes and writes the note on the stale inherited authority, and commits.

Account deletion runs at `READ COMMITTED`, so nothing rejects that execution. **A surviving principal
commits after their authority was revoked** — which is the shape ADR 0001 invariant 6 exists to
prevent, reached by a user closing their own account.

Two further consequences of the same cascade: a spurious refusal naming a grantee who has just ceased
to exist, and a concurrent `transfer_ownership` to the deleted user that raises SQLSTATE `23503`
rather than `40001` and is therefore never retried.

No others were found by searching this crate for statements writing those tables and by reading
`collab.rs` and `main.rs`'s maintenance tasks, which write none of them. That is inspection — the
same technique that missed both of these writers for a full review phase — so it is stated as what
was done rather than as a guarantee that the list is complete. Part three exists because inspection
is not enough, and because inspection over source would itself have missed the cascade that motivates
part one.

### The contradiction

ADR 0002 says both of these, and they cannot both hold:

> Every transaction that can read or write the state participating in one of those invariants must
> join the `SERIALIZABLE` protocol; making only the move transaction serializable would not establish
> the invariant against a `READ COMMITTED` share writer.

and, as a bullet under its `### Not decided` heading:

> Authorization atomicity for non-HTTP entry points, including collaboration/WebSocket paths. A later
> audit may extend the rule through a separate decision if their mutation model requires it.

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
concurrently committed change had already invalidated, with no error raised anywhere. That is
demonstrated for both writers: the synchronization path's notebook deletion leaves a handler
admitting or refusing on state that no longer exists, and an account closure that cascades a notebook
away lets a surviving principal commit on inherited authority that has already been revoked.

**Out of scope.** Collaboration line editing, which writes none of this state; and read paths, which
are phase 3.

## Options considered

### Option 1 — Keep ADR 0002's enumeration and record the gap

Narrow what phase 2 claims, document the two writers as a known limit, open an issue.

Not adopted. The maintainer's input on being shown the two writers was that the gap is not an edge:
it is reachable by syncing a notebook deletion, which is ordinary use, so a documented limit would be
a documented defect. That input is recorded as input — this ADR is `proposed`, and nothing in it
predetermines the decision the maintainer has yet to take on the document as a whole.

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
bottleneck on paths that have none today.

An earlier draft added a fourth cost, that it would collide with the lock-domain contract
[ADR 0003](0003-per-user-quota-serialization.md) requires. **That was wrong.** `SELECT … FOR UPDATE`
takes row locks, not advisory locks, so it does not touch `lock_note_order`'s space at all. The claim
is removed rather than quietly dropped, because the strongest alternative was being rejected partly on
a false ground. The three costs above stand on their own.

### Option 4 — `REPEATABLE READ` for the writers instead of `SERIALIZABLE`

Not expressible as a fix: `REPEATABLE READ` takes no predicate locks either, so it changes nothing
about what the guard transaction observes.

## Decision and justification

> This ADR is `proposed`. It records a recommendation and does not authorize implementation. Only
> the maintainer may accept or reject it.

**Proposed decision: adopt Option 2.**

**Part one — the enumerated set becomes nine.** `delete_account` joins `update_note`, `delete_note`,
`create_share`, `delete_share`, `transfer_ownership`, `create_notebook_share`,
`delete_notebook_share` and `transfer_notebook`.

**This justification has been wrong in both directions, and the record of that is kept.** The first
draft claimed the cascade silently removes a controlled principal's access. Round 1 refuted it for
the case it examined — a grantee's deletion takes the grantee and their share together, leaving no
survivor to have lost anything — and a second draft generalized that to *every* schedule being
fail-closed. **That generalization was false**, and round 2 produced the counter-example: the
notebook-owner case above, where a surviving principal commits on revoked inherited authority because
`notes.notebook_id` carries no foreign key. A claim weakened past what is true is as wrong as one
left too strong.

So part one rests on three things, in descending order of force:

- **A demonstrated fail-open schedule**, the notebook-owner deletion above. It is the ADR 0001
  invariant 6 shape, reached without an adversary.
- **A concrete unenumerated failure.** A `transfer_ownership` to a user whose account is being deleted
  raises SQLSTATE `23503`, which ADR 0002's retry does not classify, so it surfaces as a `500`.
- **Precautionary closure** for the remaining fail-closed schedules, labelled as precautionary.

**Serializing `delete_account` alone does not close the second of those**, and the ADR says so rather
than implying otherwise: `transfer_ownership` resolves its target user *before* entering the
transaction and never re-reads it inside, so no serialization dependency exists to detect the
deletion. This decision therefore also requires that **every handler resolving a target principal
re-verify that principal's existence inside its transaction** — `transfer_ownership`,
`transfer_notebook`, `create_share` and `create_notebook_share` all resolve outside today.

`ADR 0002` states no reason for excluding `delete_account`, so its absence reads as an omission
rather than a scoping choice — but the ADR is silent, not explicit, and that is an inference.

**Part two — the synchronization path's notebook writes join the protocol, stated as a property
rather than as a call boundary.** Any transaction that contains a guarded notebook write is
serializable and is retried as one whole unit within the same bound. Today that unit is
`materialize`'s call to `upsert_notebook` or `delete_notebook`; under keeplin-srv#75's option A it
would be the transaction containing the journal append and every projection, and the property is
written so that it survives that change instead of pinning the current boundary. This is the part that extends ADR 0002 into a non-HTTP entry point, which that ADR reserved
for a separate decision; this is that decision, and it is deliberately narrow: **only the notebook
writes, only because they feed authorization guards.** Nothing else in `materialize` changes, and
this ADR does not decide anything about the rest of the synchronization path, which remains
keeplin-srv#75's ground.

**Part three — the participant set is structural, and its derivation must reach cascades.** A test
enumerates every writer of the state the enumerated guards read — note rows, note shares, notebook
rows, notebook shares and ownership — and fails when a writer appears that is not in the set or does
not join the protocol. The handler inventory added in phase 1 detects a new *handler*; nothing today
detects a new *writer*, which is exactly how both of these arrived unnoticed.

**The derivation is named here because the obvious one would have missed the writer that motivates
this decision.** `delete_user` never writes `note_shares`; a foreign-key cascade does. An inventory
that scans source SQL statements would have found `DELETE FROM users` at `store.rs:377` and
enumerated it as a writer of `users`, while missing the share revocation entirely — and the share
revocation is the part that matters. So the derivation must union:

- **source-level writers**: statements in this crate that write those tables;
- **schema-level writers**: the foreign-key cascade closure over those tables, read from the
  PostgreSQL catalog rather than from a list in a test, plus any trigger that writes them.

An inventory built from source alone does not establish invariant 3, and saying so is the point of
naming the method rather than the goal.

**What that derivation still cannot do, stated so the check is not read as complete.** The catalog
enumerates declared foreign-key actions and triggers; it does not map a database-side write back to
the Rust entry point that must join the protocol, so the union produces a set of *tables reachable by
a write* and a set of *code sites that write*, and joining them is a judgement the check cannot make.
It does not follow trigger functions recursively, dynamically executed SQL, writable views or rules,
or writes inside called database functions. The planted decoy proves detection of the shape the decoy
has and no other. Invariant 3 is therefore established **up to that boundary**, and the boundary is
written here rather than left for the next reader to discover the way this decision's own authors
discovered the cascade.

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
   HTTP request. What it *does* is keeplin-srv#75's to decide, and this ADR does not decide it.

### The synchronization path's exhaustion, which this ADR does not decide

An earlier draft decided it, and review established that doing so would have been this ADR's own
version of the error [ADR 0004](0004-sync-quota-refusal.md) was rejected for.

The draft said: on exhaustion the notebook write is not applied, telemetry records it, and the change
remains in the journal. **That last clause presupposes that the journal entry and the projection are
separately durable — which is exactly what keeplin-srv#75's option A abolishes.** Under
journal-plus-projections-in-one-transaction, a failed notebook write rolls the journal append back
with it, and the behaviour that draft "decided" is not expressible at all. Deciding it here would
also entrench the non-retried loss #75 exists to eliminate.

So this ADR decides only the isolation level and the retry bound on those writes. What happens when
the bound is exhausted is **keeplin-srv#75's, and is recorded here as an observation of current
behaviour rather than as a decision**: today a materialization failure applies nothing, logs, and is
never re-attempted, because nothing re-drives the journal and no watermark records what was
projected. Serializing these writes makes that existing outcome reachable by contention as well as by
error.

Whether that is acceptable in the interval before #75 lands is a cost the maintainer is accepting,
and it is stated below rather than softened.

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
- The synchronization path can now fail to apply a notebook write under contention. **Nothing
  re-drives the journal and no watermark records what was projected**, so that failure is permanent:
  a deleted notebook's inherited access can persist indefinitely and silently. This is the existing
  behaviour of every materialization failure, made reachable by contention rather than only by error,
  and it is keeplin-srv#75's to repair.
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
| 1 | A deterministic interleaving test pauses one of the nine after its serializable snapshot, materializes a notebook deletion through the synchronization path, and resumes. The expected outcome is pinned per interleaving rather than offered as a choice: when the deletion removes the destination's write authority the request is refused with the ordinary current-state refusal; when it removes an inherited principal the move is admitted and the refusal that the stale view produced does not occur | negative, forced interleaving | **Fails on the current tree.** An oracle phrased as "either replay or refuse" would pass against an implementation that always takes one branch, which is why each interleaving names its outcome |
| 2 | Three `delete_account` interleavings against a paused participant, each with its outcome pinned: **notebook owner** — Bob's inherited write is refused rather than committed, which is the fail-open case and the essential one; **grantee** — the refusal does not name the vanished principal; **target of a transfer** — the request does not surface an unenumerated `500` | negative, forced interleaving | Fails if the cascade can revoke authority a paused transaction still acts on. Named per case because the three differ materially and an unpinned oracle passes on whichever the implementation happens to produce |
| 3 | A structural writer inventory enumerates every writer of note rows, note shares, notebook rows, notebook shares and ownership, and fails when one is not in the protocol | structural | Fails if a new writer can be added outside the set, which is how both known writers arrived |
| 4 | A decoy writer of one guarded table, planted permanently in the test fixtures, is detected by the inventory on every run | standing check | Fails if the inventory stops deriving. A one-time mutation performed during the pull request proves nothing a year later, when someone can replace the derivation with a hand-written list and row 3 passes forever |
| 5 | Downgrading any one participant to `READ COMMITTED` fails a test | mutation | Fails if the protocol is asserted by spelling rather than by behaviour |
| 6 | The synchronization path retries `40001` within the same bound and, on exhaustion, applies no notebook write, emits telemetry and returns no HTTP status | failure injection | Fails if ADR 0002's `503` is copied onto a path that is not a request, or if a partial write survives. The row deliberately says nothing about the journal row: what happens to it is keeplin-srv#75's, and an acceptance row requiring it to remain present would be unsatisfiable under that issue's option A |
| 7 | `delete_account` under injected `40001` retries within the bound and returns `503` only on exhaustion, with the account not deleted | failure injection | Fails if the cascade partially commits or exhaustion is hidden |
| 8 | ADR 0002's existing evidence — the move interleaving, byte-equivalent refusal, rollback, replay, retry bound and exhaustion tests — still passes unchanged | regression | Fails if widening the set changed the behaviour ADR 0002 established |
| 9 | The synchronization path's throughput, latency and `40001` abort rate are measured before and after against a stated regression budget, and the budget is agreed before the measurement is taken | operational, budgeted | Fails if the measured regression exceeds the budget. A row that only requires a number to be recorded passes on a catastrophic result, which is why the budget precedes the measurement |
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
