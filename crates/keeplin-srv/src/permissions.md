# `permissions.rs` — note roles

## Purpose

Defines the sharing roles and the one function that resolves a user's role on a note. Pure
authorization logic with a single database lookup; every note handler and the collaborative
channel call `resolve_role` before mutating.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `Role` | enum | `Owner` \| `Editor` \| `Viewer` |

Role capabilities:

| Role | `can_write()` | `can_share()` |
|------|:-:|:-:|
| `Owner` | ✓ | ✓ |
| `Editor` | ✓ | ✗ |
| `Viewer` | ✗ | ✗ |

## Public API

| Function | Description |
|----------|-------------|
| `resolve_role(store, note, user_id) -> Role` | returns `Owner` if `user_id` owns the note, else the shared role, else `Forbidden` |

## Design notes

- The owner is implicit (`notes.owner_id`), never a `note_shares` row — so ownership cannot be
  revoked by deleting a share.
- Only the owner may share, delete the note, or revoke access (`can_share`); editors write but
  cannot reshare; viewers may join a collaborative session and watch but their `Op`s are
  rejected.

## Related files

- `http.md` — REST handlers gate on `resolve_role` + `can_write`/`can_share`.
- `collab.md` — the `/api/ws` `Join` and `Op` paths use the same resolution.
- `store.md` — `get_share` backs the lookup.
