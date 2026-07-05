# `protocol.rs` — collaborative wire types

## Purpose

The JSON message types of the collaborative channel (`GET /api/ws`), shared by the server
(`collab.rs`) and mirrored by the client in keeplin-core. Pure data definitions — no logic.
The client half in `keeplin/keeplin-core/src/collab/protocol.rs` must stay byte-compatible.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `LineOp` | enum (`op`-tagged) | `Insert` / `Update` / `Delete` / `Move`; each carries its own `vv`, `last_writer`, `updated_at` |
| `CollabClientMsg` | enum (`type`-tagged) | `Join` / `Leave` / `Op` / `Cursor` / `Ack` |
| `CollabServerMsg` | enum (`type`-tagged) | `Welcome` / `Op` / `Presence` / `Error` |
| `LineSnapshot`, `NoteLinesSnapshot` | struct | a line and the full note state sent in `Welcome` |
| `Cursor`, `PresenceInfo` | struct | caret position and a participant's presence entry |

## The wire protocol

- **Serde tags**: messages use `#[serde(tag = "type")]`, ops use `#[serde(tag = "op")]`, both
  `rename_all = "PascalCase"` — so a frame looks like `{"type":"Op","note_id":…,"ops":[{"op":"Insert",…}]}`.
- **`UserId` type alias**: in presence it is a user id; in ops (`last_writer` and the vv keys)
  it is the **device** id — the concurrency actor. Documented on the alias.
- **`Welcome` carries a full snapshot** (versioned order + every line, tombstones included) so
  a reconnecting client rebuilds from state rather than replaying a log.

## Notes & gotchas

- Changing a field name or the tag renaming breaks the client; the two `protocol.rs` files are
  a contract. Add fields as optional (`#[serde(default)]`) to stay backward compatible.
- The server's `Op` includes `note_id` (a deliberate addition over the original design sketch)
  so one connection can multiplex several notes.

## Related files

- `collab.md` — the engine that produces/consumes these types.
- `keeplin/keeplin-core/src/collab/protocol.md` — the client mirror.
