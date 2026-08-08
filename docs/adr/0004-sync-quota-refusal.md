# 0004 — How the synchronization path refuses an over-quota change

- Status: proposed
- Date: 2026-08-08
- Decision owners: maintainer of `jsunyermias/keeplin-srv`
- Scope: keeplin-srv
- Issue: [keeplin-srv#145](https://github.com/jsunyermias/keeplin-srv/issues/145)
- Acceptance PR: link once the ADR is accepted
- Supersedes: [ADR 0003](0003-per-user-quota-serialization.md) **in part** — its invariants 2 and 3 as
  they apply to the synchronization path. See *What this supersedes, and why it must say so*.
- Superseded by: none

## Context and problem

`Verified at`: `keeplin-srv@7863d23` and `keeplin@1b92f5d`.

Accepted [ADR 0003](0003-per-user-quota-serialization.md) requires every path that creates a counted
object to consult the limit that counts it, and names the synchronization path as one of the four.
Its *Not decided* section leaves one thing open and requires it to be settled before implementation:

> **How the synchronization path refuses an over-limit change.** It is not an HTTP request, so `507`
> is not its natural refusal, and rejecting a synchronized change has consequences for convergence
> that the HTTP path does not have.

This ADR settles that, and in doing so it partially supersedes the decision that asked for it. That
is stated in the header rather than buried, because two review rounds established that the two cannot
both hold.

### What the protocol actually does

Eleven facts, each read out of the tree. They are listed first because every option below is shaped
by them, and because the first two drafts of this document were each wrong about the code they
reasoned over.

1. **The order is journal, then materialize, then fan out — but the last two run over the wrong
   set.** `// md:fn handle_incoming` calls `Store::append_changes`, which returns only the sequence
   numbers it newly inserted; if that list is non-empty it then calls `materialize(state, user_id,
   &changes)` at `sync.rs:272` and builds the fan-out frame from `changes.iter()` at `sync.rs:274`.
   Both use the **whole submitted slice**, not the newly inserted subset. On a batch that partially
   duplicates an earlier one, changes already journaled are materialized again and broadcast again.
2. **The journal stores the blob bytes, amplified, and may keep them forever.** `changes.payload` is
   `JSONB` and `keeplin-core`'s `Change::ResourceCreate` carries `data: Option<Vec<u8>>`, which
   `serde_json` encodes as an array of decimal integers — on the order of four times the blob's size.
   `Store::user_blob_bytes_excluding` sums `octet_length` over `resource_blobs` only, so no quota
   counts those journal bytes. `prune_delivered_changes` deletes a row only when it is older than the
   retention window **and** `seq <= COALESCE(MIN(device_cursors.last_seq), 0)` across the user's
   devices, so one device that never advances its cursor — or a device with no cursor row, which
   makes the minimum `0` — pins the entire journal indefinitely. The amplified copy is not
   time-bounded.
3. **`materialize` reports no outcome, but its completion is observed.** It returns `()`, logs each
   error and continues (`sync.rs:393`). Its caller `await`s it and does not broadcast until it
   returns. So the server cannot know *what* materialization did; it does know *when* it finished.
   The second draft of this ADR conflated those and built an impossibility argument on the confusion.
4. **Nothing repairs a skipped projection from the journal, and nothing records what was
   materialized.** There is no watermark separating journaled work from projected work. A skipped
   projection is not re-attempted by any repair path. It can nevertheless be re-attempted by
   accident: by fact 1, a later frame reusing the same `batch_id` with at least one new index causes
   the whole slice to be materialized again. That is a replay artefact, not a repair mechanism, and
   it is [keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75)'s ground.
5. **The client is fire-and-forget.** `DbBackend::send_changes` returns `Ok` once the socket accepts
   the frame, and `run_sync` then advances the `last_sync` watermark. A change the server discards is
   **never re-sent**.
6. **The client discards any frame whose `type` is not `"changes"`**, silently and without a log. A
   new frame type is backward compatible with every deployed client, and equally invisible to it.
7. **Notes are not materialized from synchronization.** `Change::NoteCreate`, `NoteUpdate` and
   `NoteDelete` are `Ok(())` no-ops (`sync.rs:388`). Only `max_user_storage_bytes` is reachable
   through this path.
8. **`Resource.size` is client-declared and is not the byte count.** Nothing reconciles it with
   `data`. A measurement that trusts it is defeated by a client that declares a small number.
9. **Deleting a resource is a tombstone that keeps the blob, and a metadata-only create resurrects
   it.** `Store::delete_resource` runs `UPDATE resources SET deleted_at = …` and leaves
   `resource_blobs` untouched. `user_blob_bytes_excluding` counts only rows with
   `deleted_at IS NULL`, so the retained blob stops counting. `Store::upsert_resource_meta` writes
   `ON CONFLICT (id) DO UPDATE SET … deleted_at = EXCLUDED.deleted_at`, so a `ResourceCreate`
   carrying **no data at all** that wins the last-writer comparison clears the tombstone and the
   retained blob counts again. **Zero bytes on the wire, arbitrary bytes added to the counted
   total.**
10. **`put_resource_blob` replaces.** `ON CONFLICT (resource_id) DO UPDATE SET data = EXCLUDED.data`.
    For a resource that already has a blob the counted delta is `new − old`, which may be negative.
    Facts 9 and 10 together mean the quantity a quota check must measure is the **net change in
    counted bytes**, and that `data.len()` is that quantity only in the case where the resource is
    new.
11. **A refused create leaves nothing dangling.** There is no `ResourceUpdate` variant.
    `Store::delete_resource` returns `Ok(false)` for a row that does not exist. `resources.note_id`
    carries no foreign key. A later `ResourceDelete` naming a never-admitted resource is therefore a
    clean no-op rather than a constraint failure — which is the narrow claim; the client can and does
    create tombstones for resources the server never saw.

Canonical [keeplin ADR 0001](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0001-current-sync-delivery.md)
records the consequence of facts 5 and 6 as **unconfirmed delivery**.

### What the earlier drafts got wrong

Recorded because the corrections are why this decision is narrower and more expensive than the one
first written, and because a document that hides its revisions is worth less than one that shows
them. Both rounds were independent and neither is the author.

**Round 1 found the mechanism vacuous.** The first draft took ADR 0003's advisory lock at ingress and
claimed it serialized quota-bearing writes. It does not: ADR 0003's lock works because the lock, the
deciding read and the write are one transaction, and at ingress the write is elsewhere. Serializing
two reads that observe the same state and decide identically changes nothing. It also found that one
batch carrying two resources, each within the headroom and jointly over it, violated the invariant
with no concurrency at all, and that a verification row demanded an outcome a conforming
implementation cannot produce.

**Round 2 read the tree and found four things neither of us could have argued from the document.**
Fact 9's resurrection, which defeats a `data.len()` measurement with a zero-byte change. Fact 1's
slice mismatch, which means filtering a change out of the batch before `append_changes` would
compress every later `batch_index` and would fan out changes that were never journaled. Fact 2's
pruning predicate, which makes the amplified journal copy potentially permanent rather than bounded
by the retention window. And a fourth mechanism for closing the concurrent race that the second
draft's impossibility argument had missed, discussed below.

It also found the contradiction with ADR 0003 that the header now records.

### Why the concurrent bound is not established here

For a deciding read at ingress to be safe against a batch admitted a moment earlier, it must account
for bytes that are journaled but not yet counted. Four mechanisms exist:

- read the pending amount from the journal, which requires a watermark separating journaled work from
  projected work — fact 4 says none exists, and creating one is a projection state machine;
- hold a reservation until materialization completes, which requires materialization to have an
  observable completion **per change** — fact 3 gives completion of the batch but no outcome, so a
  reservation could be released on a projection that silently failed;
- write the blob in the transaction that decides, which is #75's option A by definition;
- **take a session-scoped `pg_advisory_lock` at ingress on one connection, hold it across the awaited
  `materialize`, and release it after.** Fact 3 makes this expressible, and round 2 is right that it
  is neither #75's A nor B.

The fourth is rejected on its costs rather than declared impossible, which the second draft got
wrong. A session-scoped lock is not released by transaction end, so a process crash or a dropped
connection between acquisition and release **strands it until the backend exits** — the stranded-lock
failure mode ADR 0003 rejected option 4 for, reintroduced deliberately. It holds a pool connection
for the duration of an entire batch's materialization, including arbitrarily large blob writes, on a
path with no backpressure ([keeplin-srv#77](https://github.com/jsunyermias/keeplin-srv/issues/77)).
And by fact 3 it would release on completion regardless of whether the projection succeeded, so the
reservation it implements can be wrong in the unsafe direction.

So the concurrent bound is obtainable, and every way of obtaining it either decides #75 or buys the
guarantee with a stranded-lock and connection-holding cost this ADR judges worse than the residue.
That is a weaker and more honest claim than the second draft's.

### The boundary this decision must not cross

keeplin-srv#75 requires its own prior ADR choosing between (**A**) journal and all its projections in
one PostgreSQL transaction and (**B**) a durable outbox with an idempotent projection worker, and
reserves that choice for the maintainer.

[keeplin#150](https://github.com/jsunyermias/keeplin/issues/150) owns the acknowledgement protocol,
and #75's own sketch already names a stable `NACK`. Any refusal frame here is a fragment of that
design arriving early.

### What this supersedes, and why it must say so

ADR 0003's invariant 2 requires that a quota-bearing write commit only if the total was read inside
the same transaction and under the lock, and its invariant 3 requires lock, read and write to be one
transaction. On the synchronization path the write is at materialization and the decision is at
ingress, so **neither can hold**, and this ADR does not take the lock.

ADR 0003 is accepted and its body is immutable historical record. The registry's mechanism for this
situation is supersession by a later accepted decision, not amendment, so the header records that
this ADR supersedes ADR 0003 in part: invariants 2 and 3, on the synchronization path only. ADR
0003's invariant 1 — that no path creates a counted object without consulting the limit — is
**strengthened** rather than weakened, since this is the decision that makes it true on the fourth
path.

Accepting this ADR therefore means accepting that an invariant accepted earlier today does not hold
on one of its four paths. That is stated plainly because the alternative is a registry in which two
accepted decisions disagree and nothing records which one governs.

## Forces and requirements

- The bypass is unbounded and trivially reachable today: one connection, no concurrency. Closing it
  should not wait on #75, which is blocked on its own ADR and tied to a protocol bump.
- Whatever is decided must remain correct under either outcome of #75.
- **The decision must not claim serialization it does not have**, which is the failure
  [keeplin-srv#138](https://github.com/jsunyermias/keeplin-srv/issues/138) records.
- The measured quantity must be the net change in counted bytes, not the bytes on the wire.
- Filtering a batch must not corrupt the journal's identity scheme or the fan-out.
- A refused change must not consume the storage the refusal exists to protect.
- The refusal must not leave the server missing an object every other device holds.
- The HTTP contract is untouched: `507 QuotaExceeded` stays exactly as it is.
- No client is required to change for this decision to take effect.

## Threat model

**Asset.** Server storage capacity, and the per-user bound `max_user_storage_bytes` asserts.

**Trust boundary.** An authenticated device's WebSocket frames, interacting with PostgreSQL.

**Adversary.** An authenticated user. Today the capability required is to synchronize rather than to
upload over HTTP — no timing, no privilege, repeatable and unbounded. After this decision the
capability required is concurrency: several batches in flight, available to anyone who opens more
than one connection.

**Capabilities and consequence.** Today a user's stored blob total is unbounded by the declared limit
through this path. After this decision a sequential client is bounded, and a concurrent one exceeds
the limit by up to the headroom times the number of batches in flight. That is a reduction from
unbounded-by-construction to bounded-by-concurrency-width, not an elimination.

**Also not bounded by any decision here.** Fact 2's journal amplification, which is roughly four
times the blob, is uncounted, and by fact 2's pruning predicate it is not even time-bounded: a single
device that never advances its cursor pins every journal row for that user. It is not
adversary-dependent — a fully compliant user causes it — and it is in scope for
[keeplin-srv#145](https://github.com/jsunyermias/keeplin-srv/issues/145), which stays open after this
ADR is decided precisely because this ADR does not address it.

**Accepted leakage.** A refusal is observable to the refusing device as a frame it may ignore, and to
other devices as the absence of a resource. Neither reveals another user's state.

**Out of scope.** Per-connection quotas and backpressure, which keeplin-srv#77 owns; delivery
guarantees, which keeplin#150 owns.

## Options considered

### Option 1 — Refuse inside `materialize`, silently

Rejected on two grounds. It runs after the journal commit, so the bytes are already stored and
amplified (fact 2) — the refusal protects a table and not a disk. And by facts 1 and 3 the batch is
broadcast regardless of what materialization did, and stays in the journal for `deliver_backlog` to
deliver on any other device's reconnection, so **every other device receives and stores the resource
while the server does not**.

### Option 2 — Refuse at ingress, before the journal insert, silently

The bytes never enter the journal and nothing is fanned out, so the server and every other device
agree that the resource does not exist.

Its costs are fact 5 — the origin device advanced its watermark and will never re-send — the
concurrent bound it does not establish, and the prerequisite in fact 1 that it forces.

### Option 3 — Option 2, plus a refusal frame

As option 2, and the server emits a frame naming the refused change. Fact 6 makes this backward
compatible. It does not, by itself, make any client act. Its cost is a fragment of the
acknowledgement vocabulary keeplin#150 owns, arriving before that design exists.

### Option 4 — Refuse the whole batch and close the connection

Rejected. One over-limit resource would discard unrelated changes, and fact 5 means none would be
re-sent.

### Option 5 — Admit it, mark it, and repair later

The most useful behaviour for a real user. Rejected on ownership: a projection state machine with a
repair worker is exactly what #75 must decide.

### Option 6 — Decide nothing here and fold the sync half into #75's ADR

Implement ADR 0003's HTTP half now, and leave the sync path enforced by whatever #75 lands.

**Two review rounds have made this materially stronger than the first draft allowed**, and honesty
requires saying so in the document rather than only in the recommendation. Choosing option 3 now
costs: a partial supersession of an ADR accepted the same day; a prerequisite fix to `handle_incoming`
that has nothing to do with quota; a net-delta measurement that must read the current state of every
resource id in the batch; and a guarantee that remains sequential-only. Option 6 pays none of those
and reaches a stronger end state in one step.

What it costs is time on an unbounded hole: the storage limit stays bypassable with a single
connection until an ADR that has not been written, for an issue inside a critical epic that must ship
in lockstep with a protocol bump.

## Decision and justification

> This ADR is `proposed`. It records a recommendation and does not authorize implementation. Only
> the maintainer may accept or reject it.

**Proposed decision: adopt Option 3 — refuse at ingress, say so on the wire, and claim only the
sequential bound.** The maintainer is asked to weigh it against option 6 with the costs above
explicit, because those costs were not visible when the direction was chosen.

**Part zero — a prerequisite that is not about quota.** By fact 1, `handle_incoming` materializes and
fans out the submitted slice rather than the journaled subset, and `append_changes` derives
`batch_index` from the position in that slice. Removing an element would compress every later index,
so a retry of the same `batch_id` after the headroom changed could place a different payload at an
index already taken and have it silently dropped by `ON CONFLICT DO NOTHING`, and a change could be
broadcast with no journal row behind it. **Admission control at ingress is not implementable until
each change keeps its original index and materialization and fan-out derive from what was actually
inserted.** This is a defect in its own right, independent of quota, and belongs to keeplin-srv#75
rather than to a new issue.

**Part one — the synchronization path performs admission control before journaling, on the net
delta.** The batch is walked in order. For each `Change::ResourceCreate` — including one carrying no
data, by fact 9 — the implementation computes the change in counted bytes it would cause: the bytes
added for a new resource, the difference for a resource whose blob is replaced (fact 10), and the
whole retained blob for a tombstoned resource the change would resurrect (fact 9). Bytes are taken
from `data`, never from `Resource.size` (fact 8). Each delta is accumulated against a per-batch
running total compared with the live total read once for the batch. A change that fits is admitted; a
change that does not is refused, and **evaluation continues**, so a later change that still fits is
admitted rather than dropped for following a refusal.

A refused change is never journaled, never fanned out and never materialized. On this path no counted
object is created at all.

**Part two — the refusal is stated on the wire.** The server sends a frame identifying the refused
change and the reason. Its vocabulary is provisional and subordinate to keeplin#150.

**Part three — no advisory lock is taken, and the concurrent bound is not claimed.** ADR 0003's lock
exists to make a deciding read and the write it authorizes mutually exclusive; here they are in
different transactions, so it would serialize two reads that decide identically. The fourth mechanism
that would work is rejected above on stranded-lock and connection-holding costs.

**Part four — the obligation attached to keeplin-srv#75.** Whichever option #75 accepts must carry
the quota decision into the transaction that performs the projection. This **narrows #75's option
space**: it forecloses the outcome in which admission stays at ingress permanently. Stated as a
constraint, not as deference.

The invariants proposed are:

1. No `Change::ResourceCreate` whose net effect on counted bytes exceeds the headroom remaining after
   every earlier admitted change in the same batch is journaled, fanned out or materialized. Net
   effect includes resurrection of a tombstoned resource and replacement of an existing blob, and is
   not the length of `data` except for a new resource.
2. Bytes are measured from `data`. `Resource.size` is never the basis of a quota decision.
3. A refused change causes no journal row, no new or mutated row in `resources` or `resource_blobs`,
   and no frame in any other device's stream, live or on backlog delivery. Rows that existed before
   the refused change are unchanged, not absent.
4. Refusing one change admits every other change in the batch that fits, evaluated in batch order.
5. Every change retains its original position, so `batch_index` identity is stable across retries of
   the same `batch_id`; materialization and fan-out derive from what `append_changes` inserted.
6. The refusal is reported to the origin device in a frame a client that does not understand it may
   ignore without error.
7. The HTTP `507 QuotaExceeded` contract is byte-identical to today's.
8. No advisory lock is taken on this path.
9. The bound established is sequential. Concurrent batches may jointly exceed the limit, and that is
   a stated non-guarantee rather than an implementation defect.

### Costs, stated rather than implied

**It supersedes part of a decision accepted the same day.** Recorded in the header and argued above.

**It requires a fix to `handle_incoming` that quota did not cause**, and that fix is inside the area
#75 must reason about.

**The measurement is no longer a length.** It reads the current state of every resource id in the
batch, which is a query per distinct id at ingress on a path that does none today.

**The concurrent overrun is real and this decision does not remove it.** Invariant 9 exists so that
no later reader mistakes this for the quota holding.

**The user is not told by anything they can see.** Until `keeplin` acts on the frame, a refused
resource stays on the origin device and surfaces to the user as nothing at all. This decision makes
the refusal representable, not visible.

**A change that is never re-sent is lost.** Fact 5.

**A refusal frame before the acknowledgement protocol is vocabulary debt**, with nothing forcing its
reconciliation since no client acts on it.

### Not decided

- The frame's exact name and field set beyond the requirement that it be ignorable.
- What a client does with the refusal — `keeplin` work, belonging with keeplin#151.
- Whether the journal's amplified copy should count against the quota or not be stored in that form.
  In scope for keeplin-srv#145, which remains open after this decision.
- Where the blob write finally sits, and with it the concurrent bound. keeplin-srv#75, with the
  obligation in part four attached.

## Consequences and risks

- A user currently over `max_user_storage_bytes` through synchronization will have those changes
  refused. Intended, and a behaviour change.
- Refused resources are silently absent to every user of a client that has not been updated.
- The concurrent residue remains until keeplin-srv#75 lands.
- The journal amplification remains uncounted and, by fact 2, potentially unpruned.
- Fact 4's general defect is untouched: this decision adds no new way for a projection to be lost and
  repairs none of the existing ones.
- The registry carries a partial supersession, which is a cost paid in comprehensibility every time
  someone reads ADR 0003 afterwards.

## Compatibility, migration, and rollback

No schema change and no migration. `keeplin-core` is untouched and its pin does not move.

`PROTOCOL_VERSION` does not move. The added frame is server-to-client and additive, and fact 6
establishes that a client which does not know it discards it without error.

Rollback is removing the ingress check, returning the storage limit to bypassable by synchronization
with a single connection. Part zero's fix should not be rolled back with it: it corrects a defect
that exists independently.

## Verification plan

| # | Evidence | Kind | What fails if the decision is violated |
|---|---|---|---|
| 1 | A `ResourceCreate` whose net counted bytes would exceed `max_user_storage_bytes` is refused; afterwards no journal row exists for it, and no row in `resources` or `resource_blobs` is created or mutated by it. Run for three cases: a new id, an id with a live row, and an id with a tombstoned row | negative | Fails if the limit remains bypassable by synchronizing. The three cases exist because a refusal must leave a pre-existing row **intact**, not absent, so an assertion of "no row" would be unsatisfiable for two of them |
| 2 | A `ResourceCreate` carrying **no data** that would resurrect a tombstoned resource whose retained blob does not fit is refused | negative | Fails if the measurement is the length of `data`, under which a zero-byte change adds arbitrary counted bytes (fact 9) |
| 3 | A `ResourceCreate` replacing an existing blob is measured on the difference, and one that shrinks a blob is admitted even when the user is at the limit | positive, negative | Fails if the measurement double-counts a replacement (fact 10) |
| 4 | A `ResourceCreate` declaring a small `Resource.size` and carrying oversized `data` is refused | negative | Fails if the measurement trusts the client-declared field |
| 5 | One batch carrying two `ResourceCreate`s, each within the live headroom and jointly exceeding it: the second is refused | negative | Fails if the deciding read is taken per change against the live total instead of against a per-batch running total. Single connection, no concurrency |
| 6 | The same batch's other changes are journaled and materialized, including one that fits and arrives **after** the refused one | positive | Fails if evaluation stops at the first refusal (invariant 4) |
| 7 | A batch whose middle change is refused, re-sent with the same `batch_id` after headroom increased: the previously refused change is journaled at its original index and no other payload was displaced | negative, recovery | Fails if refusal compresses `batch_index`, which silently drops a payload through `ON CONFLICT DO NOTHING` (invariant 5) |
| 8 | A batch that partially duplicates an earlier one materializes and fans out only the changes `append_changes` inserted | negative | Fails if materialization and fan-out run over the submitted slice, which broadcasts changes with no journal row behind them (part zero) |
| 9 | A second device receives no frame naming the refused change, on the live fan-out and on backlog delivery after reconnecting | negative, recovery | Fails if the refusal leaves other devices holding an object the server does not, which is option 1's divergence |
| 10 | Moving the ingress check to after `append_changes` makes rows 1 and 9 fail | mutation | Fails if the tests pass with the check in a position that journals and broadcasts before deciding |
| 11 | Replacing the per-batch running total with the live total re-read per change makes row 5 fail | mutation | Fails if row 5 is insensitive to the accumulation |
| 12 | Replacing the net-delta computation with `data.len()` makes rows 2 and 3 fail | mutation | Fails if those rows pass against the measurement this decision replaced |
| 13 | Two concurrent batches from one user, each within the limit and jointly exceeding it, with a test-controlled rendezvous: **both are admitted, and the test asserts that outcome** and the resulting overrun | concurrency, forced interleaving | Fails if the residue is larger than invariant 9 states, or if an unauthorized mechanism was introduced. This row pins a **non**-guarantee deliberately: a row asserting "exactly one is admitted" would fail against a correct implementation |
| 14 | The refusal frame is emitted to the origin device and identifies the change | positive | Fails if the refusal is unrepresentable on the wire |
| 15 | A client built from `keeplin@1b92f5d`, which knows only `type == "changes"`, completes a sync cycle without error when the frame is present | compatibility | Fails if the frame breaks a deployed client |
| 16 | A concurrent session holds every advisory lock the repository's lock-domain contract defines, and a synchronization batch still completes materialization | behavioral | Fails if an advisory lock was introduced on this path (invariant 8). A structural assertion over `materialize`'s body was rejected: it passes when the acquisition moves one frame down, which is the class of evidence keeplin-srv#138 discredits |
| 17 | No HTTP status is produced on the synchronization path, and an HTTP `507` body is byte-identical to today's | compatibility | Fails if the HTTP refusal is copied onto a path that is not an HTTP request, or if the HTTP contract moved. Stated as the prohibited surface rather than a forbidden symbol name, because the frame's reason may legitimately derive from the same error type |
| 18 | `./scripts/check-docs.sh` passes with every changed source companion synchronized | documentation | Fails if implementation and documentation diverge |
| 19 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` pass | repository | Fails on behavioral regressions, warnings, or formatting drift |

Rows 10 to 12 are mutation evidence and row 13 requires a forced rendezvous, for the reason
keeplin-srv#138 records: during ADR 0002's phase 1 a test named for exactly the property it was meant
to establish passed for a full review round while that property was violated.

Rows 2 and 3 exist because the second draft's measurement would have passed every other row in this
table while being wrong. Row 9 exists because it is the one observable that separates this decision
from option 1.

Invariant 7 is deliberately **not** given a row asserting that no transaction appears inside
`materialize`. Round 2 established that no test distinguishes an ordinary transaction there from its
absence in a way that survives a callee, so a row claiming to enforce it would be decoration. The
prohibition is stated as intent; the enforceable part is invariant 8, which row 16 covers.

No migration or format-recovery evidence is required because this decision changes neither surface.

## Equivalent decision in the other repository

None is required now. The frame is additive, `PROTOCOL_VERSION` does not move, and no `keeplin`
change is needed for this decision to take effect.

The moment `keeplin` is required to *act* on the refusal, the protocol stops being server-local and
that becomes a canonical decision in `keeplin`, versioned there. It belongs with keeplin#150 and
keeplin#151 rather than with this ADR, and this paragraph exists so that boundary is crossed
deliberately rather than by accretion.
