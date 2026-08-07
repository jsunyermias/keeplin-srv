# 0002 — Authorization and mutation atomicity

- Status: proposed
- Date: 2026-08-07
- Decision owners: maintainer of `jsunyermias/keeplin-srv`
- Scope: keeplin-srv
- Issue: [keeplin-srv#123](https://github.com/jsunyermias/keeplin-srv/issues/123)
- Acceptance PR: none; link once accepted
- Supersedes: none
- Superseded by: none

## Context and problem

`Verified at`: `keeplin-srv@25efb30`.

The HTTP layer contains 17 references to `resolve_note_access` or
`resolve_notebook_access`, and contains no call to `begin()` or `.transaction(`. The mutating
handlers among those call sites therefore read authorization state through the pool and later
mutate through a separate pool operation. Nothing makes the authorization fact and the mutation it
authorizes one atomic decision. The resolver count is not a complete inventory: mutating handlers
such as `put_resource_data`, `change_password`, and `delete_account` use handler-specific ownership,
credential, or authenticated-identity inputs and call neither resolver. This is an HTTP
permission-surface pattern, not a property unique to note moves.

The concrete instance recorded as `REV-122-02` in
[keeplin-srv#123](https://github.com/jsunyermias/keeplin-srv/issues/123) is the move guard in
`// md:fn update_note`. Alice owns notebook *N* and note *X*, which is inside *N*:

1. Alice begins moving *X* out of *N*.
2. The guard reads `Store::inherited_note_principals(N)`; Carol is not yet present.
3. A concurrent request inserts Carol's notebook share on *N*. Carol now has inherited access to
   *X*.
4. `Store::update_note_meta` moves *X* out. Carol loses access without a refusal or notice.

Carol is a principal Alice controls because Alice owns *N* and can undo its grants. The accepted
[keeplin-srv ADR 0001](0001-note-moves-and-share-provenance.md), invariant 6, requires:

> **A move never removes a third party's access silently — where the mover controls that access.**
> A move that would drop the inherited access of another principal is refused **when that access
> derives from a grant the mover can themselves undo**. The mover resolves it explicitly first:
> preserving the access needs no notice, while revoking it does. Access deriving from a grant the
> mover does not control does not block: the move succeeds and the affected principal is notified.
> See the ordering rule.

The interleaving can therefore violate an invariant of an accepted decision. Exploitability is
low: the share insert must commit in the milliseconds between two queries, and an attacker cannot
control that instant beyond retrying. The consequence is nevertheless real: access is silently
removed when ADR 0001 requires the move to be refused until Alice chooses Preserve or Revoke.

A durable decision is needed for the relationship between every HTTP authorization check and the
mutation it authorizes. Repairing only the move guard would leave the same unresolved pattern at
the other mutating resolver call sites and no rule by which future handlers can be reviewed.

## Forces and requirements

- Every mutation must commit only if the authorization and policy guard that authorizes it still
  holds when re-evaluated in the transaction that commits the mutation. At PostgreSQL's default
  `READ COMMITTED` isolation this narrows, but does not close, the interval in which that fact can
  become stale.
- The audit boundary is every HTTP mutating handler and every authorization or policy input it
  uses, including handler-specific guards and authenticated-identity constraints. Resolver
  references are an input to that inventory, not its key.
- A changed guard outcome must leave no mutation committed.
- A move newly blocked by a controlled principal must produce the ordinary ADR 0001 refusal: HTTP
  `403` with the same `MoveBlocked` body and the same named and counted principals.
- A client must not need a concurrency-specific retry path and must not learn whether an ordinary
  refusal arose during re-verification.
- The mechanism must avoid turning an active shared notebook into a serialization bottleneck.
- Serialization failures on the selected invariant-bearing paths must be retried by the server;
  no client-side serialization-retry protocol is introduced.
- Guard logic must be usable both before the transaction and within it. Store reads used by
  `resolve_note_access`, `resolve_notebook_access`, and handler-specific guards must accept the
  transaction rather than silently escaping to the pool.
- ADR 0001's notice ordering survives: a failed notice does not roll back the operation that owed
  it, and notices are attempted only after commit.
- The implementation must update every affected source companion and project document and provide
  mechanical tests that fail if an authorization/mutation window is reintroduced.
- This `proposed` ADR does not authorize implementation. Only the maintainer may change it to
  `accepted` or `rejected`.

## Threat model

**Assets.** Note and notebook confidentiality, integrity, ownership, sharing state, and the
ADR 0001 guarantee that controlled access is not removed silently.

**Trust boundary.** Concurrent authenticated requests whose authorization and mutations interact
through PostgreSQL. The server and database are trusted to enforce a correctly expressed
transaction; authenticated callers are not trusted to avoid adversarial timing.

**Adversary.** An authenticated caller who can repeatedly invoke an otherwise authorized endpoint
or cause an otherwise authorized share change. They cannot select an exact database scheduling
instant, but can retry to increase the chance of the narrow interleaving.

**Capabilities and consequence.** The demonstrated interleaving silently removes Carol's inherited
access. Other call sites may admit the corresponding time-of-check/time-of-use class: a mutation
committing after the permission or policy fact that authorized it has changed. This ADR does not
claim that all 17 call sites have the same consequence or exploitability.

**Accepted leakage and limits.** The ordinary refusal may disclose exactly what its non-racing
equivalent already discloses. No concurrency-specific status or body is added for a guard refusal.
Notices have the same commit-then-effect shape as the critical journal-to-projection defect tracked
by [keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75). An outbound send cannot be
made atomic with PostgreSQL, but an intent-to-notify record could be written in the same database
transaction. Leaving that intent outside is therefore a choice, not a difference in kind. This ADR
makes that choice for scope discipline: accepted ADR 0001's ordering rule already decides that a
failed notice does not roll back the operation, and #75 may later supply reusable outbox machinery.
The accepted loss window is process failure after commit but before the send attempt. ADR 0001 also
owns the destination of the failure record: its ordering-rule text specifies a runtime
`tracing::warn!` log line rather than persistent state, and its verification-plan row 14 requires
that a failed notice leave the revocation committed and record the failure. This ADR does not make
that record durable or narrow that prior decision; its missing coverage is being added under
[keeplin-srv#123](https://github.com/jsunyermias/keeplin-srv/issues/123).

**Non-goals.** This ADR does not redesign capabilities, ADR 0001's permission schemes, or its
Preserve/Revoke policy. It does not decide transaction isolation for unrelated database work, make
notices durable, or address authorization in collaboration/WebSocket surfaces. The journal-to-
projection path in `sync.rs` is positively out of scope; its separate transaction-boundary defect
belongs to [keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75).

## Options considered

### Option 1 — Keep the current split operations

Benefits: no refactor and no added queries. Operational burden is unchanged.

Costs and failure modes: authorization can become stale before its mutation commits. The verified
move interleaving violates accepted ADR 0001 invariant 6. Evidence that would change the assessment
would be a database guarantee or single atomic statement proving that every guarded mutation is
conditioned on the same authorization state; neither exists in the current HTTP layer.

### Option 2 — Repair only the move guard with a pessimistic source-notebook lock

Acquire `FOR UPDATE` on the source notebook before checking affected principals and hold it through
the move.

Benefits: localizes the first repair and makes share changes wait behind a move.

Costs and failure modes: it leaves sixteen sibling resolver references without a general rule. It
also serializes every move and every share change on the source notebook, making an active shared
notebook a bottleneck. Operational burden is lock contention and the need to reason about lock
ordering. Evidence that would change the assessment would be measured contention showing this lock
is immaterial together with proof that no sibling call site can authorize a mutation; the current
call-site audit does not support the latter premise.

### Option 3 — Lock only rows read by each guard

Run the guard's queries with `SELECT ... FOR UPDATE` and hold the returned row locks through the
mutation.

Benefits: permits unrelated existing rows to change concurrently and avoids locking the common
notebook row.

Costs and failure modes: PostgreSQL row-level locks protect only rows actually returned. The
demonstrated competing operation is an `INSERT` of a previously nonexistent `notebook_shares` row,
so there is no row for the guard to lock. The inserted share is a phantom and can commit after the
guard query. Closing that case requires either locking a common existing parent row, as Option 2
does, or `SERIALIZABLE` isolation, whose predicate locking and dependency checks can abort a
non-serializable execution. This option therefore cannot establish the move invariant. PostgreSQL's
[row-level lock documentation](https://www.postgresql.org/docs/current/explicit-locking.html#LOCKING-ROWS)
defines these locks over retrieved rows, not absent predicate matches.

### Option 4 — Use `SERIALIZABLE` transactions with server-side retry

Run selected guarded endpoints at PostgreSQL `SERIALIZABLE` isolation and retry serialization
failures in the server.

Benefits: asks the database to reject non-serializable interleavings, including the phantom insert,
and can close the authorization/mutation window without a common-parent bottleneck.

Costs and failure modes: retry requires the whole transaction, including its guard, to be replayed.
It is inappropriate as a repository-wide default without proving every mutation retry-safe.
Serialization failures can persist through the server's bound and then must surface as a transient
server failure. Monitoring and contention remain operational concerns.

### Option 5 — Re-verify inside the mutation transaction

Perform the existing early guard for normal refusal, open a transaction, re-read the authoritative
resource and re-evaluate the complete authorization and policy guard on the state immediately
before the proposed mutation, then mutate and commit only if it still authorizes the operation. If
it changed, return the guard's real refusal and roll back.

Benefits: narrows the authorization/mutation window with a server-local contract, preserves the
endpoint's existing refusal surface, and avoids a notebook-wide pessimistic lock.

Costs and failure modes: guard logic executes twice. `resolve_note_access`,
`resolve_notebook_access`, and their store reads must be refactored to run against either the pool
or a transaction without accidentally mixing executors. Handler-specific guard reads, the mutation,
and re-verification must all use the same transaction. The extra reads add database work to every
guarded mutation. A missed store read that still uses the pool would recreate the defect despite
the appearance of a transaction.

At PostgreSQL's default `READ COMMITTED` isolation, each statement sees a snapshot taken when that
statement begins. A concurrent authorization change can therefore commit after the final
re-verification statement begins and before this transaction commits. Re-verification alone
narrows the window to that residual interval; it does not close it. `REPEATABLE READ` does not by
itself establish the application invariant either. The two mechanisms considered here that close
the phantom window are a common-parent lock or `SERIALIZABLE`. These are the documented PostgreSQL
[isolation semantics](https://www.postgresql.org/docs/current/transaction-iso.html), including the
requirement to retry a complete serializable transaction after a serialization failure.

## Decision and justification

> This ADR is `proposed`. It records the maintainer's selected recommendation but does not authorize
> implementation. Only the maintainer may accept or reject it.

**Proposed decision: adopt Option 5 as the general rule for the whole server HTTP permission
surface, plus Option 4 for accepted-ADR invariant paths.** Inventory every mutating HTTP handler and
its authorization inputs. Whenever a resolver result, authenticated identity, credential check, or
handler-specific policy guard authorizes a mutation, the server performs the ordinary early check,
then re-reads the authoritative resource and re-evaluates the complete guard inside the transaction
immediately before performing the mutation. A changed outcome rolls the transaction back. Under
`READ COMMITTED` this general rule narrows the window but leaves the interval from the start of the
final guard statement through commit.

The current repository has one accepted local ADR, ADR 0001. Its written invariants commit the
behavior of exactly these eight HTTP mutation handlers, so these paths additionally use
`SERIALIZABLE`: `update_note` (including the move authority, destination, controlled-principal, and
equal-principal guards), `delete_note`, `create_share`, `delete_share`, `transfer_ownership`,
`create_notebook_share`, `delete_notebook_share`, and `transfer_notebook`. The note-share paths
preserve direct-grant exclusivity and the revocation ordering; the notebook-share and ownership
paths change inputs to computed inheritance, its ceiling, and the move guard; delete and transfer
preserve ADR 0001's ownership-bound powers. No other accepted ADR exists in this repository, so the
set is not implicitly larger. Adding or accepting an ADR with a mutation invariant requires this
enumeration to be revisited.

Every transaction that can read or write the state participating in one of those invariants must
join the `SERIALIZABLE` protocol; making only the move transaction serializable would not establish
the invariant against a `READ COMMITTED` share writer. On SQLSTATE `40001`, the server aborts and
replays the complete transaction from its first read, for at most three total attempts (the initial
attempt plus two retries). No notice or other external effect occurs until an attempt commits. If
all three attempts fail, no mutation has committed and the server returns a generic HTTP
`503 Service Unavailable`, records the exhausted retry in server telemetry, and does not
manufacture a guard refusal. Thus ordinary serialization conflicts are hidden by the server and a
real guard change still has its established refusal, but exhaustion is an explicit, unavoidable
transient failure rather than a promise that concurrency can never surface.

Re-verification always evaluates whether the proposed operation is authorized against the
transaction's current **pre-mutation** state. It does not evaluate a guard against state that the
mutation itself has already changed. Inputs that identify the object and relationship under review
come from the authoritative transaction re-read; request identity remains captured and immutable.
For a move, this means the source notebook is the note's `notebook_id` re-read before
`update_note_meta`, and the affected-principal guard remains keyed to that source while evaluating
the proposed destination. The existing row-1 interleaving test would catch the naive behavior that
mutates first and then derives the source from the moved note.

The proposed invariants are:

1. No HTTP mutation commits solely on an authorization or policy fact read outside its transaction.
2. The mutation and re-verification use one transaction; every store read used by the
   re-verification executes on that transaction, and re-verification sees the pre-mutation state
   plus the proposed mutation as inputs.
3. A changed outcome rolls back and returns exactly the refusal the current state requires. For the
   demonstrated move, that is the same HTTP `403` and `MoveBlocked` shape, naming the same
   principals, as an ordinary ADR 0001 guard refusal.
4. A client cannot distinguish a re-verification refusal from the equivalent ordinary refusal.
   `409 Conflict` is not introduced.
5. Notices remain outside the transaction and are attempted after commit. Notice failure never
   rolls back the operation that owed it.
6. Every accepted-ADR invariant path enumerated above commits only as a serializable execution, and
   every serialization failure is retried wholly by the server within the stated bound.
7. On paths governed only by the general rule, the final guard can still become stale after its
   statement snapshot is taken. This residual `READ COMMITTED` window is accepted, not described as
   closed.

This combination closes the demonstrated phantom window for the accepted invariants, applies one
reviewable re-verification rule to the whole HTTP mutation surface, confines compatibility work to
the server, and avoids both a common-parent bottleneck and repository-wide `SERIALIZABLE`.

### Not decided

- Durable notice delivery, retry, and queue semantics. ADR 0001 verification-plan row 14 already
  decides the current requirement: "A revocation whose notice fails to send still commits, and the
  failure is recorded." Its missing test coverage is being implemented under
  [keeplin-srv#123](https://github.com/jsunyermias/keeplin-srv/issues/123). If
  [keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75) later chooses a durable
  outbox for its separate journal-to-projection problem, that machinery could also close the
  commit-to-notify window; this ADR neither requires nor assumes that future choice.
- The exact Rust abstraction used to pass a pool or transaction executor through permission and
  store reads. The implementation must satisfy the invariants, but this ADR does not choose a trait,
  generic signature, or wrapper type.
- Performance budgets for the additional guard reads and the observability threshold for rollback
  frequency. Measurements may establish those without changing the decision.
- Authorization atomicity for non-HTTP entry points, including collaboration/WebSocket paths. A
  later audit may extend the rule through a separate decision if their mutation model requires it.

### Read-only paths

Read-only HTTP paths remain at `READ COMMITTED` and do not re-verify. A revocation can commit after
authorization resolves and before a later content `SELECT`, so one already in-flight response may
still return data under the earlier authorization. Revocation is not retroactive, the request made
no persistent change, and the next request observes the revocation; this ADR explicitly accepts
that bounded read staleness. Read paths remain in the handler inventory so they cannot be mistaken
for unclassified mutations.

## Consequences and risks

Positive: the demonstrated move cannot silently evade ADR 0001 invariant 6; every HTTP mutation
receives the explicit re-verification rule; accepted-ADR invariant paths also receive isolation
that closes their phantom window; clients keep the refusal shapes they already understand in all
non-exhausted executions; and no source-notebook row becomes a global serialization point.

Negative: guard logic and relevant store reads require a real refactor so they can execute twice
and against a transaction rather than only `Store`'s pool. Guarded mutations add queries and may
hold transactions open longer. The eight enumerated handlers incur serializable-dependency
tracking and may replay work. Review must detect accidental pool reads, partial retries, external
effects before commit, and invariant writers left at `READ COMMITTED`.

Residual risks and non-guarantees:

- The demonstrated race has low exploitability because the relevant insert must land in a
  millisecond-scale window and timing is controllable only through retries.
- Re-verification-only paths retain the explicitly accepted interval between the final guard
  statement's snapshot and commit. This decision guarantees closure only for the enumerated
  serializable invariant set, not freedom from every application-level race or deadlock.
- A serialization failure is retried only for SQLSTATE `40001`. Exhausting three attempts produces
  a generic HTTP `503`; it is observable concurrency fallout, but never a partial commit or a false
  policy refusal. PostgreSQL errors unrelated to serialization retain their existing handling.
- Notices occur after commit. If the process dies between commit and notification, the notice is
  lost. This ADR accepts that commit-to-notify window, but not silent failure: ADR 0001
  verification-plan row 14 requires that "A revocation whose notice fails to send still commits,
  and the failure is recorded," with its missing coverage being implemented under
  [keeplin-srv#123](https://github.com/jsunyermias/keeplin-srv/issues/123). If #75 later selects a
  durable outbox for the separate journal-to-projection path, that machinery could close this
  window too; it is a possible future reuse, not ownership of the notice requirement.
- A mutating handler omitted from the handler-and-authorization-input inventory, a re-verification
  read that escapes to the pool, or a writer omitted from an invariant's serializable participant
  set can preserve the defect. Mechanical inventory and interleaving tests are therefore required.
- The added query and transaction duration costs are not measured by this ADR.

Observability should distinguish ordinary successful commits, guard-changed rollbacks,
serialization retries, exhausted retries, and notice failures in server-side telemetry without
changing an ordinary guard-refusal response. The exact metric or log schema is implementation
detail, but tests must not treat observability as the correctness mechanism.

## Compatibility, migration, and rollback

**Wire and persistent-format compatibility: not applicable.** The decision changes server-local
transaction boundaries and function signatures. It adds no REST field or status, no database
schema, no `keeplin-core` type, no collaboration message, and no protocol or format version.

**REST compatibility.** A request that loses authorization or becomes subject to a policy refusal
during its transaction can now receive the same existing refusal it would receive if started after
that state change. For the move guard this is HTTP `403` with the existing `MoveBlocked` JSON shape.
It is deliberately not `409 Conflict`, and no client serialization-retry contract is added. The
server absorbs up to two retries. Exhaustion after three total attempts adds a generic HTTP `503`
transient-failure case; this is the explicit exception to the otherwise unchanged refusal surface.

**Data migration and rollout.** No migration or cross-repository rollout ordering is required.
The server change must land atomically with the refactored executor-aware reads and tests; a partial
implementation that transaction-wraps mutations while re-verifying through the pool does not
satisfy this decision. Nor does an implementation that makes the move serializable while a
participating share or ownership writer remains at `READ COMMITTED`.

**Rollback.** No persisted representation changes, so a code rollback needs no data recovery.
Rollback reopens the time-of-check/time-of-use windows and must therefore be treated as restoring a
known violation risk, not as a safe steady state. Partially upgraded fleets may differ only in
whether the race is refused; their wire and stored data remain compatible.

## Verification plan

| # | Evidence | Kind | What fails if the decision is violated |
|---|---|---|---|
| 1 | A deterministic `sqlx::test`, using a test-only injection seam inside `update_note`, pauses after the early `inherited_note_principals(N)` result, inserts and commits Carol's notebook share, resumes the handler, and asserts HTTP `403`, the exact `MoveBlocked` principals, and unchanged `notes.notebook_id` | negative + failure injection; fails today | Fails if the second evaluation uses the stale first guard, derives the source after mutation, commits the move, or changes the refusal shape |
| 2 | The same move without the concurrent insert succeeds, commits the new notebook location, and sends any owed notices only after commit | positive | Fails if re-verification rejects stable authorization or notice ordering moves into the transaction |
| 3 | An interleaving-test harness is parameterized over every mutation-authorizing entry in the row-5 inventory; for each feasible guard-state transition it changes an authorization input after the early guard and asserts either the endpoint's ordinary refusal or, for a serializable conflict, successful server replay against the new state | negative + failure injection | Fails if testing samples only one unnamed note or notebook endpoint, or if a mutation family escapes the general rule |
| 4 | For `update_note`, one serializable transaction reads no Carol share and proposes the move while another serializable `create_notebook_share` inserts Carol; controlled scheduling proves they cannot both commit the stale decision | concurrency, phantom | Fails if either participant is `READ COMMITTED`, if row-only locks are mistaken for phantom protection, or if the silent-loss execution can commit |
| 5 | A source inventory test or mechanically maintained table accounts for every HTTP mutating handler and lists each authorization input (resolver, authenticated identity, credential, ownership query, quota, or handler-specific policy), while separately marking read-only handlers; it fails when an unclassified handler or input is added. The current audit must include resolver-free mutations such as `put_resource_data`, `change_password`, and `delete_account` | structural | Fails if the audit remains keyed only to the 17 resolver references or misses a handler authorized solely by a handler-specific guard |
| 6 | A structural assertion enumerates exactly the eight accepted-ADR invariant handlers named by this decision and requires every invariant participant to start `SERIALIZABLE` | structural | Fails if an accepted-ADR path or a racing writer is silently left at weaker isolation, or if repository-wide serializable scope is implied instead of enumerated |
| 7 | Executor-focused tests make every store read used by both resolvers, handler-specific guards, and the move guard observe transaction-local state | integration | Fails if re-verification silently reads through `Store`'s pool |
| 8 | Re-verification that finds Carol controlled and losing access returns HTTP `403` with byte-equivalent `MoveBlocked` JSON to an ordinary pre-mutation refusal; no `409` path is accepted | compatibility | Fails if a real guard refusal leaks through a concurrency-specific response or omits principals |
| 9 | Failure injection immediately after the mutation statement and before commit leaves all affected rows unchanged after rollback | recovery | Fails if an internal error can partially commit the mutation |
| 10 | Inject SQLSTATE `40001` on the first serializable attempt and assert that the server rolls it back, re-runs the complete guard and mutation, commits once, and returns the ordinary endpoint response | failure injection | Fails if a serialization failure reaches the client, if retry resumes mid-transaction, or if work commits twice |
| 11 | Inject SQLSTATE `40001` twice, then allow the third attempt to succeed; assert exactly three attempts, one commit, and no notice or other external effect before that commit | failure injection, retry bound | Fails if the server provides fewer than two retries, exceeds the three-attempt bound, or repeats an external effect |
| 12 | Inject SQLSTATE `40001` on all three attempts; assert exactly three aborted attempts, no database mutation or notice, generic HTTP `503`, and an exhausted-retry telemetry record | failure injection, exhaustion | Fails if exhaustion is hidden as a guard refusal, loops without bound, partially commits, emits an external effect, or is not observable |
| 13 | A notice send failure after commit leaves the authorized mutation committed and emits ADR 0001's required runtime `tracing::warn!` record with the affected resource and principal | failure injection | Fails if notice delivery enters the transaction, rolls back the operation, or the failure is silently discarded without the owned log destination |
| 14 | A process-stop test at the post-commit/pre-notify boundary documents that the notice can be lost while the mutation remains committed | operational, known limit | Fails if documentation or tests falsely claim durable notice intent or delivery; it does not require a queue |
| 15 | A read-path interleaving authorizes `get_note`, revokes access before body materialization, and records that the in-flight response may complete while the next request is forbidden | known-limit regression | Fails if implementation or documentation claims read revocation is retroactive or that read staleness was closed |
| 16 | `./scripts/check-docs.sh` passes with every changed source companion and project document synchronized | documentation | Fails if implementation and documentation diverge |
| 17 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` pass | repository | Fails on behavioral regressions, warnings, or formatting drift detectable by the repository checks |

No cross-repository, schema-migration, or format-recovery test is required because this decision
changes none of those surfaces. Rows 9, 12, 14, and 15 cover transactional recovery, retry
exhaustion, and the explicitly retained operational limits in proportion to this decision's risk.

## Equivalent decision in the other repository

Not required. `keeplin` has no `keeplin-srv` HTTP layer and no PostgreSQL store, and this decision
changes no `keeplin-core` type, wire message, format constant, or protocol version. There is no
immutable dependency-pin implication, paired PR, or cross-repository compatibility test. If a
future decision extends authorization atomicity into a shared protocol or client/daemon persistence
surface, that would be a new cross-repository ADR canonical in `keeplin`, not an amendment to this
server-local record.
