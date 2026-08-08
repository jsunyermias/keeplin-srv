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

Seven facts, each checked against the tree rather than assumed. They are listed first because every
option below is shaped by them, and because the first draft of ADR 0003 was wrong twice by reasoning
about this repository without reading it.

1. **The order of operations is journal, then materialize, then fan out.** `// md:fn handle_incoming`
   calls `Store::append_changes`, which commits, then calls `materialize` at `sync.rs:272`, then
   broadcasts. Any check inside `materialize` runs after the batch is durable.
2. **The journal stores the blob bytes, amplified.** `changes.payload` is `JSONB`
   (`migrations/0001_initial.sql`) and `keeplin-core`'s `Change::ResourceCreate` carries
   `data: Option<Vec<u8>>`, which `serde_json` encodes as an array of decimal integers — on the order
   of four times the blob's size. `Store::user_blob_bytes_excluding` sums `octet_length` over
   `resource_blobs` only, so **no quota counts those journal bytes**, before or after this decision.
   They are removed by `prune_delivered_changes` once `retention_days` elapses.
3. **`materialize` cannot refuse anything.** It returns `()`, logs the error and continues
   (`sync.rs:393`). Its single caller ignores it.
4. **Nothing re-materializes from the journal.** A projection that is skipped is absent from the
   server permanently; there is no repair path. This is not specific to quota — it is the defect
   [keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75) exists for.
5. **The client is fire-and-forget.** `DbBackend::send_changes` returns `Ok` once the socket accepts
   the frame, and `run_sync` then advances the `last_sync` watermark. A change the server discards is
   **never re-sent**: it falls behind the watermark and `get_changes_since` will not collect it again.
6. **The client discards any frame whose `type` is not `"changes"`**, silently and without a log. A
   new frame type is therefore backward compatible with every deployed client, and equally invisible
   to it.
7. **Notes are not materialized from synchronization.** `Change::NoteCreate`, `NoteUpdate` and
   `NoteDelete` are `Ok(())` no-ops (`sync.rs:388`). Only `max_user_storage_bytes` is reachable
   through this path; `max_notes_per_user` is not.

Canonical [keeplin ADR 0001](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0001-current-sync-delivery.md)
already records the consequence of facts 5 and 6 as **unconfirmed delivery**: no application-level
acknowledgement exists in either direction.

### The boundary this decision must not cross, and nearly did

[keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75) already owns the ground a
materialization-time quota check would stand on. It requires its own prior ADR choosing between:

- **A** — journal and all its projections in one PostgreSQL transaction, or
- **B** — a durable outbox with an idempotent projection worker.

That choice decides *where materialization happens at all*. Under A the journal insert and the blob
write share a transaction, so a quota check has one obvious home; under B the blob write happens
later in a worker, and a refusal there is further from the client still. **A decision that placed the
quota transaction inside `materialize` would be deciding #75's A/B question as a side effect**, which
#75 explicitly reserves for its own decision record.

Similarly, [keeplin#150](https://github.com/jsunyermias/keeplin/issues/150) owns the acknowledgement
protocol, and #75's own sketch already names a stable `NACK` for a permanently invalid change. Any
refusal frame this ADR introduces is a fragment of that design arriving early.

This ADR therefore restricts itself to what is well defined under **both** A and B, and says so
rather than leaving a reader to discover the entanglement.

## Forces and requirements

- The bypass is unbounded and trivially reachable today. Closing it should not wait on the
  resolution of #75, which is blocked on its own ADR and tied to a protocol bump.
- Whatever is decided must remain correct under either outcome of #75. Prejudging that choice is a
  worse outcome than a narrower decision.
- The refusal must not silently claim more than it establishes. ADR 0003's invariant 3 — lock,
  deciding read and write in one transaction — cannot be satisfied on a path whose write placement
  is undecided, and pretending otherwise would repeat the failure that
  [keeplin-srv#138](https://github.com/jsunyermias/keeplin-srv/issues/138) records.
- A refused change must not consume the storage the refusal exists to protect. A refusal that leaves
  the bytes in the journal protects a table, not a disk.
- The refusal must not corrupt convergence further than the protocol already does. In particular it
  must not leave the server missing an object every other device holds.
- The HTTP contract is untouched: `507 QuotaExceeded` stays exactly as it is.
- No client is required to change for this decision to take effect.

## Threat model

**Asset.** Server storage capacity, and the per-user bound `max_user_storage_bytes` asserts.

**Trust boundary.** An authenticated device's WebSocket frames, interacting with PostgreSQL.

**Adversary.** An authenticated user. The capability required is to synchronize rather than to upload
over HTTP — no timing, no privilege, no knowledge of internals. It is repeatable and unbounded.

**Capabilities and consequence.** Today a user's stored blob total is unbounded by the declared limit
through this path. After this decision the `resource_blobs` total is bounded, and the journal
amplification of fact 2 remains uncounted; that residue is in scope for
[keeplin-srv#145](https://github.com/jsunyermias/keeplin-srv/issues/145) and is stated here rather
than left implicit.

**Accepted leakage.** A refusal is observable to the refusing device as a frame it may ignore, and to
other devices as the absence of a resource. Neither reveals another user's state.

**Out of scope.** Per-connection quotas, batch size limits and backpressure, which
[keeplin-srv#77](https://github.com/jsunyermias/keeplin-srv/issues/77) owns; and delivery
guarantees, which keeplin#150 owns.

## Options considered

### Option 1 — Refuse inside `materialize`, silently

Add the check to the `ResourceCreate` arm; on over-limit, skip `put_resource_blob` and log.

Rejected on three grounds, of which only the first is obvious. It runs after the journal commit, so
the bytes are already stored and amplified (fact 2) — the refusal protects a table and not a disk.
It runs after the fan-out is queued, so **every other device receives and stores the resource while
the server does not**, a divergence in the worst available direction. And placing a locked
transaction inside `materialize` decides #75's A/B question by implication.

### Option 2 — Refuse at ingress, before the journal insert, silently

Inspect the batch in `handle_incoming`; drop an over-limit `ResourceCreate` before
`append_changes`, journal the rest.

The bytes never enter the journal and nothing is fanned out, so the server and every other device
agree: the resource does not exist. Nothing about materialization is decided.

Its cost is fact 5: the origin device advanced its watermark and will never re-send, so the resource
lives on that one device forever and nobody is told.

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

Not rejected on merit — it is the most useful behaviour for a real user, and it is the shape #75's
option B already implies. Rejected on ownership: a projection state machine with a repair worker is
precisely what #75 must decide, and building one here would decide it.

### Option 6 — Decide nothing here and fold the sync half into #75's ADR

Implement ADR 0003's HTTP half now, and leave the sync path enforced by whatever #75 lands.

Its merit is that it crosses no boundary at all. Its cost is that the storage limit stays trivially
bypassable until an ADR that has not been written, for an issue inside a critical epic that must ship
in lockstep with a protocol bump. That is an unbounded hole held open by a scheduling dependency.

## Decision and justification

> This ADR is `proposed`. It records a recommendation and does not authorize implementation. Only
> the maintainer may accept or reject it.

**Proposed decision: adopt Option 3 — refuse at ingress, and say so on the wire.**

**Part one — the synchronization path performs admission control before journaling.** In
`handle_incoming`, before `append_changes`, each `Change::ResourceCreate` carrying `data` is measured
against `max_user_storage_bytes` under the same per-user, per-quota advisory lock ADR 0003 defines,
in one transaction: acquire, read the live total, decide. An admitted change proceeds unchanged. A
refused change is removed from the batch and is never journaled, never fanned out and never
materialized.

The consequence worth naming is that on this path **no counted object is created at all**. ADR 0003's
invariants 1 and 2 hold because there is no write to authorize, not because a write was guarded.

**Part two — the refusal is stated on the wire.** The server sends a frame identifying the refused
change and the reason. Its vocabulary is provisional and subordinate to keeplin#150: when the
acknowledgement protocol lands, this frame is reconciled into it rather than kept alongside it. That
is recorded as a debt here rather than discovered later.

**Part three — what this decision explicitly does not establish.** ADR 0003's invariant 3 requires
the lock, the deciding read and the write to be one transaction. On this path the deciding read is at
ingress and the blob write is at materialization, in a different transaction, so **invariant 3 is not
satisfied here and this ADR does not claim it is.** The residue is a cross-path write skew: a sync
batch admitted at ingress can still be materialized after a concurrent `put_resource_data` consumed
the room, exceeding the limit by approximately one round — the same bound and the same class as the
HTTP-only skew ADR 0003 describes, not a new unbounded hole.

Closing that residue requires the blob write to sit in the transaction that decided it, which is
`keeplin-srv#75` option A. **This ADR states the requirement and cedes the choice**: whichever option
#75 accepts must carry the quota decision into the transaction that performs the projection, and #75's
ADR must say how. That obligation is written here so it is inherited rather than rediscovered.

The invariants proposed are:

1. No `Change::ResourceCreate` that would exceed `max_user_storage_bytes` is journaled, fanned out or
   materialized.
2. The measurement is taken under the ADR 0003 lock for that user and that quota, in one transaction
   with the deciding read.
3. A refused change leaves no row in `changes`, no row in `resources` or `resource_blobs`, and no
   frame in any other device's stream.
4. Refusing one change in a batch admits the others; the batch is not discarded.
5. The refusal is reported to the origin device in a frame that a client which does not understand it
   may ignore without error.
6. The HTTP `507 QuotaExceeded` contract is byte-identical to today's.
7. Nothing in this decision places a transaction inside `materialize`, and nothing in it constrains
   the choice keeplin-srv#75 must make beyond the obligation stated above.

### Costs, stated rather than implied

**The user is not told by anything they can see.** Until `keeplin` acts on the frame, a refused
resource stays on the origin device, is absent everywhere else, and surfaces to the user as nothing
at all. This decision makes the refusal *representable*; it does not make it *visible*. Claiming
otherwise would be the same overstatement ADR 0003's first draft made twice.

**A change that is never re-sent is a change that is lost.** Fact 5 means even a client that learns
of the refusal cannot recover the resource by syncing again without the outbox keeplin#151 designs.
Retention of the local copy is the client's, and this ADR does not reach it.

**A refusal frame before the acknowledgement protocol is vocabulary debt.** Small, deliberate, and
recorded.

**Ingress measurement costs a lock and a read per resource-bearing batch**, on a path whose latency
characteristics differ from HTTP.

### Not decided

- The frame's exact name and field set beyond the requirement that it be ignorable. Reconciling it
  with keeplin#150's acknowledgement vocabulary is that epic's work.
- What a client does with the refusal. That is `keeplin` work and belongs with
  [keeplin#151](https://github.com/jsunyermias/keeplin/issues/151), not to a new issue.
- Whether the journal's amplified copy of the blob should count against the quota, or should not be
  stored at all. In scope for keeplin-srv#145.
- Where the blob write finally sits. Ceded to keeplin-srv#75, with the obligation above attached.

## Consequences and risks

- A user currently over `max_user_storage_bytes` through synchronization will have those changes
  refused. That is the intended correction and it is a behaviour change.
- Refused resources are silently absent to every user of a client that has not been updated. This is
  a worse user experience than a `507`, and it is the honest consequence of a protocol with no
  acknowledgement.
- The residual cross-path skew above remains until keeplin-srv#75 lands.
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

Rollback is removing the ingress check, returning the storage limit to bypassable by synchronization.

## Verification plan

| # | Evidence | Kind | What fails if the decision is violated |
|---|---|---|---|
| 1 | A `ResourceCreate` whose bytes would exceed `max_user_storage_bytes` is refused, and afterwards `changes`, `resources` and `resource_blobs` contain no row for it | negative | Fails if the storage limit remains bypassable by synchronizing, which is the defect |
| 2 | The same batch's other changes are journaled and materialized | positive | Fails if one refusal discards unrelated work |
| 3 | A second device connected to the same user receives no frame naming the refused change, on the live fan-out and on backlog delivery after reconnecting | negative, recovery | Fails if the refusal leaves other devices holding an object the server does not, which is option 1's divergence |
| 4 | Moving the ingress check to after `append_changes` makes rows 1 and 3 fail | mutation | Fails if the tests pass with the check in a position that journals and broadcasts before deciding |
| 5 | Two concurrent resource-bearing batches from one user, each within the limit and jointly exceeding it, with a test-controlled rendezvous holding both between the deciding read and the journal insert: exactly one is admitted | concurrency, forced interleaving | Fails if the ingress check is not under the lock. The rendezvous is required: without it the test can pass on scheduling luck |
| 6 | Redirecting the ingress quota read to `Store`'s pool makes a test fail | mutation | Fails if the deciding read can escape the transaction while tests stay green |
| 7 | The refusal frame is emitted to the origin device and identifies the change | positive | Fails if the refusal is unrepresentable on the wire |
| 8 | A client built from `keeplin@1b92f5d`, which knows only `type == "changes"`, completes a sync cycle without error when the frame is present | compatibility | Fails if the frame breaks a deployed client, which would make it a protocol break rather than an addition |
| 9 | A structural assertion that `materialize` contains no transaction and no advisory lock | structural | Fails if this decision drifts into keeplin-srv#75's territory, which is the boundary it claims not to cross |
| 10 | A structural assertion that no `507` and no `QuotaExceeded` symbol appears on the synchronization path | structural | Fails if the HTTP refusal is copied onto a path that is not an HTTP request |
| 11 | An HTTP `507` is byte-identical to today's, before and after this change | compatibility | Fails if the HTTP contract moved |
| 12 | `./scripts/check-docs.sh` passes with every changed source companion synchronized | documentation | Fails if implementation and documentation diverge |
| 13 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` pass | repository | Fails on behavioral regressions, warnings, or formatting drift |

Rows 4 and 6 are mutation evidence and row 5 requires a forced rendezvous, for the reason
keeplin-srv#138 records: during ADR 0002's phase 1 a test named for exactly the property it was meant
to establish passed for a full review round while that property was violated.

Row 3 exists because it is the one observable that separates this decision from option 1, and a test
suite that omitted it would pass identically against the design this ADR rejects.

No migration or format-recovery evidence is required because this decision changes neither surface.

## Equivalent decision in the other repository

None is required now. The frame is additive, `PROTOCOL_VERSION` does not move, and no `keeplin`
change is needed for this decision to take effect — fact 6 is what makes that true.

The moment `keeplin` is required to *act* on the refusal, the protocol stops being server-local and
that becomes a canonical decision in `keeplin`, versioned there. It belongs with
[keeplin#150](https://github.com/jsunyermias/keeplin/issues/150) and
[keeplin#151](https://github.com/jsunyermias/keeplin/issues/151) rather than with this ADR, and this
paragraph exists so that boundary is crossed deliberately rather than by accretion.
