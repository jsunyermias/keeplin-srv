# `permissions.rs` — note capabilities

## Purpose

Defines the **capability model** and the function that resolves a user's access to a note.
Pure authorization logic with a single database lookup; every note handler and the
collaborative channel call `resolve_note_access` before reading or mutating.

## The capability model (Front B)

A grant is a bitset of capabilities, not a fixed role. Higher bits **imply** the lower ones
(normalised on the way in), so there is no way to hold `WRITE` without `READ`:

| Bit | Value | Implies | Meaning |
|-----|:-:|---------|---------|
| `READ` | 1 | — | see the note |
| `WRITE` | 2 | READ | edit the note/lines |
| `SHARE_READ` | 4 | READ | see who the note is shared with |
| `SHARE_WRITE` | 8 | SHARE_READ, WRITE, READ | grant/revoke shares |
| `MANAGE` | 16 | all of the above | full control short of ownership |

**Ownership is separate and transferable.** The owner (`notes.owner_id`) always has every
capability *plus* the two owner-only powers — **delete** and **transfer ownership** — that no
capability bit confers. Ownership is never a `note_shares` row, so it cannot be revoked by
removing a share.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `Capabilities` | struct (i32 bitset) | normalised capability set; `from_bits`, `all`, `can_read/can_write/can_share_read/can_share_write/can_manage` |
| `Access` | struct | `{ caps: Capabilities, is_owner: bool }` — a user's effective access; adds `can_delete`/`can_transfer_ownership` (owner-only) |

## Public API

| Function | Description |
|----------|-------------|
| `resolve_note_access(store, note, user_id) -> Access` | owner → full `Access`; else the **notebook owner** of the note's notebook → `manage` capabilities (not ownership); else the grantee's `note_shares` capabilities; else `Forbidden` |
| `resolve_notebook_access(store, notebook_id, user_id) -> Access` | owner (`notebooks.user_id`) → full; else `notebook_shares` capabilities; else `Forbidden` (missing notebook → `NotFound`) |

## Notebook permissions & the destructive cascade

Notebooks have the same owner + capability-share model as notes. A notebook's grants
**cascade destructively** onto the notes it contains — a child note's `note_shares` are
*replaced* with a copy of the notebook's `notebook_shares` (see `store.rs`) when:

- the notebook's shares change (grant / revoke), or its ownership is transferred; **and**
- a note is **moved into** the notebook (a move to the Inbox — a null notebook — leaves the
  note's own shares untouched).

Because the cascade materialises the notebook's grants onto each note, `resolve_note_access`
needs no share fallback at read time. The cascade governs the collaborator **grants** only;
it never changes a note's `owner_id` (ownership stays independent and transferable).

**The notebook owner holds implicit `manage` over every child note** (the folder-owner model):
full capabilities on notes filed in their notebook — even notes they do not own — but not the
owner-only powers (delete/transfer stay with `note.owner_id`). This is resolved at access time
in `resolve_note_access` (and mirrored in `list_notes_for_user`), **not** materialised by the
cascade, so a notebook ownership transfer needs no share rewrite.

**Moving a note into a notebook requires `write` on the destination notebook** as well as
`write` on the note: the move adopts the destination's grants (disclosure) and replaces the
note's own shares (possible self-lockout), so it needs consent on both sides. Moving to the
Inbox (null) needs no destination check.

## Enforcement rules

- **read** (get/export/join): `can_read`.
- **write** (patch, collaborative `Op`): `can_write`.
- **share** (`POST /share`): `can_share_write`, and the granted bits are **capped to the
  granter's own** — you cannot grant a capability you do not hold (no privilege escalation).
- **list shares** (`GET /share`): `can_share_read`.
- **revoke** (`DELETE /share/:user`): `can_share_write`, or removing *yourself*.
- **delete / transfer**: owner only.

Apart from the notebook owner's implicit `manage`, a grant is resolved solely from
`note_shares` because the destructive notebook→note cascade materialises notebook grants onto
each note.

## Design notes

- Capabilities are stored **already-normalised**, so an authorization check is a single mask
  test — cheap and total-ordering-free.
- Sharing targets a user by **email** (resolved to a `user_id` server-side) or by raw id.

## Related files

- `http.md` — REST handlers gate on `resolve_note_access` + the capability checks; the share
  and `transfer` endpoints.
- `collab.md` — the `/api/ws` `Join`/`Op` paths use the same resolution.
- `store.md` — note shares (`get_share`/`list_shares`/`create_or_update_share`/`set_note_owner`),
  notebook shares + the cascade (`create_or_update_notebook_share`, `delete_notebook_share`,
  `apply_notebook_shares_to_note`, `cascade_notebook_to_notes`).
- `../../migrations/0005_permissions.sql`, `0006_notebook_permissions.sql` — the capability columns.
