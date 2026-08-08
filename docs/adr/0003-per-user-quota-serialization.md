# 0003 — Making per-user quotas hold

- Status: proposed
- Date: 2026-08-08
- Decision owners: maintainer of `jsunyermias/keeplin-srv`
- Scope: keeplin-srv
- Issue: [keeplin-srv#142](https://github.com/jsunyermias/keeplin-srv/issues/142), [keeplin-srv#145](https://github.com/jsunyermias/keeplin-srv/issues/145)
- Acceptance PR: link once the ADR is accepted
- Supersedes: none
- Superseded by: none

## Context and problem

`Verified at`: `keeplin-srv@b5b82b1`.

The server declares two per-user quotas, `max_notes_per_user` and `max_user_storage_bytes`, and
refuses over-limit requests with `507 QuotaExceeded`. Neither holds.

The first draft of this ADR asked where each quota is **checked**. That question returns two
handlers and looks complete. The question that matters is where the counted object is **created**,
and it returns four places:

| Counted object | Write path | Enforcement today |
|---|---|---|
| note | `// md:fn create_note` | reads `count_live_notes_for_user`, refuses over `max_notes_per_user` |
| note | `// md:fn import_note` | **none** — calls `Store::create_note` at `http.rs:1541` |
| resource blob | `// md:fn put_resource_data` | reads `user_blob_bytes_excluding`, refuses over `max_user_storage_bytes` |
| resource blob | synchronization, `Change::ResourceCreate` | **none** — reaches `Store::put_resource_blob` at `sync.rs:369`; `sync.rs` contains zero occurrences of any quota symbol |

So there are two distinct defects, and they are not equally serious.

**Absent enforcement.** Two of the four write paths never consult the limit. A client exceeds either
quota by choosing the other endpoint: import instead of create, synchronize instead of upload. No
concurrency, no timing, no privilege. This is unbounded and repeatable.

**Write skew on the guarded paths.** On the two paths that do check, two concurrent requests from
one user can each read the total before either writes, each see room, and both commit.

### What re-verification does and does not close

Accepted [ADR 0002](0002-authorization-mutation-atomicity.md) requires re-verifying a guard inside
the operation transaction. Applied to a quota, that is worth stating precisely, because the first
draft of this ADR overstated it.

PostgreSQL's default is `READ COMMITTED`, where each statement takes a fresh snapshot. So
re-verification **does** close every schedule in which one request commits before the other
re-reads: the second re-read sees the new total and the request is refused. What survives is the
schedule in which both re-reads complete before either commit — each still sees room, and neither
has read a stale value at any point.

Re-verification therefore narrows the window rather than leaving it untouched. It does not close it,
because the surviving schedule is the one an adversary gets by simply issuing requests together.
This is write skew, and no per-statement freshness prevents it.

ADR 0002's `SERIALIZABLE` set is defined as the handlers whose behaviour the written invariants of
accepted [ADR 0001](0001-note-moves-and-share-provenance.md) commit. ADR 0001 contains no mention of
quota or storage bytes — verified, zero occurrences. Quota sits outside that enumeration by
construction, and the set is not short.

### The overrun is bounded and self-limiting

Also corrected from the first draft, which claimed the overrun was "repeatable once the totals
settle". It is not. Once the stored total exceeds the limit, every subsequent guarded write is
refused, because the deciding read now returns a figure already over the limit. The concurrency
defect therefore yields approximately one round of overrun, bounded by how many requests land
together.

The absent-enforcement defect has no such bound: an unguarded path never consults the total at all.

## Forces and requirements

- A quota that any write path can ignore is not a quota. Enforcement must exist wherever the counted
  object is created, not wherever someone remembered to check.
- A quota that concurrent requests can jointly exceed is not a quota either. The check and the write
  it authorizes must be mutually exclusive per user.
- The HTTP refusal stays `507 QuotaExceeded` with its current body. A client must not be able to
  distinguish a refusal issued under contention from an ordinary one.
- No client-visible retry protocol and no `503` on these paths: the honest answers are "written" and
  "over limit".
- Users must not serialize against one another beyond what the chosen key admits.
- The deciding read and the write must execute in one transaction, which phase 1 of ADR 0002's
  implementation ([keeplin-srv#141](https://github.com/jsunyermias/keeplin-srv/pull/141)) made
  expressible.
- Completeness must be established mechanically. Inspection is what missed the two unguarded paths;
  a decision that relies on the same inspection to stay correct has learned nothing.
- ADR 0002's eight-handler enumeration does not change.

## Threat model

**Assets.** Server storage capacity, and the note-count limit that bounds per-user footprint.

**Trust boundary.** An authenticated user's requests, across both the HTTP surface and the
synchronization path, interacting through PostgreSQL.

**Adversary.** An authenticated user. Against absent enforcement they need only call the unguarded
endpoint. Against write skew they need only issue guarded requests in parallel. Neither requires
timing precision, knowledge of internals, or another user's cooperation.

**Capabilities and consequence.** Through an unguarded path, a user's total is unbounded by the
declared limit. Through the guarded paths, they exceed it by roughly one round of concurrent
requests, after which further guarded writes are refused. A user cannot affect another user's
quota; the aggregate is per user.

**Accepted leakage and limits.** The refusal discloses exactly what its non-racing equivalent
discloses. Lock acquisition is invisible to the client except as latency. A user waiting on their
own lock delays only their own writes.

**Out of scope.** Rate limiting, which has its own configuration; total server capacity across all
users, which no per-user limit bounds; and quota accounting for objects deleted and recreated, which
the existing aggregate queries already answer by reading live rows.

## Options considered

### Option 1 — Accept both defects

Record quotas as advisory. Rejected: the server refuses requests with `507` and a message naming a
limit, so the limit is asserted to the client. A limit that refuses one request while admitting the
same bytes through another endpoint is not the thing its own error message claims.

### Option 2 — Enforce everywhere, and use `SERIALIZABLE` with retry for the race

Add the missing checks, and put the four paths under ADR 0002's serializable protocol with its
three-attempt retry and `503` on exhaustion.

Rejected, and on a stronger ground than the first draft gave. The first draft argued only that a
`503` is a worse client contract on a path whose honest answers are "written" and "over limit" —
which is partly circular, since that contract is itself a force this ADR chose. The substantive
objection is cost: under retry, a losing `put_resource_data` re-executes its blob write on each
attempt, and a resource blob is arbitrarily large. A user can be made to pay bandwidth and database
work two extra times only to reach the same `507` they would have reached immediately. An advisory
lock converts that contention into a wait rather than into repeated work.

### Option 3 — Enforce at every write path, and serialize per user with an advisory lock

Add the missing enforcement, and take a `pg_advisory_xact_lock` before the deciding read, inside the
transaction that writes. Concurrent quota-bearing writes from the same user serialize; the lock
releases at commit or rollback.

No schema change, no migration, no row lifecycle, and no lock stranded by a crash.

### Option 4 — Enforce everywhere, and serialize with a `user_quota` row

Add a table with one row per user, locked with `SELECT … FOR UPDATE`.

The first draft undersold this. Its minimal form — a row used purely as a lock target, with no
cached total — has no second source of truth, no drift, no hash collisions, and can be keyed per
quota rather than per user. It is closer to option 3 than "costs a migration and a cached counter"
suggested; the cached total is optional, and only that optional part carries drift.

What remains against it is real but narrower than claimed: a migration, and a row lifecycle that has
to stay in step with user creation and deletion — including the question of what happens when the
row is missing, which is a new failure mode that option 3 does not have.

### Option 5 — Database constraint

Not expressible: a `CHECK` cannot range over an aggregate of other rows, so this reduces to a
trigger maintaining a counter, which is option 4 with the bookkeeping hidden where it is harder to
test.

## Decision and justification

> This ADR is `proposed`. It records a recommendation and does not authorize implementation. Only
> the maintainer may accept or reject it.

**Proposed decision: adopt Option 3, in two parts that must land together to mean anything.**

**Part one — enforcement is a property of the write, not of the handler.** Every path that creates a
counted object consults the limit: `create_note`, `import_note`, `put_resource_data`, and the
synchronization path's `Change::ResourceCreate`. Completeness is established by a structural
inventory keyed to the **call sites that create the counted object**, which fails when a new one
appears uncovered — the shape of the handler-authorization inventory added in
[keeplin-srv#141](https://github.com/jsunyermias/keeplin-srv/pull/141). An inventory keyed to the
guards would have reported this system complete on the day it was not.

**Part two — the check and the write are mutually exclusive per user and per quota.** Each
quota-bearing transaction takes a `pg_advisory_xact_lock` **before** the deciding read, and performs
the read and the write on that transaction.

The ordering is the decision, and it is what an implementation gets subtly wrong: a lock taken
*after* the read serializes nothing, because both requests have already read. The verification plan
requires evidence that fails when the lock moves after the read.

**The lock key is per user and per quota kind, with a namespace discriminator.** Two consequences,
both deliberate:

- A user's note-creating writes do not serialize against their own blob writes. They contend for
  different limits and sharing one key would be false sharing.
- No path in this decision touches both quotas, so each transaction takes **at most one** advisory
  lock and no deadlock cycle is constructible. That property is load-bearing and the verification
  plan makes it structural rather than a comment.

The discriminator exists because the advisory-lock space is global to the database. Nothing else in
this repository uses advisory locks today, and an unnamespaced key would make the next user of that
space collide with this one silently.

The proposed invariants are:

1. No path creates a counted object without consulting the limit that counts it.
2. A quota-bearing write commits only if the total, read inside the same transaction and under the
   lock, leaves room for it.
3. The lock is acquired before the deciding read; both precede the write; all three are one
   transaction.
4. Concurrent quota-bearing writes from the same user for the same quota are serialized. Different
   users, and different quotas for one user, are not, except where the key collides.
5. Each transaction acquires at most one advisory lock.
6. An HTTP refusal is `507 QuotaExceeded` with today's body, indistinguishable from a refusal issued
   with no contention.
7. ADR 0002's eight-handler `SERIALIZABLE` enumeration is unchanged.

### Costs, stated rather than implied

**Hash collisions.** `pg_advisory_xact_lock` takes 64 bits and a user identifier is a UUID. Any
mapping admits collisions, so two colliding users serialize against each other. This makes the lock
stricter, never weaker — a latency cost, not a correctness one. It is stated because "per-user
locking" reads as though users are isolated, and with a hashed key they are isolated only up to
collisions.

**Waiting is not free, and it is not invisible.** A request that waits on its own user's lock takes
longer. If a statement or lock timeout is configured, the wait can fail, and that failure is an
internal error rather than a quota refusal — it must not be reported as `507`, because the request
was never determined to be over limit. Invariant 6 constrains what a *refusal* looks like; it does
not promise that no request can fail for any other reason.

**A transaction now spans the deciding read and the write.** On `put_resource_data` that transaction
also spans the blob write, holding a connection for the duration of an arbitrarily large statement.

### Not decided

- **How the synchronization path refuses an over-limit change.** It is not an HTTP request, so `507`
  is not its natural refusal, and rejecting a synchronized change has consequences for convergence
  that the HTTP path does not have. This ADR requires that the path enforce the limit; it does not
  decide the representation of that refusal, and that must be decided before implementation rather
  than during it.
- The exact 64-bit key derivation. Any stable mapping satisfies the invariants and the collision
  consequence is identical for all of them.
- Whether future per-user invariants share this lock space. They may, but each must re-establish
  invariant 5, and this decision does not pre-authorize it.
- Whether the aggregate reads should later become maintained counters. That is option 4's optional
  part and remains available as a later decision.

## Consequences and risks

- Two write paths gain a check they do not have today, which will refuse requests that succeed now.
  That is the point, and it is a behaviour change visible to any client currently over its limit.
- A user issuing many writes against one quota has them serialized. Visible as latency.
- Colliding lock keys serialize two unrelated users. Stricter than needed, never wrong.
- `count_live_notes_for_user` must gain an executor-aware form; `user_blob_bytes_excluding` already
  has one. Phase 1 of ADR 0002 converted one and not the other, and this decision resolves that
  asymmetry rather than inheriting it.
- The synchronization path acquires a per-user lock it does not take today, on a path whose latency
  characteristics differ from HTTP.
- Deadlock risk is nil as written, by invariant 5. It stops being nil the moment a transaction takes
  two advisory locks, which is why that invariant is structural rather than advisory.

## Compatibility, migration, and rollback

No schema change, no migration, no wire or format change. `keeplin-core` is untouched and its pin
does not move.

The HTTP client contract is unchanged in shape: the same `507` with the same body. It changes in
reach — two paths that never refused will now refuse. A deployment whose users are already over a
limit will begin refusing their writes on those paths, which is the intended correction and should
not be discovered in production.

Rollback of part two is removing the lock acquisition, returning to quotas that hold sequentially
and not concurrently. Rollback of part one is removing the added checks, returning to quotas that do
not hold at all; it should be considered a rollback of the whole decision rather than of a part.

## Verification plan

| # | Evidence | Kind | What fails if the decision is violated |
|---|---|---|---|
| 1 | A structural inventory enumerates every call site that creates a note or writes a resource blob, and fails when one is not covered by an enforcement point | structural | Fails if a new write path can be added without enforcement, which is exactly how `import_note` and the sync path came to be unguarded |
| 2 | Creating notes through `import_note` past `max_notes_per_user` is refused | negative | Fails if the note-count limit remains bypassable by importing |
| 3 | Writing blobs through the synchronization path past `max_user_storage_bytes` is refused, in whatever representation that path's refusal takes | negative | Fails if the storage limit remains bypassable by synchronizing |
| 4 | Two concurrent `put_resource_data` requests from one user, each individually within the limit but jointly exceeding it, with a test-controlled rendezvous that holds both transactions between their deciding read and their write: exactly one succeeds, one is refused, and the stored total never exceeds the limit | concurrency, forced interleaving | Fails if both commit. The rendezvous is required: without it the test can pass on scheduling luck while the mechanism is absent |
| 5 | The same for `create_note` against `max_notes_per_user` | concurrency, forced interleaving | Fails if the note count exceeds the limit |
| 6 | Moving the lock acquisition to after the deciding read makes rows 4 and 5 fail | mutation | Fails if those rows pass with the lock in a position that serializes nothing, which would mean they never established the ordering |
| 7 | Redirecting either quota read to `Store`'s pool makes a test fail | mutation | Fails if the deciding read can escape the transaction while tests stay green |
| 8 | Two different users issue concurrent quota-bearing writes and both reach a test-controlled barrier between read and write; neither blocks the other | concurrency, deterministic | Fails if the lock is coarser than the key it claims: with a shared key the second transaction never reaches the barrier and the test fails by timeout rather than by luck |
| 9 | Coarsening the lock key to a constant makes row 8 fail | mutation | Fails if row 8 is insensitive to granularity, which would make it a timing observation rather than evidence |
| 10 | A user's `create_note` and `put_resource_data` proceed concurrently, demonstrating that the two quotas do not share a key | concurrency, deterministic | Fails if one key serves both quotas, reintroducing the false sharing this decision rejects |
| 11 | A structural assertion that no transaction acquires more than one advisory lock, and that every advisory-lock call site uses the namespaced key derivation | structural | Fails if a second call site takes a raw key, or if a path takes two locks, either of which makes invariant 5 and the deadlock argument false |
| 12 | An HTTP refusal issued under contention is byte-equivalent to one issued with no contention | compatibility | Fails if a concurrency-specific status or message leaks |
| 13 | No `503` and no retry appear on any quota path | structural | Fails if ADR 0002's retry protocol is applied here by copying |
| 14 | A failed lock wait surfaces as an internal error and never as `507` | negative | Fails if a request that was never determined to be over limit is reported as over limit |
| 15 | `count_live_notes_for_user` has an executor-aware form and every enforcement point uses one | structural | Fails if one quota read runs on the transaction and another does not |
| 16 | Failure injection between the deciding read and the write leaves no partial state and no lock held after rollback | recovery | Fails if a crash can commit a write its quota check did not authorize, or strand a lock |
| 17 | `./scripts/check-docs.sh` passes with every changed source companion and project document synchronized | documentation | Fails if implementation and documentation diverge |
| 18 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` pass | repository | Fails on behavioral regressions, warnings, or formatting drift |

Rows 6, 7 and 9 are mutation evidence rather than tests, and rows 4, 5 and 8 require a forced
rendezvous rather than hoping for an interleaving. Both follow the argument in
[keeplin-srv#138](https://github.com/jsunyermias/keeplin-srv/issues/138): during ADR 0002's phase 1 a
test named for exactly the property it was meant to establish passed for a full review round while
that property was violated. A test that asserts an outcome without failing when the mechanism is
removed has not established the mechanism, and a concurrency test without a rendezvous can pass on
scheduling luck in either direction.

No cross-repository, schema-migration, or format-recovery test is required because this decision
changes none of those surfaces.

## Equivalent decision in the other repository

None. Quotas are a server concept: `keeplin` has no per-user aggregate limit and no equivalent write
path, so there is nothing to mirror and no companion ADR to link. `keeplin-core` is untouched and
its pin does not move.
