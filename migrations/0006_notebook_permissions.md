# `0006_notebook_permissions.sql` — notebook shares + cascade

## Complete migration

```sql
-- Front B (stage 1b): notebook permissions + the destructive notebook→note cascade.
--
-- Notebook shares mirror note shares: a capability bitset per grantee (see
-- `permissions.rs`). The notebook owner is `notebooks.user_id` (its creator), separate and
-- transferable like note ownership.
--
-- The cascade is applied in application code (see `store.rs`), not by a trigger, so it stays
-- explicit and testable: changing a notebook's shares — or moving a note into a notebook —
-- destructively **overwrites** the affected notes' `note_shares` with the notebook's grants.

CREATE TABLE IF NOT EXISTS notebook_shares (
    notebook_id  UUID NOT NULL REFERENCES notebooks(id) ON DELETE CASCADE,
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    capabilities INTEGER NOT NULL DEFAULT 1,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (notebook_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_notebook_shares_user ON notebook_shares(user_id);

```

## Purpose

The sixth schema migration. Extends the capability model (0005) to **notebooks** and enables the
**destructive notebook→note cascade** (Front B stage 1b): a notebook's grants define who may
access the notes it contains.

## What it defines

| Table | Purpose |
|-------|---------|
| `notebook_shares` | one row per (notebook, grantee): a capability bitmask, mirroring `note_shares` |

The notebook **owner** is `notebooks.user_id` (its creator) — separate and transferable, like
note ownership.

## The cascade (application code, not a trigger)

Applied in `store.rs` so it stays explicit and testable. It **replaces** the affected notes'
`note_shares` with a copy of the notebook's `notebook_shares` when:

- a notebook share is granted / revoked, or the notebook's ownership is transferred
  (`cascade_notebook_to_notes` — every live note in the notebook); or
- a note is **moved into** the notebook (`apply_notebook_shares_to_note` — that one note; a move
  to the Inbox / null notebook leaves the note's shares untouched).

The cascade governs collaborator **grants** only; a note's `owner_id` is never touched.

## Related files

- `../crates/keeplin-srv/src/permissions.rs` — `resolve_notebook_access`.
- `../crates/keeplin-srv/src/store.rs` — the notebook-share CRUD and cascade helpers.
- `../crates/keeplin-srv/src/http.rs` — `/api/notebooks/:id/share` + `/transfer`, and the
  move-cascade hook in `update_note`.
- `0005_permissions.sql` — the note capability column this builds on.
