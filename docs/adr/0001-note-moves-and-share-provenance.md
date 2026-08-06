# 0001 — Note moves and the provenance of note shares

- Status: accepted
- Date: 2026-08-06
- Decision owners: maintainer of `jsunyermias/keeplin-srv`
- Scope: keeplin-srv
- Issue: [keeplin-srv#110](https://github.com/jsunyermias/keeplin-srv/issues/110)
- Acceptance PR: [keeplin-srv#121](https://github.com/jsunyermias/keeplin-srv/pull/121)
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

That ceiling is complete only if no handler reads `MANAGE` directly and does something further
with it. It does not: `Access` deliberately exposes no `can_manage` accessor, and no call to
`can_manage` exists anywhere outside `crates/keeplin-srv/src/permissions.rs` — the only other
occurrences of the string are inside the name of
`// md:fn notebook_owner_can_manage_child_notes_they_do_not_own`. One nit for exactness: "anyone"
excludes the owner, whom `// md:fn create_share` rejects with `BadRequest`.

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

  This is the **third** symptom of the same contradiction, and it is not benign. The grants that
  survive include the ones the cascade materialized. Alice shares notebook *NB* with Carol, which
  copies a row into `note_shares` for the contained note *N*; Alice then moves *N* to the Inbox;
  Carol keeps access to *N* although *N* is no longer in *NB* and Carol was never granted anything
  on *N* directly. The row is orphaned: nothing distinguishes it from a deliberate direct grant,
  and no later revocation of the notebook share can find it, because
  `// md:fn cascade_notebook_to_notes_tx` only touches notes still in the notebook.
- The relay is not a second entry point, but the evidence for that is weaker than it looks.
  `// md:fn note_changes_are_explicitly_non_materializing` does not exercise materialization: it
  `include_str!`s `crates/keeplin-srv/src/sync.rs` and asserts that the `Change::NoteCreate |
  NoteUpdate | NoteDelete` arm is present and returns `Ok(())`. That is an assertion about the
  shape of the source, and `RELAY_CHANGE_CAPABILITY_UNCOVERED` is a hand-written attestation
  beside it. Both fail if the arm is edited away, which is enough to keep this ADR's claim honest,
  but neither is a behavioral test. `PATCH /api/notes/:id` is the only route that moves a note
  today; nothing proves at runtime that it stays the only one.

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
- A third party's access is never removed without the owner deciding it and the affected principal
  being told. Silent revocation is the mirror image of silent escalation and this decision must not
  trade one for the other.
- Notification must not be a precondition of a permission change. `// md:impl Mailer > fn enabled`
  is false whenever no webhook is configured, so any rule that blocks on delivery either breaks
  mailer-less deployments or drops notices quietly.
- The policy points this decision fixes must be selectable per deployment rather than compiled in,
  so that a deployment can adopt a different trade-off without a fork.

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

**Accepted, and out of scope here.** A principal obtaining the scheme-bounded access that follows
from a note owner deliberately placing a note in a notebook: that is consented delegation, not the
escalation. Under `strict` that inherited access is at most `READ | WRITE`, for the notebook owner
and its grantees alike; a deployment may deliberately select a different ceiling. Also out of
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
because nothing was ever copied. Direct grants can change only through the note-share endpoints,
under their explicit capability checks; notebook moves and notebook-share mutations can no longer
destroy them as a side effect. This distinction matters because the current delete endpoint also
permits a grantee to revoke their own access and permits a principal with `SHARE_WRITE` to revoke
other grants. The escalation is closed twice over: Bob cannot move the note, and even if he could,
moving it would no longer erase anything.

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

**Partially adopted.** Invariants 6 and 7 take this option's insight — that a move is not purely
the mover's business — without its cost. The refusal is computed from state that already exists, so
there is no pending-request record, no expiry and no cleanup; the owner resolves it by granting or
revoking through endpoints that already exist. What is *not* adopted is consent by the affected
party: they are told, and they are protected from silent loss, but they cannot veto.

## Decision and justification

> This ADR is `accepted`. It authorizes implementation of
> [keeplin-srv#110](https://github.com/jsunyermias/keeplin-srv/issues/110), and its decision body
> is now immutable historical record.

**Decision: Option 4, together with the move restriction from Option 2, expressed through a named
permission scheme whose default is stated below.**

The invariants it establishes:

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
6. **A move never removes a third party's access silently — where the mover controls that access.**
   A move that would drop the inherited access of another principal is refused **when that access
   derives from a grant the mover can themselves undo**. The mover resolves it explicitly first:
   preserving the access needs no notice, while revoking it does. Access deriving from a grant the
   mover does not control does not block: the move succeeds and the affected principal is notified.
   See the ordering rule.
7. **A move is refused while another principal holds capabilities equal to the owner's**, under the
   same control bound as invariant 6. A grantee holding `MANAGE` normalizes to every bit in
   `Capabilities::all()` and is therefore the owner's capability equal on that note; relocating the
   note out from under such a principal is not a decision one of two equals takes alone. But that is
   only true when the owner *made* that principal their equal. A full set inherited from a notebook
   the mover does not own is someone else's grant and does not bind them.

   Invariant 7 also applies only where the full set **depends on containment**. A principal holding
   `MANAGE` through a direct grant on the note loses nothing when the note moves — invariants 2 and 3
   make direct grants independent of where the note sits — so refusing the move protects no asset of
   theirs and merely forces the owner to demote them, with a notice, to relocate their own note. The
   risk invariant 7 exists to address is an equal whose standing the move would remove.

   The control bound in 6 and 7 is not a softening. Without it the guard is a hostage mechanism:
   Alice moves her note into Mallory's notebook, Mallory has shared that notebook with Bob at the
   full set, and Alice can then neither preserve (a direct grant matching Bob's inherited set keeps
   him her equal, so invariant 7 still refuses) nor revoke (the grant is Mallory's and Alice cannot
   touch another user's `notebook_shares`). Her own note would be trapped until Mallory chose to
   cooperate, and any notebook grantee could hold it there without lifting a finger. The rule
   protects a principal from losing access **you** gave them; it was never meant to let a stranger's
   grant overrule your ownership.

Why this over the alternatives: Option 2 alone leaves a live grant-destruction path; Option 3 is
unsound without provenance; Option 5 buys, at the cost of new persistent state, a consent that
invariant 1 already supplies. Option 4 is the only one that removes the contradiction rather than
choosing a side of it, and it makes the fix structural — there is no `DELETE FROM note_shares` left
for a future route to reach.

### The permission scheme

The policy choices around the single ejection exception to invariant 1, invariants 6 and 7, and the
inheritance ceiling are not universal truths; they are the *default* policy of a deployment. The
core of invariant 1 remains structural under every scheme. The maintainer's ruling is that this ADR
defines a **named permission scheme** rather than hard-coding the selectable policy points, so that
a deployment can select a different trade-off without a code change, and so that the two rulings
below are recorded as defaults rather than as facts about the domain.

A scheme is a named, deployment-selected value carrying exactly four policy points. It does not
introduce roles, does not make `Capabilities` dynamic, and does not touch the bit lattice in
`// md:impl Capabilities` — the bits and their normalization stay exactly as they are.

| Policy point | What it fixes | `strict` (default) |
|---|---|---|
| `notebook_inheritance` | Ceiling on **every** capability inherited by containment over a note the inheritor does not own — the notebook's owner and its grantees alike | `READ \| WRITE` |
| `foreign_note_ejection` | Whether the containing notebook's owner may move a foreign note out to the Inbox | denied |
| `move_out_guard` | What happens to a move that would drop inherited access the mover controls | refuse until explicitly resolved, then notify |
| `equal_principal_guard` | Whether a move is refused while another principal the mover made holds the owner's full capability set | refuse |

`notebook_inheritance` is a **ceiling applied after** the notebook grant is read, not a substitute
for it. Inheritance is computed as the notebook-derived capabilities of the principal — the notebook
owner's full rights, or a grantee's `notebook_shares` bits — intersected with this ceiling. Applying
it to the owner alone would leave the hole open one hop away: a notebook owner bounded to
`READ | WRITE` over a foreign note could still share **the notebook** with `SHARE_WRITE`, and that
grantee would inherit `SHARE_WRITE` over the foreign note and reshare it. The notebook owner would
thereby confer over someone else's note a power they do not themselves hold, which is the reshare
half of `keeplin-srv#110` returning by another door. The ceiling binds every principal who gets
anything by containment.

Selection is a single `Config` field, read from the environment beside the other policy scalars in
`// md:Config`, defaulting to `strict` when unset. An unrecognized value is a startup error, not a
silent fallback: a deployment that believes it selected a permissive scheme must not get a strict
one, and the reverse is worse.

Three properties are **not** scheme-configurable and no scheme may weaken them, because they are the
defect this ADR exists to close:

- Invariants 2, 3 and 4. Materialization does not come back under any scheme; provenance is
  structural, not policy.
- Invariant 5. `can_delete` and `can_transfer_ownership` stay bound to `notes.owner_id`.
- **The core of invariant 1.** That only the note's owner may change `notes.notebook_id` is
  structural, not policy: a scheme that let grantees move notes again would reopen the escalation
  directly. `foreign_note_ejection` modulates the single 1b exception — a notebook owner removing a
  foreign note to the Inbox — and nothing else. No policy point grants move authority to a grantee.

The destination check is unchanged and is not scheme-configurable either: an owner moving their own
note into a notebook must still be able to write that notebook, exactly as
`// md:fn update_note` requires today via `resolve_notebook_access`.

An explicit grant by the note's owner always overrides the scheme upward. The scheme fixes what a
principal gets *without* being granted anything — inheritance by containment. It never caps what an
owner may deliberately confer with `// md:fn create_share`.

The scheme deliberately stops at these four points. Anything broader — a `FULL_CONTROL` capability
and its separation from `MANAGE`, or administrable limits and feature flags — belongs to
[keeplin-srv#80](https://github.com/jsunyermias/keeplin-srv/issues/80) (`orden-18`) and
[keeplin-srv#81](https://github.com/jsunyermias/keeplin-srv/issues/81) (`orden-19`) and is not
decided here. Those issues extend this scheme; they do not replace it. Recorded because the
alternative considered at authoring time was to scope the scheme out of this ADR entirely and leave
the two rulings as bare constants; the maintainer chose the scheme.

### First ruling: what a notebook owner inherits

Today it is `Capabilities::all()`, which includes `SHARE_WRITE` and `MANAGE`, so a notebook owner may
reshare another user's note. `// md:fn notebook_owner_can_manage_child_notes_they_do_not_own` asserts
read, patch, listing and a delete refusal; it does not assert the share bits, so either choice keeps
that test green.

- **3a — keep `Capabilities::all()`.** Preserves current behavior exactly.
- **3b — bound inheritance to `READ | WRITE`.** A notebook owner manages content but cannot extend
  access to third parties; only the note's owner may.

**Ruled: 3b, as the `strict` default.** The reshare right stays with the principal who owns the
asset, and the blast radius of a compromised notebook owner narrows. A deployment that wants the old
behavior selects a scheme with `notebook_inheritance = all`, and a note owner who wants a specific
notebook owner to have more grants it explicitly.

### Second ruling: may a notebook owner eject a foreign note?

Invariant 1 has a consequence it does not state. Today a notebook owner inherits
`Capabilities::all()` over a contained note and can therefore change its `notebook_id` — moving it
between their own notebooks, or ejecting it to the Inbox. Under invariant 1 read literally that
becomes `403`, and since `can_delete` is already bound to ownership, a foreign note placed in
someone's notebook is **trapped there** until its own owner acts.

- **1a — invariant 1 literal.** Only the note's owner may ever change `notebook_id`; the
  trapped-note case is accepted.
- **1b — owner, plus eject-to-Inbox by the containing notebook's owner.**

**Ruled: 1a, as the `strict` default.** The note stays where its owner put it, and a notebook owner
who wants it gone asks its owner. The cost is accepted knowingly: a notebook owner has no unilateral
remedy against a foreign note in their notebook, which is a nuisance surface rather than a security
one, since inheritance under 3b confers no power to the note's owner over the notebook. A deployment
that finds the nuisance unacceptable selects `foreign_note_ejection = allowed`.

### The ordering rule for a move that drops access

Invariants 6 and 7 need a defined sequence, because "notify the affected" cannot be a precondition
the server is always able to satisfy: `// md:impl Mailer > fn enabled` is false whenever no webhook
is configured, and `// md:impl Mailer > fn send` is shaped for a token and an expiry, not for a
general notice.

The sequence, under the `strict` scheme:

1. The move is evaluated for principals who would lose inherited access. They split in two by the
   control bound of invariants 6 and 7:
   - **Controlled** — their access derives from a grant the mover can undo: a share on a notebook
     the mover owns, or a direct share on the note the mover owns. These block; the move is refused
     and nothing changes.
   - **Uncontrolled** — their access derives from a notebook the mover does not own. These do not
     block. The move proceeds and they are notified, because the mover has no way to resolve them
     and an owner's claim on their own note outranks a grant a third party made.
2. If any *controlled* principal holds the owner's full capability set, the refusal is terminal for
   as long as that holds: invariant 7. The owner revokes or reduces that grant first.
3. For each remaining affected principal the owner chooses, deliberately, one of two things:
   - **Preserve.** Grant them a direct share on the note with
     `// md:fn create_share`. Their access then survives the move, because a direct grant does not
     depend on containment. Nothing is revoked and no notice is owed.
   - **Revoke.** Remove the grant explicitly through the ordinary share endpoints. This is the
     point at which a notice is owed — the revocation, not the move.

   Preserve exists so that the ordering rule does not force a disproportionate act. The inherited
   access being dropped comes from a *notebook* share, and revoking that would remove the
   principal's access to **every** note in the notebook, not just the one being moved. Requiring
   that in order to relocate a single note would make the guard worse than the problem.
4. With no controlled principal left to lose access, the move proceeds. Uncontrolled principals from
   step 1 are notified at this point — that notice is owed by the move, which is the one case where
   it is, precisely because no revocation the owner could have performed exists to carry it.

The refusal response names the affected principals so the owner can act, but it must not name
principals the mover could not otherwise enumerate. A mover who does not hold `SHARE_READ` on
another user's notebook has no right to learn its membership, and a refusal that listed it would
turn this guard into a disclosure oracle for someone else's sharing. Those are reported as a count
and the notebook that confers them, without identities. Controlled principals are by definition ones
the mover granted, so naming them discloses nothing new.

### When ejection is enabled

Under a scheme with `foreign_note_ejection = allowed`, the ejector is the containing notebook's
owner, not the note's owner, and the guards apply **with the ejector as the actor**. Grants that
ejector controls — shares on the notebook they own — are controlled and block, resolvable by the
same Preserve or Revoke choice. Everything else is uncontrolled and is notified rather than refused.
The note's owner is always among the notified: having one's note ejected from a notebook is exactly
the kind of change invariant 6 exists to stop happening silently. Stated because an implementation
would otherwise have to guess whose guards apply, and every guess is defensible.

For controlled access, notification is therefore attached to **revocation**, which the controlling
owner performs deliberately, and not to the later move, which no longer removes that access. The
exception is the explicitly non-blocking loss of uncontrolled inherited access in step 4, where the
move itself owes the notice because no revocation available to the mover exists. That keeps a
deployment without a configured mailer fully functional: it can still revoke and still move, and it
simply cannot send the notice. A failed or unconfigured notice **must not** roll back the operation
that owed it — losing an authorized revocation or trapping an owner's note is worse than losing the
e-mail — but it must be recorded, which is a new `MailKind` and a runtime log line rather than new
persistent state.

This is the point where an implementer would otherwise have invented something: the naive reading of
"notify the affected" makes mail delivery a precondition of a permission change, which either breaks
every mailer-less deployment or silently drops notices. Neither is acceptable and neither is what the
ruling asks for.

## Consequences and risks

Positive: the escalation closes; an owner's grants become durable against every actor but the
owner; the prose in `http.md` becomes true once corrected to describe ownership-bound moves rather
than two-sided consent; one whole class of "which writer ran last" bugs is removed with the cascade
functions.

Negative: a read-path join is added to permission resolution and to note listing, on every request
that resolves note access. The cost is expected to be small — both joins are on indexed foreign
keys — but it is not zero and it is not measured here.

Residual risks:

- The migration keeps rows it cannot attribute, so a deployment carries forward some access that
  was derived rather than intended. That is the accepted cost of never revoking silently, and the
  reported count is the only thing that makes it auditable. It must be stated in the release notes,
  not only in the migration companion.
- Under 3b, any client that relied on a notebook owner resharing a foreign note breaks. No such
  client is known in-repo, and none can be confirmed for external deployments.
- Under 1a, a foreign note in a notebook cannot be removed by that notebook's owner. This is a
  usability cost accepted knowingly and it has a support consequence: an operator will eventually be
  asked to remove such a note and there is no endpoint that does it. The remedy is to ask the note's
  owner, or to select `foreign_note_ejection = allowed`.
- Invariants 6 and 7 make a previously unconditional operation conditional. A note owner can now be
  refused a move of their own note, which is a new class of `403` that clients must render
  meaningfully rather than as a generic failure — the response has to name the affected principals
  or the user cannot act on it.
- The scheme is a new configuration surface, and a misconfigured deployment gets a different
  permission policy than its operator believes. That is why an unrecognized value is a startup
  error. It remains true that a *recognized* but unintended value fails silently in the only way
  that matters: correctly, per its own definition.
- The decision does not address device-revocation latency on live WebSocket sessions
  (`orden-10`): a session that resolved access before a revocation may continue until that issue
  lands. Convergence of the two is out of scope here and stays with `keeplin-srv#76`.

Observability: none of these paths logs a grant change today, and invariant 6 now makes notices a
first-class outcome that can fail independently of the operation that owed them. A failed notice
must be recorded from the start — a runtime log line and a new `MailKind`, not new persistent state.
Richer NDJSON grant-mutation events remain a follow-up once `orden-24` exists.

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
its rule for existing `note_shares` rows explicitly.

**Ruled: the migration deletes nothing. It keeps every row and reports the ones it cannot
attribute.**

The tempting rule — delete rows that exactly duplicate a `notebook_shares` row of the note's
containing notebook, on the grounds that they are almost certainly cascade output — must be
rejected, and the reason is the ADR's own epistemology rather than a preference. The premise for
keeping orphans is that *an orphan is byte-identical to a deliberate direct grant*. That sentence
applies verbatim to the duplicates: Alice granting Carol `READ | WRITE` on note *N* while Carol
already holds `READ | WRITE` on the containing notebook produces a row indistinguishable from
cascade output. Deleting it would not remove Carol's access today — containment still confers it —
but it would silently strip the grant's **durability**: a later notebook revocation, or a move, now
takes away access that Alice deliberately conferred. That is a silent revocation on a delay fuse,
which is precisely what invariant 6 forbids, performed at migration scale with no owner in the loop.

Keeping everything is therefore the only clause consistent with the rest of this decision, and it
removes the asymmetry a two-clause rule would have introduced between two row kinds that no query
can tell apart.

Two kinds of row cannot be attributed to a deliberate `create_share`, and both are **reported, not
deleted**:

- rows whose `(note_id, user_id, capabilities)` matches a `notebook_shares` row of the note's
  containing notebook;
- every row on a note whose `notebook_id` is now `NULL`, which no containing-notebook comparison can
  reach at all.

The report is **row-level**, not a count. A bare integer tells an operator that something needs
auditing while withholding everything needed to audit it; `(note_id, user_id, capabilities)` and
which of the two kinds it is makes the list actionable. An owner who reviews it and decides a grant
was never intended revokes it through the ordinary endpoint, where the notice is owed.

Note also that no *new* orphan can be created after this change: invariant 3 stops copying, so there
is no derived row left to be orphaned. The migration is a one-time reckoning with rows the old design
already wrote, not an ongoing concern.

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
| 4 | `// md:fn notebook_owner_can_manage_child_notes_they_do_not_own` stays green: it asserts read, patch, listing and a delete refusal, none of which 3b removes | regression |
| 5 | Moving a note to the Inbox preserves a **direct** share but removes an inherited-only principal's access on the next request. `// md:fn nil_notebook_id_patch_means_inbox_and_keeps_shares` stays green for the direct half; a new `sqlx::test` fixes the orphaned-materialization half | regression + negative, REV-121-01 |
| 6 | Revoking a notebook share removes inherited access on the next request, with no cascade run | positive |
| 7 | An owner's direct grant survives every notebook-share mutation on the containing notebook | negative, covers the `cascade_notebook_to_notes_tx` half |
| 8 | A move that would drop a **controlled** principal's inherited access is refused, the response names them, and the note is unmoved | negative, invariant 6 |
| 9 | After the owner preserves the affected principal with a direct share, the same move succeeds and that principal still resolves access afterwards | positive, the Preserve branch |
| 10 | After the owner revokes instead, the move succeeds and the principal no longer resolves access | positive, the Revoke branch |
| 11 | A move is refused while a **controlled** principal holds `MANAGE` **by containment**, and stays refused until that grant is reduced; a principal holding `MANAGE` by a direct grant does not block, and still resolves it after the move | negative + positive, invariant 7 and its bound — REV-121-11 |
| 12 | **The note's owner always retains a unilateral exit.** Alice's note sits in Mallory's notebook, which Mallory shares with Bob at the full set; Alice completes the move using only endpoints she is authorized for, and Bob is notified rather than blocking. Fails against an unbounded invariant 6/7 | negative, the control bound — REV-121-06 |
| 13 | A refusal does not name principals the mover cannot enumerate: those inherited from a notebook the mover neither owns nor holds `SHARE_READ` on appear as a count plus the conferring notebook | negative, disclosure — REV-121-13 |
| 14 | A revocation whose notice fails to send still commits, and the failure is recorded | failure injection, the mailer-less deployment |
| 15 | An unrecognized scheme name fails startup rather than falling back | negative, configuration |
| 16 | Under `strict`, a notebook owner resolves exactly `READ \| WRITE` over a foreign contained note, and `POST /api/notes/:id/share` by them returns `403` | positive, ruling 3b |
| 17 | **Under `strict`, a notebook *grantee* holding `SHARE_WRITE` on the notebook also resolves at most `READ \| WRITE` over a foreign contained note, and their `POST /api/notes/:id/share` returns `403`.** Fails if the ceiling binds only the notebook owner | negative, the transitive hole — REV-121-08 |
| 18 | Under `strict`, a notebook owner ejecting a foreign note to the Inbox gets `403` | negative, ruling 1a |
| 19 | Under `foreign_note_ejection = allowed`, an ejection that would drop a controlled notebook grantee's inherited access is first refused with the ejector as actor; after Preserve or Revoke resolves that loss, the ejection succeeds and the note's owner is notified | negative + positive, ejection × guard — REV-121-09 |
| 20 | The migration is applied twice against a populated database with identical end state | migration, idempotence |
| 21 | **The migration deletes no row.** A deliberate `create_share` grant that exactly duplicates a `notebook_shares` row survives, and still resolves after the notebook share is later revoked | negative, REV-121-07 |
| 22 | The migration's report identifies rows at `(note_id, user_id, capabilities)` granularity, not merely a count, on a fixture holding both unattributable kinds | migration, REV-121-10 |
| 23 | The rollback migration restores access for notebook members after a code revert | recovery |
| 24 | `./scripts/check-docs.sh` is green over every companion changed by the implementation, including `http.md`, `permissions.md`, `store.md`, configuration, mail and migration companions | documentation |
| 25 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` | repository checks |

Criterion 6 of the issue — that `SECURITY.md` stop claiming audit coverage it does not have — is
evidence, not a verifier, and is recorded as such: it cannot fail on revert and does not gate
convergence.

Not covered, deliberately: performance of the added joins. Any bound on it is a separate
measurement task.

## Equivalent decision in the other repository

None is required. The permission model, both share tables and every function named here are local
to `keeplin-srv`. `Capabilities` is not re-exported from `keeplin-core`, no collab protocol variant
carries a share, and `PROTOCOL_VERSION` is untouched, so `keeplin` needs no paired pull request and
its pinned `keeplin-core` revision is unaffected. Should a future decision move the capability
lattice into `keeplin-core`, that decision would be cross-repo and canonical in `keeplin/docs/adr/`,
and would supersede this record's scope statement rather than amend it.
