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

- `GET /health` (liveness) · `GET /ready` (readiness — DB round-trip, `503` if down) · `GET /version` (protocol version + capabilities) · `GET /api/metrics` (aggregate counters — **auth required**: users, notes, lines,
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
# 1. Start PostgreSQL only
docker compose up -d postgres

# 2. Copy environment variables
cp .env.example .env   # then set JWT_SECRET (required): openssl rand -hex 32

# 3. Build and run
cargo run
```

The server listens on `http://localhost:3000`.

## Docker

A multi-stage `Dockerfile` produces a small, self-contained image (migrations are
embedded into the binary, so nothing else is copied; runs as a non-root user).
The `keeplin-core` git dependency is **pinned by commit** in `Cargo.toml` for
reproducible builds — bump that `rev` to adopt newer keeplin.

```bash
# Whole stack (Postgres + server) for a local/demo run.
# JWT_SECRET is REQUIRED — compose refuses to start without it:
cp .env.example .env && printf 'JWT_SECRET=%s\n' "$(openssl rand -hex 32)" >> .env
docker compose up --build

# Or just the image:
docker build -t keeplin-srv .
docker run --rm -p 3000:3000 \
  -e DATABASE_URL=postgres://user:pass@host:5432/keeplin \
  -e JWT_SECRET=$(openssl rand -hex 32) \
  keeplin-srv
```

The Compose topology is dev/demo only: Postgres is bound to loopback (not the LAN), and `JWT_SECRET` must be supplied via `.env` (no working default). For production use real Postgres credentials, put a TLS reverse proxy in front, and consider `REGISTRATION_ENABLED=false`.

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3000` | HTTP/WS port |
| `DATABASE_URL` | — (required) | PostgreSQL connection string |
| `JWT_SECRET` | **required** | Token signing secret; server refuses to start on a weak/placeholder value (issue #19). `KEEPLIN_DEV_INSECURE=1` for local dev |
| `TOKEN_TTL_DAYS` | `365` | Token lifetime |
| `CHANGES_RETENTION_DAYS` | `0` (disabled) | Relay journal pruning |
| `LINES_GC_DAYS` | `30` | Compact line tombstones older than N days (`0` disables) |
| `DB_MAX_CONNECTIONS` | `10` | PostgreSQL pool size |
| `DB_ACQUIRE_TIMEOUT_SECS` | `10` | Fail a request instead of blocking forever when the pool is exhausted |
| `DB_IDLE_TIMEOUT_SECS` | `600` | Reap idle pooled connections |
| `DB_MAX_LIFETIME_SECS` | `1800` | Recycle pooled connections after this age |
| `RATE_LIMIT_PER_MIN` | `0` (disabled) | Per-client-IP request budget/minute; leave `0` behind a proxy and limit there |
| `SHUTDOWN_GRACE_SECS` | `20` | Drain window on SIGTERM/Ctrl-C before force-exit |
| `LOG_JSON` | `false` | Emit JSON logs (one object/line) for aggregation |
| `MAX_UPLOAD_BYTES` | `104857600` (100 MiB) | Max size of a resource binary upload (`PUT /api/resources/:id/data`); `413` over it |
| `MAX_NOTE_BODY_BYTES` | `26214400` (25 MiB) | Max size of a materialised note body (`GET /api/notes/:id`, export); `413` over it. `0` disables |
| `MAX_USER_STORAGE_BYTES` | `0` (disabled) | Per-user resource-blob storage cap; `507` when a blob upload would exceed it |
| `MAX_NOTES_PER_USER` | `0` (disabled) | Per-user live-note cap; `507` when creating past it |
| `RUST_LOG` | `info` | Log level |

The server drains in-flight requests on `SIGTERM`/`Ctrl-C` (bounded by
`SHUTDOWN_GRACE_SECS`, since collaborative WebSockets are long-lived), so it is
safe under systemd/Kubernetes rolling restarts. `/health` is never rate-limited
so liveness probes always pass.

In production terminate TLS at a reverse proxy (`wss://`). The collaborative
channel accepts the token in the `Authorization: Bearer` header (preferred —
query strings end up in proxy logs) with `?token=` kept as a fallback.

## Operating in production

keeplin-srv is stateless — all durable state lives in PostgreSQL — so operating it is mostly
operating its database. See **[`RUNBOOK.md`](RUNBOOK.md)** for backup/restore (pg_dump and PITR),
upgrades, routine maintenance, capacity/quotas, and an incident quick reference.

### Running multiple replicas

The server scales horizontally: run several instances behind a load balancer, all pointed at the
same PostgreSQL. Instances coordinate the live collaborative channel, presence, and the device
relay over Postgres `LISTEN/NOTIFY` (no Redis or other broker), and the per-note line order is
serialised across replicas with a Postgres advisory lock, so edits made on different instances
converge without lost updates (see `src/bus.md`). WebSocket connections can land on any replica.

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

## License

Licensed under the [GNU Affero General Public License v3.0 or later](LICENSE) (AGPL-3.0-or-later).
