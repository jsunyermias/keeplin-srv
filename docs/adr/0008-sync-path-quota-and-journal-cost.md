# 0008 — Sync-path quota and journal cost

- Status: proposed
- Date: 2026-08-10
- Decision owners: maintainer of `jsunyermias/keeplin-srv`
- Scope: keeplin-srv
- Issue: [keeplin-srv#145](https://github.com/jsunyermias/keeplin-srv/issues/145)
- Acceptance PR: none — proposed
- Supersedes: keeplin-srv ADR 0003 in part (only its property that each transaction acquires at
  most one advisory lock)
- Superseded by: none

## Context and problem

`Verified at`: the working tree on branch `claude/c1-issue-orchestrator-snxq7f`, with
`keeplin-core` pinned to `3d195f6da65a000b1005d2d432bf67eaafd9077c`.

Accepted keeplin-srv ADR 0003 requires `max_user_storage_bytes` enforcement at every path that
creates a counted blob, but deliberately leaves the synchronization path's refusal representation
undecided. Rejected keeplin-srv ADR 0004 preserves the analysis of that path. Accepted keeplin-srv
ADR 0006 then supplies the durable projection lifecycle in which the answer must live.

The production path for `Change::ResourceCreate` is `projection.rs` calling
`Store::apply_resource_create`. That function opens the projection transaction, excludes concurrent
projection of the same resource, applies metadata and bytes, and commits them together.
`Store::upsert_resource_meta` has no production caller; its callers are tests. A quota decision
placed there would therefore govern no live synchronization write.

Only `apply_resource_create` can satisfy keeplin-srv ADR 0003's ordering: the quota lock precedes
the deciding read, and that read and the write it authorizes share one transaction. Keeplin-srv ADR
0006 part seven already records that a check in the earlier journal transaction cannot satisfy this
property because projection writes later in another transaction.

That placement exposes a collision between accepted decisions. `apply_resource_create` already
takes `AdvisoryLockDomain::ResourceProjection`, keyed by `resource.id`. Adding the per-user
`BlobQuota` lock makes two advisory locks in one transaction, while keeplin-srv ADR 0003 invariant
5 permits at most one. This ADR retains both protections and replaces only that property with a
strict total acquisition order.

The journal creates a separate capacity problem. `changes.payload` is `JSONB` and
`Change::ResourceCreate { data: Option<Vec<u8>> }` serializes bytes as a JSON array of decimal
integers, on the order of four times the blob. No quota counts it. `prune_delivered_changes` deletes
only below the minimum cursor returned by the user's joined `device_cursors` rows and when no
unfinished projection job references the change. A device cursor that stops advancing pins later
journal rows; when the join returns no cursor row, `COALESCE` makes the minimum zero and prevents
pruning. The residue is **one** journal
copy: keeplin-srv ADR 0006 requires a job to reference its journal row and forbids copying payload
into the job.

## Forces and requirements

- Keeplin-srv ADR 0003's quota enforcement and serialization properties must govern synchronized
  resource projection without moving the decision outside the writing transaction.
- Per-user quota serialization and per-resource projection exclusion are both required.
- Deadlock freedom must be mechanically enforced after the one-lock structural guarantee is given
  up.
- A permanent quota refusal must fit keeplin-srv ADR 0006's existing outstanding/dead-lettered
  lifecycle and enumerated failure classification.
- The measurement is persisted net counted bytes, including replacement and resurrection, never
  bytes on the wire or client-declared metadata.
- A refusal must not become an indefinitely outstanding job that pins its own amplified input.
- Journal storage of a blob must cease costing a multiple of that blob by construction.
- The shared `Change` serde form and every byte sent on the wire remain unchanged. There is no
  coordinated `keeplin` change and no `PROTOCOL_VERSION` bump.
- Existing HTTP quota behavior remains byte-identical: `507 QuotaExceeded`, with no quota-specific
  serializable retry or `503`.

## Threat model

**Assets.** Server capacity, the truth of `max_user_storage_bytes`, projection availability, and
the absence of database deadlocks.

**Trust boundary.** An authenticated user's synchronized `Change` values, interacting with the
server-local journal, projection worker and PostgreSQL advisory locks.

**Adversary.** An authenticated user can choose blob content and size, create concurrent resources,
replace or resurrect resources, and leave one of their devices' cursor stalled. They need no timing
precision to amplify JSON journal storage; concurrency can exercise any inconsistent lock order.

**Capabilities and consequence.** Without the quota check the user grows projected blobs without
the declared bound. Without compact journal storage the user also creates roughly four times the
blob in uncounted, potentially unprunable storage. An opposite lock order can deadlock two otherwise
valid projections and consume worker capacity.

**Out of scope.** Aggregate server capacity, per-connection backpressure and batch bounds; client
acknowledgement, owned by keeplin#150; and changing the canonical shared `Change` representation.

## Options considered

### Option 1 — Check quota in the journal transaction

Rejected. The journal transaction does not perform the resource write. It cannot make the deciding
read and authorized write one transaction and therefore violates keeplin-srv ADR 0003 invariants 2
and 3, as keeplin-srv ADR 0006 part seven already records.

### Option 2 — Check in `Store::apply_resource_create`, but remove one lock

Rejected. Removing `BlobQuota` restores write skew between resources belonging to one user;
removing `ResourceProjection` permits concurrent appliers to interleave metadata and bytes for one
resource. The locks protect different invariants and neither substitutes for the other.

### Option 3 — Keep both locks under one total order

Adopted. Every transaction that needs both acquires the per-user `BlobQuota` domain first and the
per-resource `ResourceProjection` domain second. A shared acquisition API and structural inventory
make reverse or skipped ordering a failing condition.

### Option 4 — Admit over-quota work and complete it when space frees

Rejected after re-evaluation on merit, not inherited from keeplin-srv ADR 0004. Deferral leaves the
job outstanding. Keeplin-srv ADR 0006 invariant 7 prevents pruning a journal row referenced by an
unfinished job. The over-quota change therefore pins its own blob-bearing journal row indefinitely.
Even after journal compaction, this converts a bounded refusal into an unbounded, unprunable cost
that grows precisely while the user is already over quota. The gentlest-looking option is
self-defeating under the retention interlock.

### Option 5 — Dead-letter a quota refusal

Adopted with a new enumerated permanent classification, `quota_exceeded`. It uses keeplin-srv ADR
0006's existing terminal state: the change is not projected and its job is dead-lettered. There is
no third outcome and no new terminal state.

### Option 6 — Keep the JSON byte array and count the journal copy against quota

Rejected. It would make one logical blob consume approximately five times its size from a limit
whose existing meaning is live projected blob bytes, and it would preserve unnecessary physical
amplification. The journal copy is excluded by construction instead.

### Option 7 — Base64-encode `data` inside `Change`

Considered and declined. `Change`'s serde form is a shared wire surface owned by `keeplin-core`.
Changing it is a cross-repository protocol decision requiring coordinated compatibility work and
possibly a `PROTOCOL_VERSION` bump. The maintainer chose a server-local solution; the serialized
wire `Change` remains identical.

### Option 8 — Compact the journal only at the database boundary

Two server-local shapes qualify: a compact physical encoding of the complete payload, or storing
blob bytes by reference. Adopt the latter: extract `ResourceCreate.data` into a `BYTEA` side table
keyed by the journal change, store the remaining JSONB payload without the byte array, and
reconstruct the canonical `Change` before projection and delivery. A foreign-key cascade gives the
side row the journal row's lifecycle. It is chosen because `octet_length` makes its near-one-copy
cost direct to test and operate; an opaque compressed payload would make amplification depend on
content and codec behavior. This is a physical server encoding only, not a wire encoding.

## Decision and justification

**Enforce `max_user_storage_bytes` inside `Store::apply_resource_create`.** The transaction first
takes the per-user `BlobQuota` lock, then the per-resource `ResourceProjection` lock. It reads the
current resource/blob state and the user's counted total on that transaction, computes the net
delta, and either applies metadata and bytes or returns the permanent `quota_exceeded`
classification. The worker records that classification as the existing dead-letter state.

**Replace the amplified journal representation with journal-by-reference.** The canonical incoming
and outgoing `Change` is unchanged. At journal insertion the server extracts only the optional blob
bytes to a `BYTEA` side row keyed to the change; before projection, fan-out or backlog delivery it
reconstructs the same canonical value. Projection job rows continue to reference, never copy, the
journal input.

**Partial supersession is deliberately narrow.** This ADR supersedes only keeplin-srv ADR 0003's
property that each transaction acquires at most one advisory lock. Everything else in
keeplin-srv ADR 0003 stands: enforcement at every path that creates a counted object; lock acquisition before
the deciding read; the read and authorized write on that same transaction; per-user, per-quota
keying; advisory-lock domain separation; byte-identical HTTP `507`; and no serializable retry or
`503` on quota paths.

The invariants are:

1. Every synchronized `Change::ResourceCreate` is quota-decided inside the
   `Store::apply_resource_create` projection transaction; the quota lock precedes the deciding
   reads, and those reads and every authorized metadata/blob write use that transaction.
2. Advisory lock domains have one canonical total acquisition order. For this transaction it is
   `BlobQuota(user_id)` **before** `ResourceProjection(resource_id)`. A transaction may acquire any
   subset of the domains, in order, but never acquire an earlier domain after a later one; every
   future third domain is assigned one position before use. Within one transaction there is at most
   one advisory lock from each domain. Introducing a path that needs two keys in one domain requires
   a superseding decision with a canonical intra-domain key order; it is not pre-authorized here.
   Every advisory lock is acquired through the shared helper, whose ordinal and one-key-per-domain
   checks run unconditionally, including in production; the structural inventory rejects every raw
   `pg_advisory_xact_lock` outside that helper and every transaction shape absent from the inventory.
   In every transaction that uses an advisory domain, all required advisory locks strictly precede
   acquisition of any row lock. Its row-lock/write set is partitioned by the advisory keys it holds:
   it may lock or write only rows whose ownership/resource identity maps to one of those held keys,
   and every competing transaction that can lock or write such a row must first hold the same
   corresponding advisory key. The transaction inventory records and checks both the advisory and
   row-lock/write sets; a cross-user or cross-resource shared row requires a new ordered domain or a
   superseding proof before use.
3. A quota-bearing projection commits only when its net change leaves the user's counted live blob
   bytes at or below `max_user_storage_bytes`; same-user blob-quota writes serialize, while different
   users do not except for documented 64-bit key collisions.
4. The measured quantity is the **net change in counted bytes**, never bytes on the wire. For a new
   blob it is its stored length; for replacement of the same `resource_id` it is
   `new_length - old_length`, matching `put_resource_blob`'s conflict update.
5. Resurrection is measured even when no bytes cross the wire. If a winning incoming create clears
   `deleted_at` on a resource whose tombstone retained a blob, its delta includes
   `+octet_length(retained_blob)` when `data` is `None`; replacing that retained blob uses the net
   live-state transition.
6. `Resource.size` is client-declared metadata and is never used in quota measurement.
7. A synchronized change that exceeds the quota is not projected and its job becomes dead-lettered
   with the new enumerated permanent classification `quota_exceeded`. It is not left outstanding
   and no new terminal state is introduced.
8. Fan-out and journal cursor semantics do not pretend projection refusal is delivery refusal. By
   projection time the change has already been fanned out; no mechanism retracts it. Device cursors
   advance on the journal, not projection, so a dead-lettered quota change does not stall cursors.
9. The journal copy is **excluded by construction**, not charged to `max_user_storage_bytes`: blob
   bytes are stored once in a server-local `BYTEA` side row rather than as a decimal JSON array, so
   the journal's blob-bearing physical payload no longer costs a multiple of the blob. Projection
   jobs contain no payload copy.
10. `max_user_storage_bytes` bounds the sum of live projected `resource_blobs` for a user. Actual
    server-held storage also includes one compact journal copy while retained, tombstoned blobs,
    row/index/MVCC overhead, projection-job metadata, and other server data; those are not bounded by
    this per-user setting. Tests and operator documentation state both quantities without conflating
    them.
11. The canonical `Change` serialized on WebSocket fan-out and backlog delivery is byte-identical to
    the value accepted at ingress. Journal compaction is server-local; no shared wire or
    `keeplin-core` format and no `PROTOCOL_VERSION` changes.
12. HTTP quota refusal remains byte-identical `507 QuotaExceeded`; the synchronization path emits no
    HTTP status. Client-visible acknowledgement or refusal is keeplin#150's surface and is not
    invented here. Until it exists, the origin device learns of `quota_exceeded` only out of band.

### Why the new lock rule prevents cycles

For advisory-only waits, assume a deadlock cycle exists. Along each wait edge, a transaction holds
one advisory lock while requesting another. Invariant 2 requires the requested domain to be strictly
later than every advisory domain already held, and permits only one key in each domain. Following
the cycle would therefore produce a strict domain increase at every edge and eventually require the
starting domain to be later than itself, a contradiction.

Mixed advisory/row-lock waits do not escape the proof. Advisory locks precede all row locks, so a
transaction holding a row lock never waits for an advisory lock. A row wait can only be on a row
partition covered by a key the waiter already holds; every competing writer of that partition had
to acquire that same advisory key first, so it cannot hold the row while the waiter holds the
partition key. Row-only cycles within these transactions are excluded by the same partition rule.
The guarantee is weaker structurally than keeplin-srv ADR 0003's one-lock rule: one reverse
acquisition, second same-domain key, row-before-advisory acquisition, or uncovered shared row is
enough to invalidate the proof. The unconditional shared API and source inventory must reject all
four mechanically; prose is not enforcement.

### Costs, stated rather than implied

**Structural safety is reduced.** Keeplin-srv ADR 0003 made deadlock freedom checkable by counting
locks in a transaction: at most one implies no cycle. This decision requires reasoning about a
global order across every path that takes more than one lock. The total-order proof is sound, but
the implementation condition is easier to violate and must be mechanically guarded.

**Projection refusal follows fan-out.** Keeplin-srv ADR 0006 records that fan-out precedes durable
projection. Peer devices can therefore hold a resource the server refuses to project, and nothing
here retracts it. Until keeplin#150 defines client-visible acknowledgement, the origin receives no
in-band refusal. This is a convergence cost, not an omitted implementation detail.

**Refusal does not remove the change from delivery.** Backlog delivery filters by journal cursor,
not projection-job state, so every present or future device whose cursor backlogs past a
quota-refused change remains entitled to receive its canonical payload. The journal row and its
blob side row are therefore load-bearing delivery data even after dead-lettering: operator response
may re-drive the job, but must not delete, detach or filter that payload merely to reclaim refused
blob storage while ordinary cursor/retention semantics still retain it.

**Journal-by-reference adds schema and reconstruction machinery.** Every journal read that feeds
projection, fan-out or backlog delivery must reattach bytes exactly. After a row is explicitly
marked compact, a missing or mismatched side row for a payload that declares blob presence is a
corruption condition; an unmarked legacy inline row is valid during migration. Migrations must
preserve existing rows, and cascade behavior must be verified. The side table removes
multiplicative encoding cost, not PostgreSQL row, index, WAL, TOAST or MVCC overhead.

**Dead letters remain operational state.** A quota-refused change is visible and re-drivable under
keeplin-srv ADR 0006, but blind re-drive while the user remains over quota will reach the same
classification. Operators need the reason and current quota state.

**Two locks extend the transaction's wait surface.** A projection may wait first behind the user's
other blob writes and then behind work on the same resource. Timeouts remain internal failures when
no quota decision was reached; they are not `quota_exceeded`.

### Not decided

- The client-visible ACK/NACK shape or client reconciliation after peers have already received a
  refused change; keeplin#150 owns it.
- Aggregate server capacity, per-user journal-retention limits, batch limits and backpressure.
- Whether a later accepted decision replaces the side-table representation with another
  server-local encoding that preserves invariants 9 and 11 and demonstrates equal or lower measured
  server-held bytes.
- Operator policy for re-driving `quota_exceeded` dead letters after a user frees space.

## Consequences and risks

- The previously unguarded synchronization path can no longer grow live projected blob bytes beyond
  `max_user_storage_bytes` sequentially or concurrently.
- A synchronized over-quota change is durable in the journal and visible as a dead letter, while
  peer devices may already contain it. Cursors continue advancing.
- A stalled device can still retain the journal indefinitely, but the retained blob copy is near
  its binary size rather than a decimal-array multiple. This ADR does not create a retention bound.
- Two-lock transactions are safe only while the global order is complete and mechanically enforced.
- Server-local storage readers and writers gain a representation boundary whose byte-equivalence is
  compatibility-critical even though the wire contract does not move.

## Compatibility, migration, and rollback

A forward-only, idempotent migration creates the journal-blob side table with a foreign key to the
journal change identity and cascade deletion, plus an unambiguous per-row representation marker so
a reader can distinguish legacy inline bytes from a compact row. Existing migrations are not
edited.

Deployment is expand/backfill/contract. First deploy a dual reader that accepts both representations:
for an unmarked legacy row it reads canonical `ResourceCreate.data` inline; for a marked compact row
it reconstructs `Some(bytes)` only from the required side row, reconstructs `None` when the compact
marker records no blob, and treats a missing or mismatched required side row as corruption rather
than silently producing `None`. That reader is used by projection, immediate fan-out and backlog.
Next enable a writer that atomically inserts the side row and compact JSONB/marker in the journal
transaction. Then backfill each legacy `ResourceCreate` atomically by writing and verifying its
`BYTEA` side row before marking the row compact and removing its inline decimal array. Mixed-format
operation remains supported until every row is verified compact and the rollback window closes;
only then may a later migration remove the legacy read path.

There is no wire or shared format migration. `keeplin-core` remains pinned to
`3d195f6da65a000b1005d2d432bf67eaafd9077c`; `PROTOCOL_VERSION` does not move. Old and new clients
receive the same serialized `Change`.

The side table is retained throughout the rollback window. Rollback first disables compact writes,
proves every compacted journal row can be reconstructed, restores and verifies the canonical JSONB
byte array from the side table, clears the compact marker, and only then removes use of the side
representation; the side table may be removed only after no compact row remains. Removing quota
enforcement returns an unbounded synchronization bypass; removing either lock returns the race its
domain prevents. Reversing the acquisition order is never a rollback because it violates the
deadlock invariant.

## Verification plan

C2 runs every row against real PostgreSQL and pastes the real output. This proposed ADR runs none of
them. Every invariant has named evidence and a mutation that must make it red.

| Invariant | Evidence against real PostgreSQL | Mutation that must redden it |
|---|---|---|
| 1 | `sync_blob_quota_check_and_write_share_projection_transaction` forces an error after the deciding read and proves no resource/blob write commits | Move the quota read to the journal transaction or pool |
| 2 | `advisory_lock_order_inventory_is_complete` maps the wait-for graph of every transaction touching an advisory-protected table, inventories its advisory and row-lock/write sets, and proves: every advisory call routes exclusively through the shared helper; the helper unconditionally rejects decreasing ordinals and a second key in one domain; every advisory lock precedes every `FOR UPDATE`/upsert/write lock; and every row set is partitioned by the held advisory keys. `mixed_advisory_and_row_lock_order_is_deadlock_free` drives two real PostgreSQL transactions through the opposite mixed-lock schedule and proves one completes rather than cycles | Swap `BlobQuota` and `ResourceProjection`; acquire two resource keys; issue a row lock before the helper; touch a cross-user/shared row without its domain; or add a raw `pg_advisory_xact_lock` call outside the helper. Each mutation must make the inventory red, and the mixed-order mutations must redden the probe |
| 3 | `concurrent_sync_blob_quota_is_serialized` uses a forced rendezvous with two resources for one user, each fitting alone but not together; exactly one projects and the total stays bounded. A paired different-user case reaches the rendezvous concurrently | Remove `BlobQuota` or coarsen its key to a constant |
| 4 | `sync_blob_replacement_uses_net_delta` replaces a blob at the limit with larger and smaller values and asserts `new - old` admission | Charge `new_length` without subtracting the old blob |
| 5 | `sync_metadata_only_resurrection_counts_retained_blob` resurrects a tombstoned retained blob with `data: None` and refuses when it does not fit | Treat `None` as zero delta or purge the retained length from the calculation |
| 6 | `sync_blob_quota_ignores_declared_resource_size` sends a tiny declared size with oversized data and is refused | Substitute `Resource.size` for `octet_length` |
| 7 | `sync_blob_quota_exceeded_is_permanent_dead_letter` exceeds `max_user_storage_bytes` through the previously unguarded sync path and asserts dead-letter reason `quota_exceeded`, not outstanding | Classify quota as transient or leave the job outstanding |
| 8 | `quota_dead_letter_advances_cursor_without_retraction` observes fan-out before refusal, then advances and reconnects a device without cursor stall or a retraction frame | Couple cursor advancement to projection success or add an unauthorized retraction |
| 9 | `journal_blob_is_single_compact_copy` synchronizes known incompressible bytes and asserts one `BYTEA` copy, no JSON decimal array and no payload in the job row | Store `data` in JSONB too or copy it into `projection_jobs` |
| 10 | `stalled_cursor_server_held_bytes_are_bounded_by_encoding` synchronizes a resource while another device's cursor stops, measures per-user server-held live blob plus journal blob bytes, and asserts separately that quota-counted bytes are at most `payload_bytes + epsilon` and their sum is at most `2 * payload_bytes + epsilon`, where the test fixes `epsilon` before measurement to the non-payload row/representation overhead | Restore the JSON array, choose epsilon after measurement, or report only quota-counted bytes. This covers issue criteria 7 and 9 and is red on the pre-fix tree |
| 11 | `journal_round_trip_preserves_canonical_change_bytes` covers every `Change` variant and compares canonical serialized bytes before storage and after reconstruction for projection, fan-out and backlog. It runs against a partially migrated table containing legacy inline rows and marked compact side-table rows, including `Some(empty)`, `Some(bytes)` and `None`, and makes a missing required side row fail as corruption | Omit, reorder or alter `ResourceCreate.data`; treat a missing compact side row as `None`; or remove either half of the dual read during mixed-format rollout |
| 12 | Existing HTTP quota compatibility test compares the contended and uncontended `507` status/body byte for byte; sync test asserts no HTTP surface. A protocol inventory pins keeplin#150 as the only future ACK owner | Introduce a sync HTTP response, change the body, or add an ad hoc ACK frame |

The existing `quota_write_inventory_is_complete` in `crates/keeplin-srv/tests/quotas.rs` currently
asserts both `!sync.contains("lock_blob_quota")` and
`!projection.contains("lock_blob_quota")`. Implementing this decision must invert **both** guards:
the synchronization surface must be tied structurally to the projection enforcement point, and the
projection must contain the quota lock. A reviewer seeing two flipped assertions without this ADR
would reasonably read them as a weakened test; here they are explicit acceptance evidence for issue
criterion 4. Mutating either guard back to its current negative form must make the implemented
inventory fail.

Repository completion also requires `./scripts/check-docs.sh`, `cargo test --workspace` against
PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, and
`cargo fmt --all --check`. This prose-only proposal does not claim those checks passed.

## Equivalent decision in the other repository

None. Quota enforcement, projection transactions, PostgreSQL lock ordering, job classification and
journal physical encoding are local to `keeplin-srv`. The canonical `Change` serde form remains
identical on the wire, the pinned `keeplin-core` revision does not move, and `PROTOCOL_VERSION` does
not change. Therefore no canonical or companion ADR and no coordinated PR is required in `keeplin`.
