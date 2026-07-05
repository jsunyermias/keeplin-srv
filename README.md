# Keeplin Server

The multi-user server for [Keeplin](https://github.com/jsunyermias/keeplin) with
**real-time collaborative editing by lines**: several users edit the same note
simultaneously, Google-Docs style but over Markdown, keeping the same concepts
as keeplin-core — `VersionVector`, `last_writer`, `updated_at` and soft-delete
tombstones. No locks anywhere: resolution is always by version vector
(`note_log::resolve`), never by locking.

Written in Rust (axum + PostgreSQL).

## Model

- **The unit of concurrency is the line.** Each line is an independently
  versioned entity that is created, edited, deleted (tombstone) and resolved on
  its own.
- **The order of lines is another versioned entity** with its own `vv`,
  `updated_at` and `last_writer`. It contains every `line_id`, deleted ones
  included, until garbage collection.
- **The server is the broker and the durable source of truth**: it validates
  each operation, resolves it against current state, persists it and forwards
  it to the note's other subscribers. Clients are stateful and rebuild from the
  snapshot on (re)connect — there is no infinite op log.
- A note's `body` is not stored: it is materialised by joining the live lines
  with `\n` for non-collaborative REST reads.

## Collaborative protocol (`GET /api/ws?token=<jwt>`)

JSON messages with a `type` field:

- Client → server: `Join { note_id }`, `Leave { note_id }`,
  `Op { note_id, ops: [LineOp…] }`, `Cursor { note_id, cursor }`, `Ack { server_seq }`.
- Server → client: `Welcome { note_id, snapshot }` (versioned order + every
  line), `Op { server_seq, note_id, user_id, ops }`, `Presence { note_id, users }`,
  `Error { code, message }`.

`LineOp` (`op`): `Insert { after_line_id, line_id, content, vv, last_writer, updated_at }`,
`Update`, `Delete` (tombstone) and `Move { line_ids, after_line_id, … }`. Every
operation carries its own `vv`; the server requires `last_writer` to be the
authenticated user and the vector to advance the writer's component.

**Resolution** (design §5): per line, `resolve(local, incoming)` — a dominated
operation is ignored; concurrent ones fall to the deterministic
`(updated_at, last_writer)` tiebreak, identical on every replica. `Insert`/`Move`
resolve against the order entity.

**Limits**: 10,000 characters per line, 100,000 lines per note, 1 MB per message.

## REST API

- `GET /health` · `GET /api/metrics` (aggregate counters: users, notes, lines,
  tombstones, live sessions/connections)
- `POST /api/register` — `{ email, password, display_name? }`
- `POST /api/login` — `{ email, password, device_name }` → `{ token, device_id }`
- `POST /api/devices` · `GET /api/devices` · `DELETE /api/devices/:id` (revokes
  that device's token immediately) (Bearer)
- `POST /api/notes` — `{ title }` · `GET /api/notes` — owned and shared
- `GET /api/notes/:id` — metadata + materialised `body` · `PATCH` (title) ·
  `DELETE` (owner only, soft delete)
- `POST /api/notes/:id/share` — `{ user_id | user_email, role }` (`editor`/`viewer`,
  owner only) · `DELETE /api/notes/:id/share/:user_id`
- `POST /api/import` — `{ title, body }` splits the body into lines (offline →
  server migration) · `GET /api/notes/:id/export` — joins the live lines
  (server → offline)

### Roles

| Role | Permissions |
|------|-------------|
| `owner` | read, edit, share, delete the note |
| `editor` | read and edit |
| `viewer` | join the session and watch; cannot send operations |

## Device sync relay (`GET /api/sync`)

Besides the collaborative channel, the server implements the WebSocket relay
that keeplin-core's current `DbBackend` speaks (`{"type":"auth","token"}`
handshake + `{"type":"changes",…}` envelopes), with a persistent journal,
deferred catch-up via per-device cursors and retry deduplication. It syncs one
user's devices while collaborative mode lands in the daemon. One login (one
token) per device.

### Connecting a keeplin-daemon

```bash
# 1. Create an account (once)
curl -X POST http://localhost:3000/api/register \
  -H 'content-type: application/json' \
  -d '{"email":"me@example.com","password":"long-secret"}'

# 2. Get a token FOR EACH device (do not share the token across machines!)
curl -X POST http://localhost:3000/api/login \
  -H 'content-type: application/json' \
  -d '{"email":"me@example.com","password":"long-secret","device_name":"laptop"}'
# → { "token": "…", "device_id": "…" }
```

In the daemon's `config.toml`:

```toml
mode = "server"
server_url = "ws://localhost:3000/api/sync"   # wss:// in production
auth_token = "<token from step 2>"
```

## Requirements

- Rust >= 1.75
- PostgreSQL 16 (or use Docker Compose)

## Quick start

```bash
# 1. Start PostgreSQL
docker compose up -d

# 2. Copy environment variables
cp .env.example .env   # change JWT_SECRET in production

# 3. Build and run
cargo run
```

The server listens on `http://localhost:3000`.

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3000` | HTTP/WS port |
| `DATABASE_URL` | — (required) | PostgreSQL connection string |
| `JWT_SECRET` | dev value | Token signing secret; change it |
| `TOKEN_TTL_DAYS` | `365` | Token lifetime |
| `CHANGES_RETENTION_DAYS` | `0` (disabled) | Relay journal pruning |
| `LINES_GC_DAYS` | `30` | Compact line tombstones older than N days (`0` disables) |
| `RUST_LOG` | `info` | Log level |

In production terminate TLS at a reverse proxy (`wss://`). The collaborative
channel accepts the token in the `Authorization: Bearer` header (preferred —
query strings end up in proxy logs) with `?token=` kept as a fallback.

## Tests

```bash
export DATABASE_URL=postgres://keeplin:keeplin@127.0.0.1:5432/keeplin
cargo test
```

- `tests/collab.rs` — the collaborative protocol end to end: Join/Welcome, op
  propagation with `server_seq`, deterministic resolution of concurrent edits,
  ignored replays, Move, presence with cursors, roles (viewer without write,
  outsiders without access), forged `last_writer` rejection and import/export.
- `tests/integration.rs` — the device relay with the real client
  (keeplin-core's `DbBackend`).

CI (GitHub Actions) runs fmt, check, the tests against Postgres 16 and clippy.
