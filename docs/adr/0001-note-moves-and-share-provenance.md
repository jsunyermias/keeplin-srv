# 0001 — Note moves and the provenance of note shares

- Status: proposed
- Date: 2026-08-06
- Decision owners: maintainer of `jsunyermias/keeplin-srv`
- Scope: keeplin-srv
- Issue: [keeplin-srv#110](https://github.com/jsunyermias/keeplin-srv/issues/110)
- Acceptance PR: none yet
- Supersedes: none
- Superseded by: none

## Context and problem

`Verified at`: `keeplin-srv@f2c8025`.

A note carries two independent access sources. It has an owner (`notes.owner_id`), and it may sit
inside a notebook (`notes.notebook_id`). Grants live in two tables, `note_shares` and
`notebook_shares`, and `// md:fn resolve_note_access` in `crates/keeplin-srv/src/permissions.rs`
combines them: the owner gets `Access::owner()`, the owner of the containing notebook gets
`Access::granted(Capabilities::all())`, and anyone else gets whatever their `note_shares` row says.

### The escalation

`// md:fn update_note` in `crates/keeplin-srv/src/http.rs` authorizes a move with exactly two
checks: that the caller can write the note, and that the caller can write the destination
notebook. It never checks who owns the note, and it never consults that owner. On a real move-in
it then calls `Store::apply_notebook_shares_to_note`, which runs
`// md:fn replace_note_shares_from_notebook_tx` in `crates/keeplin-srv/src/store.rs`:

```sql
DELETE FROM note_shares WHERE note_id = $1
```

followed by an insert selecting every row of `notebook_shares` for the destination.

The consequence, entirely within the documented API:

1. Alice owns note *N* and grants Bob `WRITE`. `can_share_write()` is false for Bob.
2. Bob creates notebook *B*, which he owns.
3. Bob calls `PATCH /api/notes/N` with `{"notebook_id": B}`. Both guards pass: Bob can write the
   note by Alice's grant, and he can write *B* because he owns it.
4. Every grant Alice made is deleted and replaced by *B*'s notebook shares.
5. `// md:fn resolve_note_access` now takes the containing-notebook-owner branch and returns
   `Capabilities::all()` for Bob.

### The exact ceiling

`Capabilities::all()` is `READ | WRITE | SHARE_READ | SHARE_WRITE | MANAGE`, so Bob can grant any
subset of those to anyone: `// md:fn create_share` requires `can_share_write()` and then admits
any requested bits that are a subset of the caller's own. Alice's earlier grants are already gone.

What Bob does not obtain: `// md:impl Access > accessors` derives `can_delete` and
`can_transfer_ownership` from `is_owner`, and `Access::granted` sets `is_owner: false`. Ownership
is read from `notes.owner_id`, which the move does not touch. Alice also keeps visibility, because
`// md:impl Store > fn list_notes_for_user` admits a row on `n.owner_id = $1` independently of any
share.

The ceiling is therefore: **reshare to anyone and revoke everyone, without being able to delete or
transfer.** That is what turns a grant of "may edit" into effective control of access, and it is
what makes this critical rather than merely wrong.

### The root cause is wider than the move

The same `DELETE`-then-repopulate shape appears in `// md:fn cascade_notebook_to_notes_tx`, which
runs whenever a notebook share is created, updated or deleted. A note that legitimately sits in
Alice's notebook and that Alice also shared directly loses that direct grant the next time Alice
touches any share on the notebook. No attacker is needed and nothing reports it.

Both defects follow from one unstated modelling choice: `note_shares` is written as an
**independent grant set** by `// md:fn create_share`, and simultaneously treated as a **materialized
projection of `notebook_shares`** by the two cascade functions. Nothing reconciles the two
readings, so whichever writer runs last wins and the loser's grants are gone.

Two further observations bound the problem:

- Moving a note *out* to the Inbox is not symmetric with moving it *in*. `moved_into` is `None`
  when `notebook_id` becomes null, so no cascade runs and grants survive — asserted today by
  `// md:fn nil_notebook_id_patch_means_inbox_and_keeps_shares`.
- The relay is not a second entry point. `// md:fn note_changes_are_explicitly_non_materializing`
  records that note variants are not materialized, and `RELAY_CHANGE_CAPABILITY_UNCOVERED` states
  the same for `NoteUpdate`. `PATCH /api/notes/:id` is the only route that moves a note.

### The documentation contradiction

`crates/keeplin-srv/src/http.md` states that consent is required on both sides of a move, but the
two sides the code checks are both permissions *of the mover*. `SECURITY.md` states that the
permission surface passed two internal code audits with every finding fixed or recorded; this path
is neither, so that sentence is false as written.

## Forces and requirements

- A grantee must never acquire, through any sequence of permitted calls, a capability the granting
  owner did not confer. This is the invariant the escalation breaks.
- An owner's grants must not disappear as a side effect of an action taken by someone else, and
  must not disappear silently as a side effect of the owner's own unrelated action.
- Notebook containment must keep working as a delegation mechanism: a notebook owner legitimately
  manages the notes their notebook holds, which `// md:fn notebook_owner_can_manage_child_notes_they_do_not_own`
  asserts today and which is not a defect.
- `can_delete` and `can_transfer_ownership` stay bound to ownership. They are already correct and
  this decision must not loosen them.
- Whatever is decided must be expressible as a test that fails when the fix is reverted. Under
  `AGENTS.md` a finding blocks only if it is reified, and neither `http.md` nor `SECURITY.md` can
  serve as a verifier: they describe intent and do not fail on revert.
- Migrations are forward-only and idempotent, and a new `NOT NULL` column carries a `DEFAULT`.
- The permission model is server-local. `Capabilities` is defined only in
  `crates/keeplin-srv/src/permissions.rs`, shares appear in no collab protocol variant, and no
  `keeplin-core` type is involved.

## Threat model

**Asset.** The confidentiality and the access-control state of a note: who may read it, who may
write it, and who may extend those rights to others.

**Trust boundary.** Between the note's owner and every principal the owner grants access to. The
server is trusted; grantees are not.

**Adversary.** An authenticated user holding a legitimate, minimal grant on a victim's note —
`WRITE` with `can_share_write()` false — who may freely create notebooks they own. No stolen
credential, no protocol abuse and no race is required; the attack is a single documented `PATCH`.

**Capabilities gained.** `SHARE_READ`, `SHARE_WRITE` and `MANAGE` over the victim's note, plus
destruction of every grant the owner made.

**Accepted, and out of scope here.** A notebook owner obtaining management over notes that the
notes' own owners moved in deliberately: that is consented delegation, not escalation. Also out of
scope: the server reading note content, which is `orden-26`, and device revocation latency, which
is `orden-10`.

**Non-goals.** This ADR does not redesign the capability lattice, does not introduce roles, and
does not decide whether `Capabilities::all()` is the right ceiling for delegation in general
beyond the sub-decision stated below.

## Options considered

### Option 1 — Keep the current behavior

Record the escalation as accepted risk.

Rejected on its face: a grant of `WRITE` conferring reshare and revocation contradicts the
capability model the code already implements, and `SECURITY.md` would have to be rewritten to
state that "may edit" means "may control access". No benefit beyond zero effort.

### Option 2 — Restrict move authority only

Require ownership of the note (or `MANAGE`) to change `notebook_id`.

Benefits: small, local to `// md:fn update_note`, and it closes the entry point in step 3 above.

Costs and failure modes: it leaves `// md:fn replace_note_shares_from_notebook_tx` destroying
grants, so the `cascade_notebook_to_notes_tx` path still erases an owner's direct grants with no
attacker present. The issue's `Do not` rules this out as a sole fix, and correctly: the escalation
and the grant loss are one defect seen in two places.

Evidence that would change the assessment: a demonstration that no path other than a move can
reach `DELETE FROM note_shares`. The three `cascade_notebook_to_notes_tx` call sites refute it.

### Option 3 — Make the cascade additive

Replace `DELETE`-then-insert with an upsert that unions notebook shares into note shares.

Benefits: Alice's grants survive a move, and the change is confined to two SQL statements.

Costs and failure modes: it makes revocation impossible. Once a notebook share and a direct share
are the same undifferentiated row, removing the notebook share cannot know whether the row was
inherited or granted directly, so it must either leave it — access that cannot be revoked — or
delete it, reintroducing the defect. This option is unsound without provenance, which is Option 4.

### Option 4 — Model provenance: inherited access is computed, not stored

Stop materializing notebook shares into `note_shares`. Keep `note_shares` as the set of **direct**
grants only, and have `// md:fn resolve_note_access` compute effective access as the union of the
direct grant and the access inherited from the containing notebook. Both cascade functions are
deleted rather than repaired. Independently, bind move authority to note ownership, as in Option 2.

Benefits: the contradiction disappears because there is exactly one writer per grant kind.
Revocation becomes correct by construction — removing a notebook share removes inherited access
because nothing was ever copied. An owner's direct grants cannot be destroyed by anyone, because
no code path deletes them except the owner's own `DELETE /api/notes/:id/share`. The escalation is
closed twice over: Bob cannot move the note, and even if he could, moving it would no longer erase
anything.

Costs and failure modes: `resolve_note_access` and the note-listing query take an extra join, and a
migration must decide what today's undifferentiated `note_shares` rows mean. A row that was
inherited becomes a direct grant that outlives its notebook share unless the migration removes it,
and a row that was direct must not be removed. The two are indistinguishable in the current schema;
the migration therefore has to choose a rule and state it.

Operational burden: one forward-only migration and a changed read path. No new background process.

### Option 5 — Explicit consent from the note owner

A move into a notebook the mover does not own creates a pending request that the note's owner must
approve.

Benefits: it is the strongest reading of the sentence already in `http.md`.

Costs and failure modes: it needs a new persistent state, new endpoints, expiry and cleanup for
stale requests, and a UI concept that does not exist. The issue explicitly asks that this be said
before implementation rather than discovered during it. It also solves a problem Option 4 already
removes: once only the owner may move a note, the mover *is* the consenting party.

## Decision and justification

> This ADR is `proposed`. What follows is the recommendation put to the maintainer, not an
> approved decision, and it does not authorize implementation.

**Recommended: Option 4, together with the move restriction from Option 2.**

The invariants it would establish:

1. **Only the note's owner may change `notes.notebook_id`.** A grantee, whatever their
   capabilities, cannot move a note they do not own. `MANAGE` is deliberately not sufficient: a
   move changes which principals inherit access, and that is an ownership decision.
2. **`note_shares` holds direct grants exclusively.** No code path other than the note's own share
   endpoints writes or deletes a row in it.
3. **Inherited access is computed at resolve time**, as the union of the direct grant and the
   access implied by the containing notebook. Nothing is materialized.
4. **Revoking a notebook share revokes the inherited access it conferred**, immediately and without
   a cascade, because the access was never copied.
5. **`can_delete` and `can_transfer_ownership` remain bound to `notes.owner_id`**, unchanged.

Why this over the alternatives: Option 2 alone leaves a live grant-destruction path; Option 3 is
unsound without provenance; Option 5 buys, at the cost of new persistent state, a consent that
invariant 1 already supplies. Option 4 is the only one that removes the contradiction rather than
choosing a side of it, and it makes the fix structural — there is no `DELETE FROM note_shares` left
for a future route to reach.

### Sub-decision requiring a separate ruling

Invariant 3 leaves open **what a notebook owner inherits over a note they do not own**. Today it is
`Capabilities::all()`, which includes `SHARE_WRITE` and `MANAGE`, so a notebook owner may reshare
another user's note. `// md:fn notebook_owner_can_manage_child_notes_they_do_not_own` asserts read,
patch, listing and a delete refusal; it does not assert the share bits, so either choice below
keeps that test green.

- **3a — keep `Capabilities::all()`.** Preserves current behavior exactly. A notebook owner may
  reshare notes that others placed in their notebook.
- **3b — bound inheritance to `READ | WRITE`.** A notebook owner manages content but cannot extend
  access to third parties; only the note's owner may.

Recommended: **3b**, because it keeps the reshare right with the principal who owns the asset and
narrows the blast radius of a compromised notebook owner. It is a behavior change and needs the
maintainer's explicit ruling, not an implementer's judgement.

## Consequences and risks

Positive: the escalation closes; an owner's grants become durable against every actor but the
owner; the prose in `http.md` becomes true once corrected to describe ownership-bound moves rather
than two-sided consent; one whole class of "which writer ran last" bugs is removed with the cascade
functions.

Negative: a read-path join is added to permission resolution and to note listing, on every request
that resolves note access. The cost is expected to be small — both joins are on indexed foreign
keys — but it is not zero and it is not measured here.

Residual risks:

- The migration's rule for existing rows is a judgement call that cannot be made correct by
  inspection, because provenance was never recorded. Whatever is chosen, some deployments will see
  a grant change. This must be stated in the release notes, not only in the migration companion.
- Under 3b, any client that relied on a notebook owner resharing a foreign note breaks. No such
  client is known in-repo, and none can be confirmed for external deployments.
- The decision does not address device-revocation latency on live WebSocket sessions
  (`orden-10`): a session that resolved access before a revocation may continue until that issue
  lands. Convergence of the two is out of scope here and stays with `keeplin-srv#76`.

Observability: none of these paths logs a grant change today. A follow-up issue should record
grant mutations in the NDJSON runtime log once `orden-24` exists; this ADR does not require it.

## Compatibility, migration, and rollback

**Wire and format compatibility: not applicable.** `Capabilities` is defined solely in
`crates/keeplin-srv/src/permissions.rs`, no `keeplin-core` type is touched, shares appear in no
collab protocol variant, and `PROTOCOL_VERSION` does not move. `keeplin`'s pinned revision is
unaffected.

**REST compatibility.** One behavior change is externally visible: `PATCH /api/notes/:id` with a
`notebook_id` by a non-owning grantee returns `403` where it returned `200`. That response is the
fix, not a regression.

**Migration.** One forward-only, idempotent migration with its companion `.md`, under the rules in
`AGENTS.md`: `IF NOT EXISTS` guards, and a `DEFAULT` on any new `NOT NULL` column. It must state
its rule for existing `note_shares` rows explicitly. The recommended rule is to **delete rows that
exactly duplicate a `notebook_shares` row for the note's containing notebook** — those are almost
certainly cascade output, and under invariant 3 the same access is now computed — and to **keep
every other row** as a direct grant. This preserves access in both directions for the common cases
and errs toward keeping access rather than silently removing it. The rule is a recommendation and
part of what the maintainer is being asked to accept.

**Rollout ordering.** Single repository, single deployment; no ordering constraint against
`keeplin`.

**Rollback.** Reverting the code after the migration has run leaves `note_shares` without the rows
the old cascade would have written, so notebook members would lose access to notes until a notebook
share is touched again. A rollback therefore needs its own forward migration that re-materializes
the projection, not a schema downgrade. This must be written and tested **before** the change ships,
not after; a code-only revert is not a rollback plan.

## Verification plan

Criteria 1–3 of `keeplin-srv#110` land inside the harness delivered by `keeplin-srv#111`,
`crates/keeplin-srv/tests/authorization.rs`, which already lists `update_note` in both
`MUTATING_HANDLER_TENANT_CASES` and `MUTATING_HANDLER_CAPABILITY_CASES`. The escalation is a third
dimension that harness does not yet cover: a caller with *legitimate* access gaining more of it.

| # | Evidence | Kind |
|---|---|---|
| 1 | A `WRITE` grantee moving a foreign note into their own notebook gets `403`, and the note's `notebook_id` is unchanged | negative, `sqlx::test` |
| 2 | Alice grants Carol, Bob attempts the move, Carol's access is byte-for-byte unchanged | negative, `sqlx::test` |
| 3 | The capability set resolved after a *legitimate* move is enumerated and compared with the expected set, fixing the ceiling by test rather than by prose | positive |
| 4 | `// md:fn notebook_owner_can_manage_child_notes_they_do_not_own` stays green | regression |
| 5 | `// md:fn nil_notebook_id_patch_means_inbox_and_keeps_shares` stays green | regression |
| 6 | Revoking a notebook share removes inherited access on the next request, with no cascade run | positive |
| 7 | An owner's direct grant survives every notebook-share mutation on the containing notebook | negative, covers the `cascade_notebook_to_notes_tx` half |
| 8 | The migration is applied twice against a populated database with identical end state | migration, idempotence |
| 9 | The rollback migration restores access for notebook members after a code revert | recovery |
| 10 | `./scripts/check-docs.sh` green over the corrected `http.md` and `store.md` companions | documentation |
| 11 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` | repository checks |

Criterion 6 of the issue — that `SECURITY.md` stop claiming audit coverage it does not have — is
evidence, not a verifier, and is recorded as such: it cannot fail on revert and does not gate
convergence.

Not covered, deliberately: performance of the added joins. If the maintainer wants a bound on it,
that is a separate measurement task and should be said at acceptance.

## Equivalent decision in the other repository

None is required. The permission model, both share tables and every function named here are local
to `keeplin-srv`. `Capabilities` is not re-exported from `keeplin-core`, no collab protocol variant
carries a share, and `PROTOCOL_VERSION` is untouched, so `keeplin` needs no paired pull request and
its pinned `keeplin-core` revision is unaffected. Should a future decision move the capability
lattice into `keeplin-core`, that decision would be cross-repo and canonical in `keeplin/docs/adr/`,
and would supersede this record's scope statement rather than amend it.
