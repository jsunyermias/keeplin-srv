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
| `resolve_note_access(store, note, user_id) -> Access` | owner → full `Access`; else the grantee's `note_shares` capabilities; else `Forbidden` |

## Enforcement rules

- **read** (get/export/join): `can_read`.
- **write** (patch, collaborative `Op`): `can_write`.
- **share** (`POST /share`): `can_share_write`, and the granted bits are **capped to the
  granter's own** — you cannot grant a capability you do not hold (no privilege escalation).
- **list shares** (`GET /share`): `can_share_read`.
- **revoke** (`DELETE /share/:user`): `can_share_write`, or removing *yourself*.
- **delete / transfer**: owner only.

The grant is resolved solely from `note_shares` (no notebook fallback at read time) because
the destructive notebook→note cascade materialises notebook grants onto each note. (That
cascade + notebook permissions land in a follow-up; this file covers notes.)

## Design notes

- Capabilities are stored **already-normalised**, so an authorization check is a single mask
  test — cheap and total-ordering-free.
- Sharing targets a user by **email** (resolved to a `user_id` server-side) or by raw id.

## Related files

- `http.md` — REST handlers gate on `resolve_note_access` + the capability checks; the share
  and `transfer` endpoints.
- `collab.md` — the `/api/ws` `Join`/`Op` paths use the same resolution.
- `store.md` — `get_share`/`list_shares`/`create_or_update_share`/`set_note_owner` back it.
- `../../migrations/0005_permissions.sql` — the `note_shares.capabilities` column.
