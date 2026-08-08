# 0006 — Making a journaled change reach its projection

- Status: accepted
- Date: 2026-08-08
- Decision owners: maintainer of `jsunyermias/keeplin-srv`
- Scope: keeplin-srv
- Issue: [keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75)
- Acceptance PR: [keeplin-srv#148](https://github.com/jsunyermias/keeplin-srv/pull/148)
- Supersedes: none
- Superseded by: none

## Context and problem

`Verified at`: `keeplin-srv@5b2bb88` and `keeplin@1b92f5d`.

The synchronization relay commits an accepted batch to the journal and then projects it into the
queryable tables. **A projection that fails is logged and forgotten.** `materialize` returns `()`,
logs each error and continues (`sync.rs:393`); its single caller ignores it; nothing re-drives the
journal and no watermark records what was projected. The journal holds the truth, `notebooks`,
`tags`, `note_tags` and `resources` hold less than the truth, and no path reconciles them.

Three decisions now wait on this one, which is why it is worth deciding carefully rather than
quickly:

- [ADR 0004](0004-sync-quota-refusal.md) was **rejected** for deciding the synchronization path's
  quota refusal ahead of this issue. That decision moves here, together with the eleven verified
  facts recorded in it.
- [ADR 0005](0005-serializable-participant-set.md) requires the synchronization path's notebook
  writes to be serializable and retried as one whole unit, and deliberately leaves what happens on
  exhaustion to this decision.
- The partial-batch defect below is a prerequisite for any of it.

### What the code does today, read rather than assumed

1. **`handle_incoming` journals, then projects, then fans out.** `append_changes` commits and returns
   only the sequence numbers it newly inserted; if that list is non-empty the whole submitted slice is
   passed to `materialize` (`sync.rs:272`) and the fan-out frame is built from the same slice
   (`sync.rs:274`). **Neither uses the inserted subset.** A batch that partially duplicates an earlier
   one therefore re-projects and re-broadcasts work already journaled.
2. **`batch_index` is positional.** `append_changes` derives it from the position in the slice it is
   given, and deduplication is keyed `(user_id, batch_id, batch_index)`. Any design that filters,
   reorders or defers a change inside a batch breaks that identity unless it carries the original
   index.
3. **Each projection opens its own transaction, and the count is not *n*.** `upsert_notebook`
   (`store.rs:1775`), `upsert_tag` (`store.rs:1879`) and `upsert_resource_meta` (`store.rs:2036`) each
   call `self.pool.begin()`. A `ResourceCreate` carrying data uses **two** — metadata then blob, in
   separate transactions. A note change uses **none**, because those arms are no-ops (`sync.rs:388`).
   None of them is tied to the journal append.
4. **Projections are idempotent by construction — except the one that matters most.** Seven
   `incoming_wins` last-writer comparisons guard the upserts. **`put_resource_blob` has none**: it is
   `INSERT … ON CONFLICT (resource_id) DO UPDATE SET data = EXCLUDED.data`, unconditional. An earlier
   draft of this ADR said nine guards and called replay safe; nine was a count of every occurrence of
   the symbol including its own definition, and the safety claim was false for exactly the projection
   where two appliers can interleave. Both errors are recorded because the design that followed from
   them was wrong.
5. **Nothing bounds a batch.** `handle_incoming` accepts whatever array arrives; the only limit is the
   WebSocket frame, `max_message_size(64 MB)` (`sync.rs:85`). A single batch can carry many changes and
   tens of megabytes of resource bytes. Bounding it is
   [keeplin-srv#77](https://github.com/jsunyermias/keeplin-srv/issues/77) and is not done.
6. **The client never re-sends.** `DbBackend::send_changes` returns `Ok` once the socket accepts the
   frame, and `run_sync` then advances its `last_sync` watermark. Canonical
   [keeplin ADR 0001](https://github.com/jsunyermias/keeplin/blob/main/docs/adr/0001-current-sync-delivery.md)
   records this as **unconfirmed delivery**: no application-level acknowledgement exists in either
   direction. A durable acknowledgement is [keeplin#150](https://github.com/jsunyermias/keeplin/issues/150)
   and is not done either.
7. **A repeated `batch_id` returns empty.** If every index is a duplicate, `inserted` is empty and
   `handle_incoming` returns before projecting (`sync.rs:266`), so a retry of a batch whose projection
   failed re-projects nothing.

8. **Multiple replicas are a documented deployment.** `README.md` describes running several
   instances behind a load balancer against one database, and every process starts its own
   maintenance loop (`main.rs:54`). `instance_id` (`state.rs:133`) is a notification-origin
   identifier and provides no exclusion. So anything periodic runs concurrently by design, not by
   accident.

Facts 5 and 6 are the ones that decide this ADR. Fact 7 is what makes today's defect unrepairable by
retry and is why invariant 5 exists; it does no work against the alternatives, and an earlier draft
cited it where it does not apply.

## Forces and requirements

- No path may leave "journal committed, projection forgotten". That is the defect.
- A transient failure must be retried without operator action; a permanent one must be visible rather
  than silent.
- Idempotency must not be weakened: replay is the recovery mechanism.
- Soft-delete semantics, version-vector resolution and the deterministic `(timestamp, device_id)`
  tiebreak are unchanged.
- **Durability must not go backwards.** Today an accepted batch is durable the moment the journal
  commits. Any option that makes acceptance conditional on projection success must answer what
  happens to a batch the client will never re-send.
- The decision must express ADR 0005's requirement: a guarded notebook write is serializable and
  retried as one whole unit.
- It must leave room for the quota enforcement ADR 0004 was rejected for deferring here.
- Existing batches already lost in deployment need a repair path, independent of which option is
  chosen.

## Threat model

Not primarily an adversary problem, and saying so is not the same as omitting the section.

**Asset.** The equivalence between the journal and the projections, on which REST, snapshots and every
newly joining device depend.

**Failure modes.** Crash between journal commit and projection; a transient database error; a
serialization failure once ADR 0005 lands; a permanently invalid payload; and process restart.

**Adversary-reachable amplification.** Fact 5 means an authenticated user chooses the batch size, so
whatever the transaction shape is, the user chooses how big it gets. Today the handler already runs
*n* unbounded transactions synchronously inside the request, so the amplification exists now; an
option that puts the whole batch in one transaction concentrates it into a single user-controlled long
transaction, and a deferred design moves it off the request path. None of them removes it —
keeplin-srv#77 is what would bound it, and it is open.

**Out of scope.** Delivery to clients, which is keeplin#150; collaboration line editing.

## Options considered

### Option 1 — Keep the current behaviour

Rejected. It is the defect, and ADR 0005 makes it worse by making the failure reachable through
contention rather than only through error.

### Option 2 — Journal and every projection in one transaction

The issue's own recommendation: "A para el volumen actual, salvo evidencia demostrable de que hace
falta una cola."

It is the simplest correct-looking answer, and **facts 5, 6 and 7 are that demonstrable evidence
against it**:

- A failure anywhere in the batch rolls the journal append back with it. The client has already
  advanced its watermark (fact 6) and will never re-send, so **the entire batch is lost** — including
  every change that would have projected fine. Today those changes survive in the journal. That is a
  durability regression, and it is not recoverable by any mechanism that exists.
- The transaction's size is chosen by the client (fact 5), and under ADR 0005 it is serializable, so
  its abort probability rises with exactly the thing the user controls.

An earlier draft added a third bullet: that a retry of the same `batch_id` could not repair it either,
by fact 7. **That was wrong.** Under option 2 the failed batch rolls back completely, so no journal
rows exist, so a retry is not a duplicate and fact 7's early return never fires. Fact 7 does real work
against the *current* tree and behind invariant 5; it does none against option 2. The case against
option 2 rests on fact 6 alone, and since this ADR's divergence from the issue's own recommendation is
justified by naming facts, a fact cited for a step it does not support has to go.

Option 2 becomes defensible **after** keeplin#150's acknowledgement and keeplin-srv#77's batch bound
exist. Both are open. Choosing it now trades a silent projection loss for a silent batch loss.

### Option 3 — Durable projection queue, applied by an idempotent worker

The journal append and one projection job per inserted change commit in a single transaction. A worker
claims jobs, applies them with the existing idempotent upserts, retries transient failures with
backoff, and moves permanently failing jobs to a dead-letter state that is visible in metrics.

Journal durability is unchanged: an accepted batch is still durable at journal commit. Fact 4 makes
the worker safe to retry. Fact 1's slice defect disappears, because jobs are created from the inserted
subset rather than from the submitted slice.

Costs, stated rather than minimised: a migration, a job lifecycle with claim/lease semantics, a worker
to operate and monitor, and end-to-end latency that is no longer synchronous with the request. It is
strictly more machinery than option 2.

### Option 5 — A durable per-batch projection marker, swept

Commit the journal as today, and in the same transaction record a marker saying this batch's
projections are outstanding. Apply the projections immediately after, as today. On success clear the
marker; on failure leave it, with an attempts counter and the indices that failed. A bounded sweep
re-drives marked batches only.

Journal durability is unchanged. Replay is safe by fact 4. Fact 1's slice defect is fixed by the
marker recording the inserted subset. The sweep never scans the journal, so the retention objection
against option 4 does not apply to it. A permanently failing change is bounded by the attempts counter
rather than retried forever.

**This looked like the smallest design that satisfies every force**, and the first draft of this ADR
omitted it entirely, which is why round 1 was right to block. It was then adopted, and round 2
refuted it against the tree. It is kept here in full because a rejected option described only in
summary cannot be re-evaluated, and because the reasons it fails are the reasons the queue's features
are not optional.

**Why it is not adopted.** Without lease semantics a sweep and the synchronous path apply the same
batch concurrently, and fact 4 does **not** make that safe: `put_resource_blob` is unguarded, so the
interleaving in the decision section produces new metadata with old bytes. By fact 8 concurrent
appliers are a documented deployment. And one state per batch cannot express a batch that is
permanently invalid at one index and transiently failing at another; giving indices their own state
and counters is the per-change lifecycle this option existed to avoid.

### Option 4 — Reconciliation sweep with no queue

A periodic job re-derives projections from the journal and repairs drift, with no per-change state.

Not adopted, and it is closer than it looks: it needs no migration to the write path and it repairs
batches already lost. Its costs are that drift is repaired on a timer rather than promptly, that the
sweep must scan a journal whose retention is unbounded in practice — `prune_delivered_changes` also
requires `seq <= COALESCE(MIN(device_cursors.last_seq), 0)`, so one stalled device pins everything —
and that a permanently invalid payload is retried forever with nowhere to put it.

**It is adopted as a component**, not as the answer: the repair path for already-lost batches is
exactly this sweep, and it is required by whichever option wins.

## Decision and justification

**Adopt Option 3 — the durable projection queue — with the retention interlock below.**

This document has now proposed the queue, been argued down to the marker, and come back. That is not
indecision; it is the bar issue keeplin-srv#75 sets being met. The issue says a queue only if a queue
is **demonstrably** needed. Round 1 was right that the first draft had not demonstrated it, because
the marker was never evaluated. Round 2 then evaluated the marker against the tree and produced the
demonstration, in three independent ways:

- **Resource projection is not idempotent, so concurrent appliers are not merely wasteful.** By fact
  4, `put_resource_blob` is unguarded. Two appliers of the same batch interleave like this: applier 1
  writes old metadata; applier 2 writes new metadata and new bytes; applier 1 resumes and overwrites
  the bytes with the old ones; applier 1 then rejects the new metadata because it is already present.
  The result is **new metadata with old bytes** — a state neither complete application produces. By
  fact 8 concurrent appliers are a documented deployment. A marker without a lease is therefore
  unsound, and a lease is the queue's principal feature.
- **One per-batch state cannot express a mixed batch.** With index 0 permanently invalid and index 1
  failing transiently, marking the batch failed stops retrying index 1, and leaving it outstanding
  retries index 0 forever and never reaches the distinct failed state invariant 6 promises. Giving
  indices their own state and attempt counters resolves it — and that is per-change lifecycle, the
  one capability the marker was chosen to avoid.
- **Nothing else changes the arithmetic.** Once a lease and per-change state are required, what
  remains of the marker's advantage is that its rows live in one table instead of two.

So the decision is the queue, and the reason is on the page rather than assumed.

**Part one — the job is created in the journal transaction.** One job per newly inserted change,
committed with `append_changes` itself. **It must be inserted inside that function before its existing
commit** (`store.rs:625`); a separate call after `append_changes` returns would not survive the crash
window this part exists to close. Jobs carry `(user_id, batch_id, batch_index)`, which fact 2 requires
and which fact 1's slice mismatch currently loses.

**Part two — `handle_incoming` stops working from the submitted slice.** Jobs and the fan-out frame
both derive from what `append_changes` actually inserted. A correctness fix independent of everything
else here, and a prerequisite.

**Part three — claim, lease and completion are specified, not assumed.** A worker claims jobs
atomically (`FOR UPDATE SKIP LOCKED` or equivalent) and holds a lease. **A lease that expires while a
slow worker is still applying must not permit a second applier**, because fact 4 makes double
application unsafe for resources; the design must either make the lease renewable or make the
resource metadata-and-blob projection atomic in one transaction. Whichever is chosen, it is chosen
explicitly and verified, rather than resting on an idempotency claim that is false.

**Part four — retention must not erase a job's input.** `prune_delivered_changes` today deletes on age
and device cursors alone (`store.rs:744`), and fan-out still happens after a projection failure, so
every cursor can advance past a change whose job is outstanding and retention can then delete the
payload the job needs. **Pruning must exclude every journal row referenced by an unfinished job**, or
the job must carry its own replay input. Without this, invariant 1 is unachievable and part six's
command cannot repair what it is for.

**Part five — a change is projected, or it is visibly failed.** After a bounded number of attempts a
job moves to a dead-letter state counted in metrics. The classification: a payload that cannot
deserialize is permanent; a serialization failure under ADR 0005 is retried under that ADR's bound and
**does not consume the attempts counter**, because contention must not fail a valid write as though it
were invalid. Constraint violations are **not** classified wholesale — `AppError` keeps every SQLx
failure in one variant (`error.rs:17`) and an ordering-dependent violation can be transient, so the
implementation must enumerate the SQLSTATEs it treats as permanent rather than inheriting a category.
A dead-lettered job is re-drivable by part six.

**Part six — a reconciliation command.** An operator command re-drives outstanding and dead-lettered
jobs and re-derives projections from the journal for a user or a range, for batches lost before any of
this existed. **It does not re-broadcast**: fan-out already happened, and re-sending would depend on a
client-side duplicate tolerance this ADR has not established. Peers learn repaired state on their next
sync.

**Part seven — what this ADR does not decide.** ADR 0005's exhaustion sentence is answered: a guarded
notebook write that exhausts its retries leaves its job claimable rather than dropping the write.
Everything else stays where it belongs:

- **Quota.** keeplin-srv#145 owns it. An earlier draft claimed a check inside the journal transaction
  at ingress would satisfy ADR 0003's invariant 3. **It would not** — the resource write happens later
  in a different transaction, and invariant 3 requires the lock, the deciding read and the write to be
  one. Only a check in the transaction that performs the projection can satisfy it. That is a fact
  about ADR 0003, not a preference of this one, and #145 remains the owner of what to do about it.
- **What the server reports about a duplicate batch.** Invariant 5 requires only that outstanding work
  complete and that no silent empty hide it; the shape of any report is keeplin#150's.

The invariants are:

1. A change that is journaled is projected, or carries a job that is outstanding or dead-lettered.
   There is no third outcome.
2. Journal durability is not conditional on projection success.
3. Jobs and fan-out derive from the inserted subset, never from the submitted slice.
4. No two appliers apply the same job concurrently, and this is established by the claim mechanism
   rather than by an idempotency assumption.
5. A repeated `batch_id` completes outstanding work and never returns a silent empty while work is
   outstanding. What it reports is keeplin#150's to define.
6. A permanently invalid payload and a transient failure reach different, observable states, by an
   enumerated classification.
7. A journal row referenced by an unfinished job is not pruned.
8. Guarded notebook writes are serializable and retried as one whole unit, per ADR 0005; exhausting
   that bound leaves the job claimable rather than dropping the write.

### The cost this decision does not remove, and must not be read as removing

Until keeplin-srv#77 bounds a batch and keeplin-srv#145 decides quota on this path,
**`max_user_storage_bytes` does not bound what synchronizing stores.** A 64 MB frame is journaled
before any quota check exists on that path, and a job row **references** the journal change rather
than copying it — payloads are never duplicated into job rows, which is stated because an unstated
policy makes the amplification uncomputable, and because part four's retention interlock only makes
sense for a job that points at something. This ADR does not make that worse and
does not fix it.

### Costs, stated rather than implied

**This is the largest amount of new machinery any decision in this repository has proposed.** A
migration, a job table, claim and lease semantics, a dead-letter state, metrics, a retention interlock
and a reconciliation command. The single transaction is a fraction of the work and the marker is less
than half; neither is adopted, and the demonstration of why is in the decision section rather than
asserted here.

**Projection becomes asynchronous on the failure path, and may become so on the common path.** If jobs
are applied by a worker rather than inline, a device can write and immediately read its own change
missing. Row 10 fixes the acceptable delay before it is measured rather than after.

**The dead-letter state is an operational obligation.** A queue whose failures nobody watches is the
current defect with more steps, and this ADR says that plainly rather than trusting a metric to be
looked at.

**The retention interlock ties two subsystems together.** Pruning must now consult job state. That is
a coupling the current design does not have, and it is the price of a job that references its input
instead of copying it.

### Not decided

- The quota representation on this path, which remains keeplin-srv#145's with ADR 0004's analysis.
- Client acknowledgement, which is keeplin#150. This decision makes an ACK *possible* to define
  honestly, because there will finally be a durable answer to acknowledge; it does not define one.
- Batch size bounds, which are keeplin-srv#77.
- Whether the worker is in-process or a separate binary.

## Consequences and risks

- The journal-to-projection loss is closed, including the loss ADR 0005 makes contention-reachable.
- Projection latency becomes visible and must be monitored rather than assumed to be zero.
- A new failure surface exists: the worker itself. It is observable, which the current failure is not.
- Already-lost batches are repairable for the first time.
- **A device joining while a job is outstanding receives a snapshot missing journaled changes, with no
  staleness signal.** True today and still true; the window becomes bounded by the retry schedule
  instead of unbounded, which is an improvement and not a fix.
- **Fan-out precedes durable projection.** Peer devices can hold a change the server's own projections
  lack until its job completes. Accepted deliberately: fan-out is what gets the change to peers at
  all, and delaying it behind projection would trade a projection gap for a delivery gap.

## Compatibility, migration, and rollback

A forward-only migration adds the job table, and pruning gains the interlock in part four. No wire or
format change; `PROTOCOL_VERSION` does not move and `keeplin-core` is untouched.

The migration must allow reconciling existing batches. Six of the seven projection kinds are
last-writer guarded and safe to replay; **`put_resource_blob` is not** (fact 4), so reconciliation of
a resource must apply metadata and bytes under the same exclusion the steady state uses rather than
relying on replay being harmless. An earlier draft of this section said fact 4 makes the projections
idempotent, which is what fact 4 used to claim and no longer does. Notices are not sent from this
path, so there are none to repeat.

Rollback removes the worker and returns to synchronous projection, restoring the defect, and must
restore the previous pruning predicate with it. It must not be silent, and any job left in the table
at rollback must be reconciled by part six's command first — after rollback nothing will.

## Verification plan

| # | Evidence | Kind | What fails if the decision is violated |
|---|---|---|---|
| 1 | Failure injected during each entity's projection leaves the job claimable and the change projected after retry | failure injection | **Fails on the current tree**, where the error is logged and dropped |
| 2 | A batch that partially duplicates an earlier one projects and fans out only the newly inserted changes | negative | Fails if either still derives from the submitted slice, which is today's behaviour |
| 3 | A repeated `batch_id` whose projection was outstanding completes that work rather than returning early | negative, recovery | Fails if fact 7's silent empty hides outstanding work. The row deliberately asserts nothing about what is reported to the client, which is keeplin#150's |
| 4 | An invalid payload reaches the dead-letter state and is counted, while a transient error is retried; the two are distinguishable in metrics | negative | Fails if both remain a `continue` or a `warn!` |
| 5 | Journal durability is unaffected by projection failure: the change is present in `changes` after every failure mode in rows 1 and 4 | recovery | Fails if the design made acceptance conditional on projection, which is option 2's regression |
| 6 | A crash between journal commit and the first claim leaves the job claimable after restart, with no operator action | recovery, restart | Fails if the job is not inserted inside `append_changes` before its commit, which is the only placement that survives this crash |
| 7 | Two appliers racing one `ResourceCreate` carrying data — metadata written by one interleaved with the blob written by the other — cannot produce new metadata with old bytes | concurrency | **Fails on the current tree.** This is the schedule that refuted the marker design, and a test using the client path that strips blobs would pass while the raw change stays unsafe |
| 8 | A guarded notebook write runs `SERIALIZABLE`, retries `40001` within ADR 0005's bound, and on exhaustion leaves the job claimable rather than dropping the write; the exhaustion does not consume the job's attempts counter | failure injection | Fails if ADR 0005's exhaustion is answered by silence, or if contention can drive a valid write into the failed state |
| 9 | The reconciliation command rebuilds projections for a user from the journal on a database seeded with a batch lost before any of this existed, reaching invariant 1's end state — every change projected or its batch marked failed — and broadcasting nothing | recovery, operational | Fails if already-lost data stays lost, if an undeserializable payload makes the command loop rather than report, or if repair re-broadcasts to peers |
| 10 | The worst-case delay between journal commit and projected state is fixed by the maintainer **before** the measurement is taken, and the measurement is compared against it | operational, budgeted | Fails if the delay exceeds what was agreed. A row that only requires a number to be recorded is satisfied by measuring and then choosing a budget to fit, which is no constraint at all |
| 11 | Metrics expose outstanding jobs, failed jobs, dead-lettered jobs and the oldest outstanding age | operational | Fails if queue health is not observable, which would reproduce the current defect with more machinery |
| 11b | A batch with one permanently invalid index and one transiently failing index reaches the dead-letter state for the first and completes the second | negative, mixed | Fails if one state per batch is used, which cannot express this and which is why the marker design was rejected |
| 11c | Retention does not delete a journal row referenced by an unfinished job, proven with every device cursor advanced past it | recovery | Fails if pruning considers only age and cursors, which is today's predicate and which would erase a job's own input |
| 12 | `./scripts/check-docs.sh` passes with every changed source companion synchronized | documentation | Fails if implementation and documentation diverge |
| 13 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` pass | repository | Fails on behavioral regressions, warnings, or formatting drift |

Row 1 is expected to fail before implementation. Rows 5 and 8 exist because they pin the two things
this decision claims over its alternatives: that durability does not regress, and that ADR 0005's
deferred sentence is answered.

## Equivalent decision in the other repository

None required. The projection tables, the worker and the journal are local to this repository, and
nothing here changes a shared wire or format surface. `keeplin`'s side of the durability story is
keeplin#150 and keeplin#151, which this decision makes definable but does not define.
