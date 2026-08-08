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

Facts 5, 6 and 7 are the ones that decide this ADR, and they are why the recommendation below differs
from the one the issue itself carries.

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
whatever the transaction shape is, the user chooses how big it gets. Under an option that puts a whole
batch in one transaction, that is a user-controlled long transaction — which is a denial-of-service
surface that does not exist today, and which keeplin-srv#77 would bound.

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
- A retry of the same `batch_id` cannot repair it either: by fact 7 a fully duplicate batch returns
  before projecting.
- The transaction's size is chosen by the client (fact 5), and under ADR 0005 it is serializable, so
  its abort probability rises with exactly the thing the user controls.

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

**Proposed decision: adopt Option 3, plus Option 4 as a bounded repair tool.**

**Part one — the projection job is created in the journal transaction.** `append_changes` and one job
per newly inserted change commit together. Jobs carry the change's `(user_id, batch_id, batch_index)`
identity, which fact 2 requires and which fact 1's slice mismatch currently loses.

**Part two — `handle_incoming` stops working from the submitted slice.** Projection jobs and the
fan-out frame are both derived from what `append_changes` actually inserted. This is a correctness fix
independent of the queue and it is a prerequisite for it.

**Part three — the worker is idempotent, retried and bounded.** It claims jobs with a lease, applies
them through the existing upserts, retries transient failures with backoff, and after a bounded number
of attempts moves a job to a dead-letter state that is counted in metrics rather than logged and
dropped. **A permanently invalid payload and a transient database error must reach different states**;
today `serde_json::from_value` failure and a store error are both a `continue` or a `warn!`.

**Part four — ADR 0005 is satisfied inside the worker.** A guarded notebook write executes in a
serializable transaction retried as one whole unit. Exhaustion no longer means silent
non-application: the job stays claimable and is retried, and only the dead-letter bound ends that.
**This is the sentence ADR 0005 deferred, and it is now answered rather than left open.**

**Part five — quota enforcement has a home.** The rejected ADR 0004 established that the
synchronization path must enforce `max_user_storage_bytes` and could not do so soundly at ingress
without deciding this issue. Under a durable queue the deciding read and the blob write are in the
worker's transaction, which is what ADR 0003's invariant 3 asks for, and ADR 0004's option 5 —
admit, defer, apply when the user frees space — becomes expressible. **This ADR does not decide the
quota's representation**; it records that this is where that decision now becomes possible, and
keeplin-srv#145 remains its owner.

**Part six — a reconciliation command.** A documented operator command re-derives projections from the
journal for a user or a range, for batches already lost before any of this existed. Option 4's costs
are acceptable for a tool that is run deliberately and not for the steady state.

The invariants proposed are:

1. A change that is journaled is projected or is visibly dead-lettered. There is no third outcome.
2. Journal durability is not conditional on projection success.
3. Projection jobs and fan-out derive from the inserted subset, never from the submitted slice.
4. Replaying any job is safe; idempotency is a requirement of the worker, not an accident of the
   upserts.
5. A repeated `batch_id` completes outstanding work or reports prior state; it never returns a silent
   empty.
6. A permanently invalid payload and a transient failure reach different, observable states.
7. Guarded notebook writes are serializable and retried as one whole unit, per ADR 0005.

### Costs, stated rather than implied

**This is the largest amount of new machinery any decision in this repository has proposed.** A
migration, a job table, lease semantics, a worker, a dead-letter state, metrics and a reconciliation
command. Option 2 is a fraction of the work, and the only reason it is not adopted is facts 5, 6 and 7.

**Projection becomes asynchronous.** A device that writes and immediately reads through REST can
observe its own change missing. Today the window is the duration of `materialize`; afterwards it is
the worker's scheduling delay. The verification plan requires that window to be measured, and it is a
user-visible behaviour change rather than an internal one.

**The dead-letter state is an operational obligation.** A queue whose failures nobody watches is the
current defect with more steps.

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
| 1 | Failure injected during each entity's projection leaves the job claimable and the change projected after retry | failure injection | **Fails on the current tree**, where the error is logged and dropped |
| 2 | A batch that partially duplicates an earlier one projects and fans out only the newly inserted changes | negative | Fails if either still derives from the submitted slice, which is today's behaviour |
| 3 | A repeated `batch_id` whose projection was outstanding completes it and reports prior state rather than an empty result | negative, recovery | Fails if fact 7's silent empty survives |
| 4 | An invalid payload reaches the dead-letter state and is counted, while a transient error is retried; the two are distinguishable in metrics | negative | Fails if both remain a `continue` or a `warn!` |
| 5 | Journal durability is unaffected by projection failure: the change is present in `changes` after every failure mode in rows 1 and 4 | recovery | Fails if the design made acceptance conditional on projection, which is option 2's regression |
| 6 | A crash between journal commit and worker claim leaves the job claimable after restart, with no operator action | recovery, restart | Fails if the job is lost with the process |
| 7 | Applying the same job twice produces the same state | idempotency | Fails if the worker relies on being run once |
| 8 | A guarded notebook write in the worker runs `SERIALIZABLE`, retries `40001` within ADR 0005's bound, and on exhaustion leaves the job claimable rather than dropping the write | failure injection | Fails if ADR 0005's exhaustion is answered by silence, which is the sentence this ADR exists to answer |
| 9 | The reconciliation command rebuilds projections for a user from the journal and converges, on a database seeded with a batch lost before the queue existed | recovery, operational | Fails if already-lost data stays lost |
| 10 | Projection latency from journal commit to projected state is measured and reported against an agreed budget | operational, budgeted | Fails if the asynchrony this introduces is assumed to be negligible rather than observed |
| 11 | Metrics expose pending jobs, failed jobs, dead-lettered jobs and oldest pending age | operational | Fails if the queue's health is not observable, which would reproduce the current defect with more machinery |
| 12 | `./scripts/check-docs.sh` passes with every changed source companion synchronized | documentation | Fails if implementation and documentation diverge |
| 13 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` pass | repository | Fails on behavioral regressions, warnings, or formatting drift |

Row 1 is expected to fail before implementation. Rows 5 and 8 exist because they pin the two things
this decision claims over its alternatives: that durability does not regress, and that ADR 0005's
deferred sentence is answered.

## Equivalent decision in the other repository

None required. The projection tables, the worker and the journal are local to this repository, and
nothing here changes a shared wire or format surface. `keeplin`'s side of the durability story is
keeplin#150 and keeplin#151, which this decision makes definable but does not define.
