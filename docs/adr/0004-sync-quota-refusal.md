# 0004 — How the synchronization path refuses an over-quota change

- Status: proposed
- Date: 2026-08-08
- Decision owners: maintainer of `jsunyermias/keeplin-srv`
- Scope: keeplin-srv
- Issue: [keeplin-srv#145](https://github.com/jsunyermias/keeplin-srv/issues/145)
- Acceptance PR: link once the ADR is accepted
- Supersedes: none
- Superseded by: none

## Context and problem

`Verified at`: `keeplin-srv@6568e46` and `keeplin@1b92f5d`.

Accepted [ADR 0003](0003-per-user-quota-serialization.md) requires every path that creates a counted
object to consult the limit that counts it, and names the synchronization path as one of the four.
Its *Not decided* section leaves one thing open and requires it to be settled before implementation:

> **How the synchronization path refuses an over-limit change.** It is not an HTTP request, so `507`
> is not its natural refusal, and rejecting a synchronized change has consequences for convergence
> that the HTTP path does not have.

This ADR settles exactly that. It changes nothing ADR 0003 decided.

### What the protocol actually does

Nine facts, each read out of the tree rather than assumed. They are listed first because every option
below is shaped by them, and because the first draft of ADR 0003 was wrong twice by reasoning about
this repository without reading it.

1. **The order of operations is journal, then materialize, then fan out.** `// md:fn handle_incoming`
   calls `Store::append_changes`, which commits; then `materialize` at `sync.rs:272`; then the
   broadcast. The broadcast is unconditional with respect to the materialization outcome, because
   fact 3 leaves nothing for it to be conditional on.
2. **The journal stores the blob bytes, amplified.** `changes.payload` is `JSONB`
   (`migrations/0001_initial.sql`) and `keeplin-core`'s `Change::ResourceCreate` carries
   `data: Option<Vec<u8>>`, which `serde_json` encodes as an array of decimal integers — on the order
   of four times the blob's size. `Store::user_blob_bytes_excluding` sums `octet_length` over
   `resource_blobs` only, so **no quota counts those journal bytes**, before or after this decision.
   They are removed by `prune_delivered_changes` once `retention_days` elapses.
3. **`materialize` cannot refuse anything.** It returns `()`, logs the error and continues
   (`sync.rs:393`). Its single caller ignores it, which is why fact 1's broadcast is unconditional.
4. **Nothing re-materializes from the journal, and nothing records what has been materialized.**
   A projection that is skipped is absent from the server permanently; there is no repair path and no
   watermark separating journaled work from projected work. This is the defect
   [keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75) exists for, and fact 4's
   second half is load-bearing below.
5. **The client is fire-and-forget.** `DbBackend::send_changes` returns `Ok` once the socket accepts
   the frame, and `run_sync` then advances the `last_sync` watermark. A change the server discards is
   **never re-sent**: it falls behind the watermark and `get_changes_since` will not collect it again.
6. **The client discards any frame whose `type` is not `"changes"`**, silently and without a log. A
   new frame type is therefore backward compatible with every deployed client, and equally invisible
   to it.
7. **Notes are not materialized from synchronization.** `Change::NoteCreate`, `NoteUpdate` and
   `NoteDelete` are `Ok(())` no-ops (`sync.rs:388`). Only `max_user_storage_bytes` is reachable
   through this path; `max_notes_per_user` is not.
8. **`Resource.size` is a client-declared field and is not the byte count.** It travels in the
   change's metadata and nothing reconciles it with `data`. Any measurement must use the length of
   `data`, never `size`, or the limit is defeated by a client that declares a small number.
9. **A refused `ResourceCreate` leaves nothing dangling.** There is no `ResourceUpdate` variant.
   `Store::delete_resource` returns `Ok(false)` for a row that does not exist — a clean no-op, not an
   error. `resources.note_id` carries no foreign key, and `resource_blobs.resource_id` references
   `resources`, so no later change can reference a resource that was never admitted and produce a
   constraint failure.

Canonical [keeplin ADR 0001](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0001-current-sync-delivery.md)
already records the consequence of facts 5 and 6 as **unconfirmed delivery**: no application-level
acknowledgement exists in either direction.

### What the first draft of this ADR got wrong

Recorded because the corrections are the reason the decision below is narrower than the one first
written, and because an ADR that hides its own revisions is worth less than one that shows them.

The first draft took ADR 0003's per-user advisory lock at ingress and claimed it serialized
quota-bearing synchronization writes. **It does not, and the claim was close to vacuous.** ADR 0003's
lock works because the lock, the deciding read and the write it authorizes are one transaction. At
ingress the write is not there: the blob reaches `resource_blobs` later, at materialization, in a
different transaction. Serializing two reads that both observe the same pre-write state changes
nothing about what they decide. The first draft then described the residue as "the same bound and the
same class as the HTTP-only skew" — false, because the HTTP skew is bounded precisely by the property
this path lacks.

It was also silent on two changes inside one batch, so the most ordinary input imaginable — one
frame, one connection, two resources each within the headroom and jointly over it — violated the
invariant the document claimed to establish, with no concurrency at all.

Both were found by independent review before acceptance. Their consequence is that the concurrent
bound is **not obtainable on this path at all** without deciding keeplin-srv#75, which is argued
below rather than asserted.

### Why the concurrent bound cannot be established here

For a deciding read at ingress to be safe against another batch admitted a moment earlier, it must
count bytes that are journaled but not yet in `resource_blobs`. Three ways exist to know that number,
and each one is #75's decision:

- read it from the journal, which requires a watermark separating journaled work from projected work
  — fact 4 says none exists, and creating one is a projection state machine;
- hold a reservation in memory or in a table until materialization completes, which requires
  materialization to have a completion the server observes — fact 3 says it does not;
- write the blob in the transaction that decides, which is #75's option A by definition.

So the honest position is not "this ADR chooses not to close the race". It is **that the race cannot
be closed on this path until #75 is decided**, and any decision claiming otherwise is claiming a
mechanism that does not exist yet.

### The boundary this decision must not cross

[keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75) requires its own prior ADR
choosing between (**A**) journal and all its projections in one PostgreSQL transaction and (**B**) a
durable outbox with an idempotent projection worker, and reserves that choice for the maintainer.
Every mechanism in the list above is one of those two in disguise.

Similarly, [keeplin#150](https://github.com/jsunyermias/keeplin/issues/150) owns the acknowledgement
protocol, and #75's own sketch already names a stable `NACK` for a permanently invalid change. Any
refusal frame this ADR introduces is a fragment of that design arriving early.

## Forces and requirements

- The bypass is unbounded and trivially reachable today: one connection, no concurrency, no limit.
  Closing it should not wait on the resolution of #75, which is blocked on its own ADR and tied to a
  protocol bump.
- Whatever is decided must remain correct under either outcome of #75.
- **The decision must not claim serialization it does not have.** ADR 0003's invariant 3 is
  unreachable on this path, and a document that implied otherwise would be the failure
  [keeplin-srv#138](https://github.com/jsunyermias/keeplin-srv/issues/138) records: a claim whose test
  passes while the property is violated.
- A refused change must not consume the storage the refusal exists to protect.
- The refusal must not leave the server missing an object every other device holds.
- The HTTP contract is untouched: `507 QuotaExceeded` stays exactly as it is.
- No client is required to change for this decision to take effect.

## Threat model

**Asset.** Server storage capacity, and the per-user bound `max_user_storage_bytes` asserts.

**Trust boundary.** An authenticated device's WebSocket frames, interacting with PostgreSQL.

**Adversary.** An authenticated user. Today the capability required is to synchronize rather than to
upload over HTTP — no timing, no privilege, repeatable and unbounded. After this decision the
capability required is **concurrency**: several batches in flight at once, which is available to
anyone who opens more than one connection.

**Capabilities and consequence.** Today a user's stored blob total is unbounded by the declared
limit through this path. After this decision a sequential client is bounded by the limit, and a
concurrent one exceeds it by up to the headroom times the number of batches in flight. That is a
reduction from unbounded-by-construction to bounded-by-concurrency-width, not an elimination, and the
verification plan measures it rather than assuming it.

**Also not bounded, by any decision here.** Fact 2's journal amplification: a fully compliant user
storing a resource costs the server roughly five times its size for `retention_days`, and the quota
measures one fifth of it. It is not adversary-dependent, and it is in scope for
[keeplin-srv#145](https://github.com/jsunyermias/keeplin-srv/issues/145), which stays open after this
ADR is decided precisely because this ADR does not address it.

**Accepted leakage.** A refusal is observable to the refusing device as a frame it may ignore, and to
other devices as the absence of a resource. Neither reveals another user's state.

**Out of scope.** Per-connection quotas, batch size limits and backpressure, which
[keeplin-srv#77](https://github.com/jsunyermias/keeplin-srv/issues/77) owns; and delivery
guarantees, which keeplin#150 owns.

## Options considered

### Option 1 — Refuse inside `materialize`, silently

Add the check to the `ResourceCreate` arm; on over-limit, skip `put_resource_blob` and log.

Rejected on two grounds. It runs after the journal commit, so the bytes are already stored and
amplified (fact 2) — the refusal protects a table and not a disk. And by facts 1 and 3 the batch is
broadcast regardless of what materialization did, and remains in the journal for `deliver_backlog` to
deliver on any other device's reconnection, so **every other device receives and stores the resource
while the server does not**: divergence in the worst available direction.

The first draft rejected it partly by saying the check "runs after the fan-out is queued", which
contradicts fact 1's ordering. The conclusion survives; the mechanism above is the correct one.

### Option 2 — Refuse at ingress, before the journal insert, silently

Inspect the batch in `handle_incoming`; drop an over-limit `ResourceCreate` before `append_changes`,
journal the rest.

The bytes never enter the journal and nothing is fanned out, so the server and every other device
agree that the resource does not exist. Nothing about materialization is decided.

Its costs are fact 5 — the origin device advanced its watermark and will never re-send, so the
resource lives on that one device forever — and the concurrent bound argued above, which it does not
establish.

### Option 3 — Option 2, plus a refusal frame

As option 2, and the server emits a frame naming the refused change. Fact 6 makes this backward
compatible: deployed clients ignore unknown frame types. It does not, by itself, make any client act.

Its cost is that it introduces a fragment of the acknowledgement vocabulary keeplin#150 owns, before
that design exists.

### Option 4 — Refuse the whole batch and close the connection

Rejected. One over-limit resource would discard unrelated changes in the same batch, and fact 5 means
none of them would be re-sent. It destroys more and informs no more than option 2.

### Option 5 — Admit it, mark it, and repair later

Journal the change, materialize nothing, record a pending state, and complete the write when the user
frees space.

Not rejected on merit — it is the most useful behaviour for a real user. Rejected on ownership: a
projection state machine with a repair worker is exactly what #75 must decide.

### Option 6 — Decide nothing here and fold the sync half into #75's ADR

Implement ADR 0003's HTTP half now, and leave the sync path enforced by whatever #75 lands.

**This option is stronger than the first draft of this ADR allowed**, and the review that produced
the corrections above is why. Once it is established that the concurrent bound is unobtainable
without #75, options 2 and 3 deliver a partial guarantee and #75 has to revisit the same code to
finish it. Doing it once, later, is a coherent position.

What it costs is time on an unbounded hole: the storage limit stays bypassable with a single
connection until an ADR that has not been written, for an issue inside a critical epic that must ship
in lockstep with a protocol bump.

## Decision and justification

> This ADR is `proposed`. It records a recommendation and does not authorize implementation. Only
> the maintainer may accept or reject it.

**Proposed decision: adopt Option 3 — refuse at ingress, say so on the wire, and claim only the
sequential bound.**

**Part one — the synchronization path performs admission control before journaling.** In
`handle_incoming`, before `append_changes`, the batch is walked in order. For each
`Change::ResourceCreate` carrying `data`, the length of `data` — never `Resource.size`, by fact 8 —
is added to a per-batch running total and compared against `max_user_storage_bytes` less the live
total read once for the batch. A change that fits is admitted and its bytes join the running total; a
change that does not is removed from the batch. Later changes are still evaluated, so a small
resource after a large refused one is admitted: **the admitted set is every change that fits as the
batch is walked, not the prefix before the first refusal.**

A refused change is never journaled, never fanned out and never materialized. On this path **no
counted object is created at all**: ADR 0003's invariants 1 and 2 hold because there is no write to
authorize, not because a write was guarded.

**Part two — the refusal is stated on the wire.** The server sends a frame identifying the refused
change and the reason. Its vocabulary is provisional and subordinate to keeplin#150: when the
acknowledgement protocol lands, this frame is reconciled into it rather than kept alongside it.

**Part three — no advisory lock is taken, and the concurrent bound is not claimed.** ADR 0003's
lock exists to make a deciding read and the write it authorizes mutually exclusive. Here they are in
different transactions, so the lock would serialize two reads that observe the same state and decide
identically. Taking it would cost latency and buy a false sense of coverage. **It is deliberately not
taken, and this is a departure from ADR 0003's part two that the maintainer is being asked to accept
explicitly rather than by implication.**

What this establishes is therefore precise: **`max_user_storage_bytes` bounds a user's
`resource_blobs` total against a sequential client, and bounds it against a concurrent one only up to
the headroom times the number of batches in flight.** Closing the remainder requires one of the three
mechanisms listed in *Why the concurrent bound cannot be established here*, each of which is #75's
decision.

**Part four — the obligation attached to keeplin-srv#75.** Whichever option #75 accepts must carry
the quota decision into the transaction that performs the projection, so that the deciding read and
the blob write are finally one transaction. This **narrows #75's option space**: it forecloses the
outcome in which admission stays at ingress permanently and the concurrent residue is accepted
forever. That narrowing is stated as a constraint rather than presented as deference, because the
first draft called it "ceding the choice" and it is not.

The invariants proposed are:

1. No `Change::ResourceCreate` whose `data` length exceeds the headroom remaining after every earlier
   admitted change in the same batch is journaled, fanned out or materialized.
2. The measurement uses the length of `data`. `Resource.size` is client-declared and is never the
   basis of a quota decision.
3. A refused change leaves no row in `changes`, no row in `resources` or `resource_blobs`, and no
   frame in any other device's stream, live or on backlog delivery.
4. Refusing one change in a batch admits every other change in it that fits, evaluated in batch
   order; the batch is not discarded and evaluation does not stop at the first refusal.
5. The refusal is reported to the origin device in a frame that a client which does not understand it
   may ignore without error.
6. The HTTP `507 QuotaExceeded` contract is byte-identical to today's.
7. No advisory lock is taken on this path, and no transaction is placed inside `materialize`.
8. The bound established is sequential. Concurrent batches may jointly exceed the limit, and that is
   a stated non-guarantee rather than a defect of the implementation.

### Costs, stated rather than implied

**The concurrent overrun is real and this decision does not remove it.** It is smaller than today's
unbounded bypass and larger than what ADR 0003 establishes on HTTP. Invariant 8 exists so that no
later reader mistakes this for the quota holding.

**The user is not told by anything they can see.** Until `keeplin` acts on the frame, a refused
resource stays on the origin device, is absent everywhere else, and surfaces to the user as nothing
at all. This decision makes the refusal *representable*; it does not make it *visible*.

**A change that is never re-sent is a change that is lost.** Fact 5 means even a client that learns
of the refusal cannot recover the resource by syncing again without the outbox keeplin#151 designs.

**A refusal frame before the acknowledgement protocol is vocabulary debt.** Small, deliberate, and
recorded — with the observation that nothing forces its reconciliation, since no client acts on it.

**Reading the live total once per batch costs one query on a path that does none today.**

### Not decided

- The frame's exact name and field set beyond the requirement that it be ignorable, and its
  reconciliation with keeplin#150's acknowledgement vocabulary.
- What a client does with the refusal. That is `keeplin` work and belongs with
  [keeplin#151](https://github.com/jsunyermias/keeplin/issues/151), not to a new issue.
- Whether the journal's amplified copy of the blob should count against the quota, or should not be
  stored at all. In scope for keeplin-srv#145, which remains open after this decision.
- Where the blob write finally sits, and with it the concurrent bound. keeplin-srv#75, with the
  obligation in part four attached.

## Consequences and risks

- A user currently over `max_user_storage_bytes` through synchronization will have those changes
  refused. That is the intended correction and it is a behaviour change.
- Refused resources are silently absent to every user of a client that has not been updated. This is
  a worse user experience than a `507`, and it is the honest consequence of a protocol with no
  acknowledgement.
- The concurrent residue above remains until keeplin-srv#75 lands.
- The journal amplification remains uncounted by any quota.
- Fact 4's general defect is untouched: this decision adds no new way for a projection to be lost,
  and repairs none of the existing ones.

## Compatibility, migration, and rollback

No schema change and no migration. `keeplin-core` is untouched and its pin does not move.

`PROTOCOL_VERSION` does not move. The added frame is server-to-client and additive, and fact 6
establishes that a client which does not know it discards it without error, so no lockstep bump is
required. This is the one respect in which the decision is cheap, and it is cheap because the client
ignores everything it does not recognize — which is the same property that makes the refusal
invisible.

Rollback is removing the ingress check, returning the storage limit to bypassable by synchronization
with a single connection.

## Verification plan

| # | Evidence | Kind | What fails if the decision is violated |
|---|---|---|---|
| 1 | A `ResourceCreate` whose `data` would exceed `max_user_storage_bytes` is refused, and afterwards `changes`, `resources` and `resource_blobs` contain no row for it | negative | Fails if the storage limit remains bypassable by synchronizing, which is the defect |
| 2 | The same batch's other changes are journaled and materialized, including one that fits and arrives **after** the refused one | positive | Fails if one refusal discards unrelated work, or if evaluation stops at the first refusal instead of continuing (invariant 4) |
| 3 | One batch carrying two `ResourceCreate`s, each within the live headroom and jointly exceeding it: the second is refused and the stored total never exceeds the limit | negative | Fails if the deciding read is taken per change against the live total instead of against a per-batch running total. This is the single-connection, no-concurrency violation the first draft admitted |
| 4 | A `ResourceCreate` declaring a small `Resource.size` and carrying oversized `data` is refused | negative | Fails if the measurement trusts the client-declared field (invariant 2) |
| 5 | A second device connected to the same user receives no frame naming the refused change, on the live fan-out and on backlog delivery after reconnecting | negative, recovery | Fails if the refusal leaves other devices holding an object the server does not, which is option 1's divergence |
| 6 | Moving the ingress check to after `append_changes` makes rows 1 and 5 fail | mutation | Fails if the tests pass with the check in a position that journals and broadcasts before deciding |
| 7 | Replacing the per-batch running total with the live total re-read per change makes row 3 fail | mutation | Fails if row 3 is insensitive to the accumulation, which would make it an accident of the fixture rather than evidence |
| 8 | Two concurrent batches from one user, each within the limit and jointly exceeding it, with a test-controlled rendezvous: **both are admitted, and the test asserts that outcome** and the resulting overrun | concurrency, forced interleaving | Fails if the implementation silently acquired a mechanism this ADR does not authorize, or if the residue is larger than invariant 8 states. This row pins a **non**-guarantee, deliberately: a row asserting "exactly one is admitted" would fail against a correct implementation of this decision |
| 9 | The refusal frame is emitted to the origin device and identifies the change | positive | Fails if the refusal is unrepresentable on the wire |
| 10 | A client built from `keeplin@1b92f5d`, which knows only `type == "changes"`, completes a sync cycle without error when the frame is present | compatibility | Fails if the frame breaks a deployed client, which would make it a protocol break rather than an addition |
| 11 | A fault-injection test proves `materialize` holds no advisory lock and runs in no caller-supplied transaction: a concurrent session takes every advisory lock the repository's lock-domain contract defines and materialization still completes | behavioral | Fails if a transaction or lock has been introduced into materialization, in a callee or otherwise. A structural assertion over the function body was rejected: it passes when the transaction moves one frame down, which is exactly the class of evidence keeplin-srv#138 discredits |
| 12 | No HTTP status is produced on the synchronization path, and an HTTP `507` body is byte-identical to today's before and after this change | compatibility | Fails if the HTTP refusal is copied onto a path that is not an HTTP request, or if the HTTP contract moved. Stated as the prohibited surface rather than as a forbidden symbol name, because the refusal frame's reason may legitimately be derived from the same error type |
| 13 | `./scripts/check-docs.sh` passes with every changed source companion synchronized | documentation | Fails if implementation and documentation diverge |
| 14 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` pass | repository | Fails on behavioral regressions, warnings, or formatting drift |

Rows 6 and 7 are mutation evidence and row 8 requires a forced rendezvous, for the reason
keeplin-srv#138 records: during ADR 0002's phase 1 a test named for exactly the property it was meant
to establish passed for a full review round while that property was violated.

Row 5 exists because it is the one observable that separates this decision from option 1, and a test
suite that omitted it would pass identically against the design this ADR rejects.

Row 8 is unusual and deliberate. A verification plan that only pins guarantees lets a non-guarantee
drift into being assumed; this one fails if someone later believes the quota holds under concurrency
on this path.

No migration or format-recovery evidence is required because this decision changes neither surface.

## Equivalent decision in the other repository

None is required now. The frame is additive, `PROTOCOL_VERSION` does not move, and no `keeplin`
change is needed for this decision to take effect — fact 6 is what makes that true.

The moment `keeplin` is required to *act* on the refusal, the protocol stops being server-local and
that becomes a canonical decision in `keeplin`, versioned there. It belongs with
[keeplin#150](https://github.com/jsunyermias/keeplin/issues/150) and
[keeplin#151](https://github.com/jsunyermias/keeplin/issues/151) rather than with this ADR, and this
paragraph exists so that boundary is crossed deliberately rather than by accretion.
