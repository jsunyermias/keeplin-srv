# 0003 — Per-user quota enforcement under concurrency

- Status: proposed
- Date: 2026-08-08
- Decision owners: maintainer of `jsunyermias/keeplin-srv`
- Scope: keeplin-srv
- Issue: [keeplin-srv#142](https://github.com/jsunyermias/keeplin-srv/issues/142)
- Acceptance PR: link once the ADR is accepted
- Supersedes: none
- Superseded by: none

## Context and problem

`Verified at`: `keeplin-srv@b5b82b1`.

The server enforces exactly two per-user quotas, and both are a check-then-act over an aggregate:

| Handler | Limit | Read |
|---|---|---|
| `// md:fn put_resource_data` | `max_user_storage_bytes` | `Store::user_blob_bytes_excluding` |
| `// md:fn create_note` | `max_notes_per_user` | `Store::count_live_notes_for_user` |

Each reads the user's current total, adds what the request would write, compares against the
configured limit, refuses with `507 QuotaExceeded` if it would be exceeded, and otherwise writes.

Two concurrent requests from the same user each read the total before either writes. Each sees
room. Both write. The sum exceeds the limit.

**Accepted [ADR 0002](0002-authorization-mutation-atomicity.md) does not close this, and that is
not an oversight in it.** Its general rule is re-verification inside the operation transaction,
which closes the interval between an early authorization check and a later mutation. This failure
is not staleness: neither request reads a stale value at any point. It is write skew. Re-verifying
inside the transaction re-reads the same total and still sees room, because the other request has
not committed yet and, once it has, the deciding read already happened.

ADR 0002's `SERIALIZABLE` set is defined as the handlers whose behaviour the written invariants of
accepted [ADR 0001](0001-note-moves-and-share-provenance.md) commit. ADR 0001 contains no mention of
quota or storage bytes — verified against the tree, zero occurrences. Quota therefore sits outside
that enumeration **by construction**, and the set is not short.

So the gap is real, it is reachable by the general rule's own terms, and no accepted decision
covers it.

### How this compares to the defect ADR 0002 was written for

Recorded because it inverts the usual priority argument, and because leaving it implicit would let
a reader assume the more elaborate machinery guards the more reachable defect.

`REV-122-02` requires a notebook-share insert to commit in the milliseconds between two queries, and
ADR 0002 records that an attacker cannot control that instant beyond retrying. Exceeding a quota
requires only issuing requests in parallel: there is no window to hit and no timing to win.

The counterweight is consequence, not reachability. Exceeding a quota costs disk. `REV-122-02`
silently removes a third party's access, violating an invariant of an accepted decision. Neither is
production-facing today. This ADR does not argue for reordering ADR 0002's phases; it argues that
quota should not stay undecided while they proceed.

## Forces and requirements

- A quota that can be exceeded by parallel requests is not a quota. Enforcement must hold under
  concurrency from the same user.
- The refusal must remain `507 QuotaExceeded` with its current body. A client must not be able to
  distinguish a refusal that arose under contention from an ordinary one.
- No client-visible retry protocol, and no `503` on this path. The server either admits the write or
  refuses it on the merits; it does not ask the client to try again.
- Users must not serialize against one another, beyond whatever collisions the chosen key admits.
- The deciding read and the write it authorizes must execute in one transaction. Phase 1 of ADR
  0002's implementation ([keeplin-srv#141](https://github.com/jsunyermias/keeplin-srv/pull/141))
  made that expressible by threading `&mut PgConnection` through the store; this decision depends on
  that seam and does not reintroduce a second mechanism for it.
- ADR 0002's enumerated eight-handler set does not change. This is an alternative mechanism for a
  different invariant, not an extension of that set.
- Whatever is chosen must state its contention cost rather than presenting per-user locking as free.

## Threat model

**Assets.** Server storage capacity, and the note-count limit that bounds per-user footprint.

**Trust boundary.** Concurrent requests from one authenticated user, interacting through PostgreSQL.

**Adversary.** An authenticated user who issues quota-bearing requests in parallel. They need no
timing precision and no knowledge of the server's internals; parallelism alone suffices. They cannot
affect another user's quota, because the aggregate is per user.

**Capabilities and consequence.** The user exceeds their own storage or note-count limit by
approximately the number of requests they can land concurrently, each up to one request's worth. The
overrun is bounded by concurrency, not unbounded, but it is repeatable: nothing prevents the user
from doing it again once the totals settle.

**Accepted leakage and limits.** The refusal discloses exactly what its non-racing equivalent
already discloses. Nothing about lock acquisition is visible to the client beyond latency. A user
who holds their own lock delays their own concurrent quota-bearing writes; that is the mechanism
working, not a denial-of-service surface against anyone else.

**Out of scope.** Rate limiting, which is a separate concern with its own configuration
(`rate_limit_per_min`), and total server capacity across all users, which no per-user quota bounds.

## Options considered

### Option 1 — Accept the overrun

Record that quotas are advisory under concurrency and do nothing. Cheapest, and defensible for a
system with no production deployment. Rejected because the limit is expressed as a hard refusal —
`507`, "storage limit reached" — and a limit that refuses one request while admitting two identical
ones concurrently is not the thing its own error message claims to be.

### Option 2 — Add these handlers to ADR 0002's `SERIALIZABLE` protocol

Make `put_resource_data` and `create_note` serializable, with the same three-attempt retry and `503`
on exhaustion. Correct, and reuses machinery that phase 2 will build anyway.

Rejected on cost and on shape. It pulls two handlers into a protocol whose retry budget and `503`
exhaustion path exist to protect ADR 0001's invariants; a quota check does not need dependency
tracking across predicates, only mutual exclusion between one user's own writers. It also introduces
a `503` on a path where the honest answer is always either "written" or "over limit", which is a
worse client contract for no gain.

### Option 3 — Per-user advisory transaction lock

Take `pg_advisory_xact_lock` keyed on the user, inside the operation transaction, **before** the
deciding read. Concurrent quota-bearing writes from the same user serialize; the lock releases
automatically when the transaction commits or rolls back.

No schema change, no migration, no new table, and no lock left behind by a crashed transaction.

### Option 4 — Per-user quota row with `SELECT … FOR UPDATE`

Add a `user_quota` table with one row per user, lock it with `FOR UPDATE`, and optionally cache the
running total there rather than recomputing an aggregate.

Stronger in one respect: the lock key is the row's identity, so there are no hash collisions, and a
cached total would remove the aggregate scan. Costs a migration, a row lifecycle to keep in step
with user creation and deletion, and a cached counter that can drift from reality — a second source
of truth for something the base tables already answer.

### Option 5 — Database constraint

Express the limit declaratively. Rejected as not expressible: PostgreSQL `CHECK` constraints cannot
range over an aggregate of other rows, so this reduces to a trigger maintaining a counter, which is
option 4 with the bookkeeping hidden inside the database and harder to test.

## Decision and justification

> This ADR is `proposed`. It records a recommendation and does not authorize implementation. Only
> the maintainer may accept or reject it.

**Proposed decision: adopt Option 3.** Both quota-bearing handlers take a per-user advisory
transaction lock, inside the transaction that performs the write, before the read that decides the
outcome. The deciding read runs on that transaction through its executor-aware store form, and the
write follows in the same transaction.

The ordering is the whole decision, and it is the part an implementation can get subtly wrong: a
lock taken *after* the read serializes nothing, because both requests have already read. The
verification plan below requires evidence that fails when the lock moves after the read.

Option 3 over option 4 because the extra strength option 4 buys — a collision-free key, and a place
to cache the total — is not worth a migration and a second source of truth for a limit that two
aggregate queries already answer correctly. If the aggregate scan ever becomes a measured problem,
option 4 remains available as a later decision, and this ADR does not foreclose it.

Option 3 over option 2 because mutual exclusion between one user's own writers is what this
invariant needs, and `SERIALIZABLE` with retry and `503` is a heavier contract that would change the
client-visible failure modes of a path whose honest answers are only "written" and "over limit".

The proposed invariants are:

1. A quota-bearing write commits only if the user's total, read inside the same transaction and
   under the per-user lock, leaves room for it.
2. The lock is acquired before the deciding read, and both precede the write, in one transaction.
3. Concurrent quota-bearing writes from the same user are serialized; from different users they are
   not, except where the lock key collides.
4. A refusal is `507 QuotaExceeded` with the body it has today, indistinguishable from a refusal
   issued with no contention.
5. No `503`, no retry loop, and no client-visible protocol change on this path.
6. ADR 0002's eight-handler `SERIALIZABLE` enumeration is unchanged by this decision.

### The cost, stated rather than implied

`pg_advisory_xact_lock` takes a 64-bit key, and a user identifier is a UUID. Any mapping from 128
bits to 64 admits collisions. Two users whose keys collide serialize their quota-bearing writes
against each other.

This is a latency cost, not a correctness one: a collision makes the lock stricter than necessary,
never weaker. It is stated here because "per-user locking" reads as though users are isolated, and
with a hashed key they are isolated only up to collisions.

The advisory-lock space is shared across the database. Nothing else in this repository uses advisory
locks today; anything that later does must not reuse this key space without revisiting this
decision.

### Not decided

- The exact key derivation from a UUID. Any stable 64-bit mapping satisfies the invariants, and the
  collision consequence is the same for all of them.
- Whether other future per-user invariants should share this lock. They may, but each would need to
  establish that sharing does not deadlock, and this decision does not pre-authorize it.
- Total server capacity across all users, which no per-user quota bounds and which this ADR does not
  address.
- Whether the aggregate reads should later be replaced by a maintained counter. That is option 4 and
  remains available.

## Consequences and risks

- A user issuing many quota-bearing writes concurrently has them serialized. That is the intended
  behaviour and it is visible to them as latency.
- Colliding lock keys serialize two unrelated users. Stricter than needed, never wrong.
- Both quota reads must be executor-aware. `user_blob_bytes_excluding` already is;
  `count_live_notes_for_user` is not, and this decision requires converting it — resolving an
  asymmetry phase 1 left behind for no reason other than scope.
- A transaction now spans the quota read and the write on both paths, holding a connection longer
  than the current independent pool operations. On `put_resource_data` that transaction also spans
  the blob write.
- Deadlock risk is nil under this decision as written: each transaction takes at most one advisory
  lock, so there is no ordering hazard. That property must be preserved if another invariant later
  joins the same lock.

## Compatibility, migration, and rollback

No schema change, no migration, no wire or format change. Nothing in `keeplin-core` is affected, and
no pin moves.

The client contract is unchanged: the same `507` with the same body, and no new status on this path.

Rollback is removing the lock acquisition. The system returns to today's behaviour, in which quotas
hold for sequential requests and can be exceeded by concurrent ones.

## Verification plan

| # | Evidence | Kind | What fails if the decision is violated |
|---|---|---|---|
| 1 | Two concurrent `put_resource_data` requests from one user, each individually within the limit but jointly exceeding it, produce exactly one success and one `507`, and the stored total never exceeds `max_user_storage_bytes` | concurrency | Fails if both commit, or if the total exceeds the limit at any observable point |
| 2 | The same for `create_note` against `max_notes_per_user` | concurrency | Fails if the note count exceeds the limit |
| 3 | Moving the lock acquisition to after the deciding read makes rows 1 and 2 fail | mutation | Fails if the tests pass with the lock in a position that serializes nothing, which would mean they never established the ordering |
| 4 | Redirecting either quota read to `Store`'s pool makes a test fail | mutation | Fails if the deciding read can escape the transaction while tests stay green |
| 5 | Two different users issue concurrent quota-bearing writes and are observed to overlap rather than serialize, except under a deliberately collided key | concurrency, negative | Fails if the lock is taken on something coarser than the user, serializing the whole server |
| 6 | A refusal issued under contention is byte-equivalent to one issued with no contention: same status, same body | compatibility | Fails if a concurrency-specific status or message leaks |
| 7 | No `503` and no retry appear on either quota path | structural | Fails if ADR 0002's retry protocol is applied here by copying |
| 8 | `count_live_notes_for_user` has an executor-aware form and the handler uses it | structural | Fails if one quota read runs on the transaction and the other does not |
| 9 | ADR 0002's eight-handler serializable enumeration is unchanged, and neither quota handler starts a `SERIALIZABLE` transaction | structural | Fails if this decision silently widens that set |
| 10 | Failure injection between the deciding read and the write leaves no partial state and no lock held after rollback | recovery | Fails if a crash can commit a write whose quota check did not authorize it, or strand a lock |
| 11 | `./scripts/check-docs.sh` passes with every changed source companion and project document synchronized | documentation | Fails if implementation and documentation diverge |
| 12 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` pass | repository | Fails on behavioral regressions, warnings, or formatting drift |

Rows 3 and 4 are mutation evidence rather than tests, following the argument in
[keeplin-srv#138](https://github.com/jsunyermias/keeplin-srv/issues/138): during ADR 0002's phase 1 a
test named for exactly the property it was meant to establish passed for a full review round while
that property was violated. A test that asserts the outcome without failing when the mechanism is
removed has not established the mechanism.

No cross-repository, schema-migration, or format-recovery test is required because this decision
changes none of those surfaces.

## Equivalent decision in the other repository

None. Quotas are a server concept: `keeplin` has no per-user aggregate limit and no equivalent
enforcement point, so there is nothing to mirror and no companion ADR to link. `keeplin-core` is
untouched and its pin does not move.
