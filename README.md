# Keeplin Server

The sync server for [Keeplin](https://github.com/jsunyermias/keeplin): the
production relay that `keeplin-daemon`'s server mode (`DbBackend`) needs and
that the main repo does not ship («*No production sync server ships in this
repo*»).

Written in Rust (axum + PostgreSQL). Implements exactly the WebSocket protocol
that `keeplin-core` speaks:

1. The daemon connects and sends the handshake `{"type":"auth","token":"<jwt>"}`.
2. It pushes batches `{"type":"changes","batch_id":…,"device_id":…,"changes":[Change…]}`.
3. The server delivers batches `{"type":"changes","changes":[Change…]}` — first
   the *backlog* that device has not seen yet and then, live, the batches from
   the user's other devices. The sender is never echoed.

`Change` values are treated as **opaque JSON**: the relay persists and forwards
them without interpreting `keeplin-core`'s model, so client model evolution never
requires server migrations. Conflict resolution (version vectors) happens on the
clients, which apply every change idempotently; that is why the relay prefers
duplicate delivery over loss.

## Guarantees

- **Durability**: every accepted batch is saved to the journal (`changes`) before
  fan-out. A device that was offline receives the full backlog on reconnect.
- **Per-device cursor**: each device has a durable delivery mark that only
  advances after a successful send.
- **Retry deduplication**: `(batch_id, batch_index)` is unique; resending a batch
  after a reconnect does not duplicate rows.
- **User isolation**: changes only travel between devices owned by the same
  account.

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

## Connecting a keeplin-daemon

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

The token identifies both the user **and** the device: the relay uses it to know
what each device has already received and to avoid echoing its own changes back.
One login per device.

## API

- `GET /health`
- `POST /api/register` — `{ email, password }`
- `POST /api/login` — `{ email, password, device_name }` → `{ token, device_id }`
- `POST /api/devices` — `{ device_name }` (Bearer token) → token for another device
- `GET /api/devices` — list the account's devices (Bearer token)
- `GET /api/sync` — WebSocket sync channel (`auth` handshake as first frame)

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `3000` | HTTP/WS port |
| `DATABASE_URL` | — (required) | PostgreSQL connection string |
| `JWT_SECRET` | development value | Token signing secret; change it |
| `TOKEN_TTL_DAYS` | `365` | Device token lifetime |
| `CHANGES_RETENTION_DAYS` | `0` (disabled) | Journal pruning: deletes changes older than N days **already delivered to all of the user's devices** |
| `RUST_LOG` | `info` | Log level |

In production terminate TLS in a reverse proxy and use `wss://` — the handshake
token travels in plaintext inside the WebSocket.

## Tests

```bash
export DATABASE_URL=postgres://keeplin:keeplin@127.0.0.1:5432/keeplin
cargo test
```

Integration tests use `sqlx::test` (temporary databases) and exercise the server
with the **real client**: two `DbBackend` instances from `keeplin-core` speaking
the genuine protocol, including deferred delivery, user isolation and invalid-token
rejection.

## History

The first iteration of this repo was a line-based collaborative server with its
own protocol; it was replaced by this relay so the server speaks exactly the
`keeplin-core` protocol instead of inventing a new one. The earlier TypeScript
version lives in `legacy/`.
