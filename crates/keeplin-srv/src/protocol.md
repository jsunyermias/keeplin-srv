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

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `LineOp` — defined here (EXTRACTED; 2 cross-file edge(s))
- `Cursor` — defined here (EXTRACTED; 1 cross-file edge(s))
- `LineSnapshot` — defined here (EXTRACTED; 1 cross-file edge(s))
- `NoteLinesSnapshot` — defined here (EXTRACTED; 1 cross-file edge(s))
- `CollabClientMsg` — defined here (EXTRACTED; 1 cross-file edge(s))
- `CollabServerMsg` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.last_writer()` — defined here (EXTRACTED; file-local)
- `PresenceInfo` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/collab.rs` — the collaborative session engine (EXTRACTED: references×7; e.g. `.broadcast()`, `touch_presence()`, `handle_msg()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Every op carries its own `vv`, `last_writer` and `updated_at`; the server resolves against current state with `note_log::resolve` — never by locking.
- The concurrency actor in ops is the **device** id (not the user), so two devices of one user never share a vv component.
- Wire shapes here are the collab protocol contract with keeplin-core's `collab::protocol`; a breaking change requires bumping `PROTOCOL_VERSION` on both sides.

## Related files

- `collab.md` — the engine that produces/consumes these types.
- `keeplin/keeplin-core/src/collab/protocol.md` — the client mirror.
