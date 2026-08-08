# 0006 — Making a journaled change reach its projection

- Status: proposed
- Date: 2026-08-08
- Decision owners: maintainer of `jsunyermias/keeplin-srv`
- Scope: keeplin-srv
- Issue: [keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75)
- Acceptance PR: link once the ADR is accepted
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
3. **Each projection opens its own transaction.** `upsert_notebook` (`store.rs:1775`), `upsert_tag`
   (`store.rs:1879`) and `upsert_resource_meta` (`store.rs:2036`) each call `self.pool.begin()`. So a
   batch of *n* changes commits in *n* independent transactions today, none of them tied to the
   journal append.
4. **Projections are idempotent by construction.** Nine `incoming_wins` last-writer comparisons guard
   the upserts, so replaying a change is safe. This is the property every option below depends on.
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

**This is the smallest design that satisfies every force above**, and an earlier draft of this ADR
omitted it entirely — comparing only the defect, the single transaction, a full queue and a full
sweep, and then concluding a queue was needed. That conclusion did not follow, because the option that
sits between them was never on the page.

What it does not give, stated so the comparison is real: no lease semantics, so a sweep and the
synchronous path can apply the same batch concurrently — safe by fact 4, but it means duplicated work
rather than coordinated work; no per-change job identity, so observability is per batch; and no place
to hold a change admitted but deliberately deferred, which is what a quota decision would want if
keeplin-srv#145 chose deferral. That last one is #145's to decide and is not a reason to build the
machinery now.

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

> This ADR is `proposed`. It records a recommendation and does not authorize implementation. Only
> the maintainer may accept or reject it.

**Proposed decision: adopt Option 5, plus Option 4 as a bounded repair tool.**

An earlier draft proposed the full queue. Independent review established that the option between the
single transaction and the queue had never been evaluated, and that issue keeplin-srv#75's bar — a
queue only if a queue is *demonstrably* needed — is therefore not met by an argument that never put
the smaller design on the page. Nothing in the forces above requires per-change job identity, lease
semantics or a worker process. What they require is that projection state be **durable**, and a marker
is durable.

**Part one — the marker is written in the journal transaction.** `append_changes` and the marker for
that batch commit together, so no crash can leave a journaled batch with no record that its
projections are outstanding. The marker carries the batch identity and the inserted indices, which
fact 2 requires and which fact 1's slice mismatch currently loses.

**Part two — `handle_incoming` stops working from the submitted slice.** Projections and the fan-out
frame both derive from what `append_changes` actually inserted. This is a correctness fix independent
of everything else here, and a prerequisite for the marker to mean anything.

**Part three — projection stays synchronous, and the sweep is the safety net.** The batch is projected
immediately after journaling, as today. On success the marker clears. On failure it stays, with an
attempts counter and the indices that failed, and a bounded sweep in the existing maintenance loop
re-drives marked batches. **This is why the asynchrony cost of a queue does not apply**: the common
path is unchanged and only failures become deferred.

**Part four — a change is projected, or it is marked and visible.** After a bounded number of attempts
a marker moves to a failed state that is counted in metrics rather than logged and dropped. Two things
that an earlier draft asserted without defining, and that this decision must define because invariant
6 rests on them:

- **The classification.** A payload that cannot deserialize is permanent. A store error is transient
  unless it is a constraint violation, which is permanent. A serialization failure under ADR 0005's
  protocol is **retried under that ADR's own bound and does not consume the attempts counter** — a
  contended write must not be failed as though it were invalid.
- **The exit.** A failed marker is re-drivable by part five's command, which is what keeps the failed
  state from being the current silence with a metric attached. A payload that can never deserialize
  will fail identically forever and correctly, and the command reports it rather than looping on it.

**Part five — a reconciliation command.** An operator command re-drives marked and failed batches, and
re-derives projections from the journal for a user or a range, for batches lost before any of this
existed. **It does not re-broadcast.** Fan-out already happened when the batch was accepted, and
re-sending repaired changes would depend on a client-side duplicate tolerance this ADR has not
established; peers learn repaired state on their next sync, and that is stated rather than assumed.

**Part six — what this ADR does not decide, having twice been drafted as though it did.** ADR 0005's
exhaustion sentence is answered — a guarded notebook write that exhausts its retries leaves the marker
outstanding, so the write is re-driven rather than dropped. Everything else stays where it belongs:

- **Quota.** keeplin-srv#145 owns it. This decision **forecloses no option of ADR 0004**: a check
  inside the journal transaction at ingress satisfies ADR 0003's invariant 3 just as a check in the
  re-drive path would. An earlier draft asserted the deferred form was the one that becomes possible,
  which pre-selected #145's answer.
- **What the server tells a client about a duplicate batch.** Invariant 5 below says only that
  outstanding work completes and that no silent empty return hides it. The *shape* of any report is
  acknowledgement vocabulary and belongs to keeplin#150.

The invariants proposed are:

1. A change that is journaled is projected, or its batch carries a marker that is outstanding or
   failed. There is no third outcome.
2. Journal durability is not conditional on projection success.
3. Projections and fan-out derive from the inserted subset, never from the submitted slice.
4. Re-driving a marker is safe; idempotency is a requirement of the sweep, not an accident of the
   upserts.
5. A repeated `batch_id` completes outstanding work and never returns a silent empty while work is
   outstanding. What it reports is keeplin#150's to define.
6. A permanently invalid payload and a transient failure reach different, observable states, by the
   classification in part four.
7. Guarded notebook writes are serializable and retried as one whole unit, per ADR 0005; exhausting
   that bound leaves the marker outstanding rather than dropping the write.

### The cost this decision does not remove, and must not be read as removing

Until keeplin-srv#77 bounds a batch and keeplin-srv#145 decides quota on this path,
**`max_user_storage_bytes` does not bound what synchronizing stores.** A 64 MB frame is journaled
before any quota check exists on that path, and the marker adds a row referencing that batch rather
than copying it — payloads are referenced, never duplicated into marker rows, which is stated here
because an unstated policy makes the amplification uncomputable. This ADR does not make that worse and
does not fix it.

### Costs, stated rather than implied

**A migration and a marker table.** Smaller than the queue an earlier draft proposed, and still not
free: a row per batch, a sweep to operate, and a failed state somebody has to watch. A marker whose
failures nobody watches is the current defect with a table attached.

**Duplicated work rather than coordinated work.** There is no lease. A sweep and the synchronous path
can apply the same batch at the same time. Fact 4 makes that safe and it is why no lease is proposed,
but it is wasted work under load and it is a deliberate trade against the machinery a lease needs.

**Observability is per batch, not per change.** A metric says a batch is failing, not which change.
The failed indices are recorded on the marker, so the information exists; nothing indexes it.

**Failed projections become deferred, and only failed ones.** The common path stays synchronous, so
read-your-writes through REST is unchanged when nothing fails. When something does fail, a device can
read its own change and not see it until the sweep runs — bounded by the sweep interval and stated
rather than assumed away.

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
- **A device joining during a failed-and-not-yet-swept window receives a snapshot missing journaled
  changes, with no staleness signal.** That is true today and stays true; the window becomes bounded
  by the sweep instead of unbounded, which is an improvement and not a fix.
- **Fan-out precedes durable projection.** Peer devices can hold a change the server's own projections
  lack until the sweep repairs it. Accepted deliberately: fan-out is the mechanism that gets the change
  to peers at all, and delaying it behind projection would trade a projection gap for a delivery gap.

## Compatibility, migration, and rollback

A forward-only migration adds the job table. No wire or format change; `PROTOCOL_VERSION` does not
move and `keeplin-core` is untouched. The migration must allow reconciling existing batches without
re-executing non-idempotent effects — fact 4 says the projections are idempotent, and notices are not
sent from this path, so there are none to repeat.

Rollback removes the worker and returns to synchronous projection, restoring the defect. It must not
be silent, and any job left in the table at rollback must be reconciled by part six's command first.

## Verification plan

| # | Evidence | Kind | What fails if the decision is violated |
|---|---|---|---|
| 1 | Failure injected during each entity's projection leaves the batch marker outstanding and the change projected after the sweep runs | failure injection | **Fails on the current tree**, where the error is logged and dropped |
| 2 | A batch that partially duplicates an earlier one projects and fans out only the newly inserted changes | negative | Fails if either still derives from the submitted slice, which is today's behaviour |
| 3 | A repeated `batch_id` whose projection was outstanding completes that work rather than returning early | negative, recovery | Fails if fact 7's silent empty hides outstanding work. The row deliberately asserts nothing about what is reported to the client, which is keeplin#150's |
| 4 | An invalid payload reaches the dead-letter state and is counted, while a transient error is retried; the two are distinguishable in metrics | negative | Fails if both remain a `continue` or a `warn!` |
| 5 | Journal durability is unaffected by projection failure: the change is present in `changes` after every failure mode in rows 1 and 4 | recovery | Fails if the design made acceptance conditional on projection, which is option 2's regression |
| 6 | A crash between journal commit and projection leaves the marker outstanding after restart, and the sweep completes it with no operator action | recovery, restart | Fails if the marker is not written in the journal transaction, which is the only placement that survives this crash |
| 7 | The sweep and the synchronous path applying the same batch concurrently produce the same state as either alone | idempotency, concurrency | Fails if the absence of a lease is unsafe rather than merely wasteful, which is the trade this decision makes |
| 8 | A guarded notebook write runs `SERIALIZABLE`, retries `40001` within ADR 0005's bound, and on exhaustion leaves the marker outstanding rather than dropping the write; the exhaustion does not consume the marker's attempts counter | failure injection | Fails if ADR 0005's exhaustion is answered by silence, or if contention can drive a valid write into the failed state |
| 9 | The reconciliation command rebuilds projections for a user from the journal on a database seeded with a batch lost before any of this existed, reaching invariant 1's end state — every change projected or its batch marked failed — and broadcasting nothing | recovery, operational | Fails if already-lost data stays lost, if an undeserializable payload makes the command loop rather than report, or if repair re-broadcasts to peers |
| 10 | The sweep interval and the resulting worst-case delay between a failed projection and its repair are fixed by the maintainer **before** the measurement is taken, and the measurement is compared against them | operational, budgeted | Fails if the delay exceeds what was agreed. A row that only requires a number to be recorded is satisfied by measuring and then choosing a budget to fit, which is no constraint at all |
| 11 | Metrics expose outstanding markers, failed markers and the oldest outstanding age | operational | Fails if marker health is not observable, which would reproduce the current defect with a table attached |
| 12 | `./scripts/check-docs.sh` passes with every changed source companion synchronized | documentation | Fails if implementation and documentation diverge |
| 13 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` pass | repository | Fails on behavioral regressions, warnings, or formatting drift |

Row 1 is expected to fail before implementation. Rows 5 and 8 exist because they pin the two things
this decision claims over its alternatives: that durability does not regress, and that ADR 0005's
deferred sentence is answered.

## Equivalent decision in the other repository

None required. The projection tables, the worker and the journal are local to this repository, and
nothing here changes a shared wire or format surface. `keeplin`'s side of the durability story is
keeplin#150 and keeplin#151, which this decision makes definable but does not define.
