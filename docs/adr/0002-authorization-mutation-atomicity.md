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

`Verified at`: `keeplin-srv@161a210`.

The HTTP layer contains 17 references to `resolve_note_access` or
`resolve_notebook_access`, and contains no call to `begin()` or `.transaction(`. The mutating
handlers among those call sites therefore read authorization state through the pool and later
mutate through a separate pool operation. Nothing makes the authorization fact and the mutation it
authorizes one atomic decision. This is an HTTP permission-surface pattern, not a property unique
to note moves.

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
  holds in the transaction that commits the mutation.
- The rule covers all 17 current HTTP resolver references as an audit boundary: every call site
  must be classified, and every mutation authorized by one must adopt the transactional rule.
- A changed guard outcome must leave no mutation committed.
- A move newly blocked by a controlled principal must produce the ordinary ADR 0001 refusal: HTTP
  `403` with the same `MoveBlocked` body and the same named and counted principals.
- A client must not need a concurrency-specific retry path and must not learn whether an ordinary
  refusal arose during re-verification.
- The mechanism must avoid turning an active shared notebook into a serialization bottleneck.
- The mechanism must not change the failure contract of every client by making ordinary endpoints
  retry serialization failures.
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
equivalent already discloses. No concurrency-specific status or body is added. Notices have the
same superficial shape as the critical journal-to-projection defect tracked by
[keeplin-srv#75](https://github.com/jsunyermias/keeplin-srv/issues/75): commit, then attempt an
out-of-transaction effect whose failure currently dies in a `tracing::warn!`. The difference is in
kind, not importance. A projection is another write to the same database and can be included in the
transaction, so leaving it outside is a defect. A notice is an outbound message to a third party;
no database transaction can roll back an email already sent, so its outside-the-transaction
placement is a fact about the effect rather than a choice made by this ADR. This ADR accepts the
commit-to-notify window: process failure after commit but before notification can lose the notice.
It does not accept silent discard. Accepted ADR 0001 verification-plan row 14 requires that
"A revocation whose notice fails to send still commits, and the failure is recorded"; that
unimplemented coverage is being added under
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

### Option 3 — Use `SERIALIZABLE` transactions with retry

Run guarded endpoints at PostgreSQL `SERIALIZABLE` isolation and retry serialization failures.

Benefits: asks the database to reject non-serializable interleavings as a general mechanism.

Costs and failure modes: any affected endpoint can fail and require retrying, changing every
client's contract rather than only the server implementation. Safe bounded retry policy,
idempotency, and exhaustion behavior become new operational concerns. Evidence that would change
the assessment would be an already-supported, uniform client retry contract and proven idempotency
for every mutation; neither is established here.

### Option 4 — Re-verify inside the mutation transaction

Perform the existing early guard for normal refusal, open a transaction, perform the mutation,
re-evaluate the same authorization and policy guard using reads on that transaction, and commit
only if the result is unchanged. If it changed, return the guard's real refusal and roll back.

Benefits: closes the authorization/mutation window with a server-local contract, preserves the
endpoint's existing refusal surface, and avoids a notebook-wide pessimistic lock or a new client
retry protocol.

Costs and failure modes: guard logic executes twice. `resolve_note_access`,
`resolve_notebook_access`, and their store reads must be refactored to run against either the pool
or a transaction without accidentally mixing executors. Handler-specific guard reads, the mutation,
and re-verification must all use the same transaction. The extra reads add database work to every
guarded mutation. A missed store read that still uses the pool would recreate the defect despite
the appearance of a transaction. Operational burden is limited to the server refactor, its tests,
and monitoring for changed latency and rollback frequency.

Evidence that would change the assessment would be a proof that this ordering cannot observe every
relevant concurrent change under the selected PostgreSQL isolation behavior. That evidence would
require a follow-up decision rather than silently substituting another mechanism.

## Decision and justification

> This ADR is `proposed`. It records the maintainer's selected recommendation but does not authorize
> implementation. Only the maintainer may accept or reject it.

**Proposed decision: adopt Option 4 as the general rule for the whole server HTTP permission
surface.** All 17 current resolver references must be audited. Whenever a resolver result or a
handler-specific authorization guard authorizes a mutation, the server performs the mutation and
then re-verifies the complete guard inside the same transaction, committing only when the outcome
still authorizes that mutation. A changed outcome rolls the transaction back.

The proposed invariants are:

1. No HTTP mutation commits solely on an authorization or policy fact read outside its transaction.
2. The mutation and re-verification read one transaction-local database history; every store read
   used by the re-verification executes on that transaction.
3. A changed outcome rolls back and returns exactly the refusal the current state requires. For the
   demonstrated move, that is the same HTTP `403` and `MoveBlocked` shape, naming the same
   principals, as an ordinary ADR 0001 guard refusal.
4. A client cannot distinguish a re-verification refusal from the equivalent ordinary refusal.
   `409 Conflict` is not introduced.
5. Notices remain outside the transaction and are attempted after commit. Notice failure never
   rolls back the operation that owed it.

This option directly enforces the accepted invariant at risk, applies one reviewable rule to all
seventeen call sites, confines compatibility work to the server, and avoids both the contention of
a pessimistic notebook lock and a universal serialization-retry contract.

### Not decided

- Whether this general rule should apply to read-only paths. They perform no authorized mutation,
  but all resolver references remain in the audit so their classification is explicit.
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

## Consequences and risks

Positive: the demonstrated move cannot silently evade ADR 0001 invariant 6; the other mutating
resolver call sites receive the same explicit atomicity rule; clients keep the refusal shapes they
already understand; and no source-notebook row becomes a global serialization point.

Negative: guard logic and relevant store reads require a real refactor so they can execute twice
and against a transaction rather than only `Store`'s pool. Guarded mutations add queries and may
hold transactions open longer. Review must detect accidental pool reads from within the second
guard.

Residual risks and non-guarantees:

- The demonstrated race has low exploitability because the relevant insert must land in a
  millisecond-scale window and timing is controllable only through retries.
- This decision guarantees the classified authorization relationship, not freedom from every
  application-level race or deadlock. PostgreSQL errors unrelated to a changed guard retain their
  existing handling.
- Notices occur after commit. If the process dies between commit and notification, the notice is
  lost. This ADR accepts that commit-to-notify window, but not silent failure: ADR 0001
  verification-plan row 14 requires that "A revocation whose notice fails to send still commits,
  and the failure is recorded," with its missing coverage being implemented under
  [keeplin-srv#123](https://github.com/jsunyermias/keeplin-srv/issues/123). If #75 later selects a
  durable outbox for the separate journal-to-projection path, that machinery could close this
  window too; it is a possible future reuse, not ownership of the notice requirement.
- A handler omitted from the 17-call-site audit or a re-verification read that escapes to the pool
  can preserve the defect. Mechanical inventory and interleaving tests are therefore required.
- The added query and transaction duration costs are not measured by this ADR.

Observability should distinguish ordinary successful commits, guard-changed rollbacks, and notice
failures in server-side telemetry without changing the client response. The exact metric or log
schema is implementation detail, but tests must not treat observability as the correctness
mechanism.

## Compatibility, migration, and rollback

**Wire and persistent-format compatibility: not applicable.** The decision changes server-local
transaction boundaries and function signatures. It adds no REST field or status, no database
schema, no `keeplin-core` type, no collaboration message, and no protocol or format version.

**REST compatibility.** A request that loses authorization or becomes subject to a policy refusal
during its transaction can now receive the same existing refusal it would receive if started after
that state change. For the move guard this is HTTP `403` with the existing `MoveBlocked` JSON shape.
It is deliberately not `409 Conflict`, and no client retry contract is added.

**Data migration and rollout.** No migration or cross-repository rollout ordering is required.
The server change must land atomically with the refactored executor-aware reads and tests; a partial
implementation that transaction-wraps mutations while re-verifying through the pool does not
satisfy this decision.

**Rollback.** No persisted representation changes, so a code rollback needs no data recovery.
Rollback reopens the time-of-check/time-of-use windows and must therefore be treated as restoring a
known violation risk, not as a safe steady state. Partially upgraded fleets may differ only in
whether the race is refused; their wire and stored data remain compatible.

## Verification plan

| # | Evidence | Kind | What fails if the decision is violated |
|---|---|---|---|
| 1 | A deterministic `sqlx::test` pauses `update_note` after `inherited_note_principals(N)`, inserts Carol's notebook share, resumes the move, and asserts HTTP `403`, the exact `MoveBlocked` principals, and unchanged `notes.notebook_id` | negative + failure injection; fails today | Fails if the mutation can commit on the stale first guard or if the refusal loses its existing shape |
| 2 | The same move without the concurrent insert succeeds, commits the new notebook location, and sends any owed notices only after commit | positive | Fails if re-verification rejects stable authorization or notice ordering moves into the transaction |
| 3 | A concurrent change that alters a mutating note endpoint's `resolve_note_access` result between its first guard and mutation causes rollback and the endpoint's ordinary refusal | negative + failure injection | Fails if the general rule is implemented only for the move-specific guard |
| 4 | A concurrent change that alters a mutating notebook endpoint's `resolve_notebook_access` result between its first guard and mutation causes rollback and the endpoint's ordinary refusal | negative + failure injection | Fails if notebook mutations or the second resolver family escape the rule |
| 5 | A source inventory test or mechanically maintained table accounts for all 17 current resolver references, classifies each as read-only or mutation-authorizing, and fails when an unclassified reference is added | structural | Fails if any current sibling or future call site avoids explicit review |
| 6 | Executor-focused tests make every store read used by both resolvers and the move guard observe uncommitted state in the supplied transaction | integration | Fails if re-verification silently reads through `Store`'s pool |
| 7 | Re-verification that finds Carol controlled and losing access returns HTTP `403` with byte-equivalent `MoveBlocked` JSON to an ordinary pre-mutation refusal; no `409` path is accepted | compatibility | Fails if the race leaks through a concurrency-specific response or omits principals |
| 8 | Failure injection after mutation but before re-verification leaves all affected rows unchanged after rollback | recovery | Fails if a changed guard or internal error can partially commit the mutation |
| 9 | A notice send failure after commit leaves the authorized mutation committed and records the failure | failure injection | Fails if notice delivery enters the transaction or rolls back the operation |
| 10 | A process-stop test at the post-commit/pre-notify boundary documents that the notice can be lost while the mutation remains committed | operational, known limit | Fails if documentation or tests falsely claim durable notice delivery; it does not require a queue |
| 11 | `./scripts/check-docs.sh` passes with every changed source companion and project document synchronized | documentation | Fails if implementation and documentation diverge |
| 12 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo fmt --all --check` pass | repository | Fails on behavioral regressions, warnings, or formatting drift detectable by the repository checks |

No cross-repository, schema-migration, or format-recovery test is required because this decision
changes none of those surfaces. Rows 8 and 10 cover transactional recovery and the explicitly
retained operational limit in proportion to this decision's risk.

## Equivalent decision in the other repository

Not required. `keeplin` has no `keeplin-srv` HTTP layer and no PostgreSQL store, and this decision
changes no `keeplin-core` type, wire message, format constant, or protocol version. There is no
immutable dependency-pin implication, paired PR, or cross-repository compatibility test. If a
future decision extends authorization atomicity into a shared protocol or client/daemon persistence
surface, that would be a new cross-repository ADR canonical in `keeplin`, not an amendment to this
server-local record.
