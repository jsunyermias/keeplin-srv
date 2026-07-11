# keeplin-srv — Architecture overview

This is the **one-page mental model** for `keeplin-srv`, the production sync server for
[Keeplin](https://github.com/jsunyermias/keeplin). Read this first; every source file has a
companion `.md` that drills into one piece.

---

## 1. What keeplin-srv is

The central server that a `keeplin-daemon` in **server mode** (`DbBackend`) talks to. It is
the **durable source of truth** for a user's notes and the broker for real-time
collaboration. A single Rust binary (axum + PostgreSQL) exposing three surfaces over one HTTP
port:

| Surface | Path | Purpose |
|---------|------|---------|
| **REST/JSON** | `/api/*` | accounts, devices, notes CRUD, sharing, import/export, metrics |
| **Collaborative channel** | `GET /api/ws` | real-time line editing (design §7): `Join`/`Op`/`Cursor` ↔ `Welcome`/`Op`/`Presence` |
| **Device sync relay** | `GET /api/sync` | keeplin-core's `DbBackend` wire protocol; also **materialises** notebooks/tags/resources into server tables (server = truth, client DB = cache) |

---

## 2. The data model (PostgreSQL)

Everything lives in PostgreSQL; the schema is versioned SQL migrations (`migrations/`).

- **`users`** — account (email, Argon2 password hash, `display_name`).
- **`user_devices`** — one row per device login. The **device** is the concurrency actor:
  the JWT carries `device_id`, and that id is what a device signs its edits with.
- **`notes`** — note metadata (title, notebook, to-do fields, soft-delete). The body is
  **not** stored; it is materialised from the live lines.
- **`lines`** — one row per collaborative line: an independently versioned entity
  (`content`, `vv`, `last_writer`, `deleted_at` tombstone).
- **`note_line_order`** — the versioned order of a note's lines (its own `vv`).
- **`note_shares`** / **`notebook_shares`** — who may access a note/notebook, as a **capability bitset** (`read`/`write`/`share_read`/`share_write`/`manage`, higher bits implying lower; owner is implicit and transferable). A notebook's grants **cascade destructively** onto its notes' `note_shares` (on a notebook-perm change or a note move). See `permissions.md`.
- **`changes`** + **`device_cursors`** — the relay's durable journal and per-device delivery
  watermarks for `/api/sync`.
- **`notebooks`**, **`tags`**, **`note_tags`**, **`resources`** (+ **`resource_blobs`**) — the
  domain entities the relay materialises from `Change`s, so the server (not the client) is their
  source of truth. Resolved by version vector on write; served over REST for cold rehydration.

**Conflict resolution** is by **version vectors** with a deterministic `(timestamp,
device_id)` tiebreak, reusing keeplin-core's `note_log::resolve`. No locks; every replica
converges.

---

## 3. The surfaces (request flow)

```
                          ┌──────────── rate limiter (per-IP, /health exempt) ────────────┐
  HTTP request ──▶ router │  auth middleware (JWT + device-still-exists check)  ─▶ handler │──▶ Store ──▶ PostgreSQL
                          └────────────────────────────────────────────────────────────────┘
```

- **`main.rs`** builds the pool (bounded, with timeouts), runs migrations, spawns the
  maintenance loop, and serves with graceful shutdown.
- **`http.rs`** is the router and every REST handler.
- **`auth.rs`** hashes/verifies passwords, mints/verifies JWTs, and the middleware that
  rejects a token whose device has been revoked.
- **`collab.rs`** is the collaborative session engine (`/api/ws`).
- **`sync.rs`** is the device relay (`/api/sync`).
- **`store.rs`** is the single data-access layer (all SQL lives here).
- **`state.rs`** is the shared `AppState` every handler holds.

---

## 4. Collaboration in one paragraph

A note is a **list of independently versioned lines**; the order of lines is itself a
versioned entity. Clients send `LineOp`s (`Insert`/`Update`/`Delete`/`Move`) signed with
their device id; the server validates (capability, existence, writer identity, vv advances),
resolves against current state with `note_log::resolve`, persists, and fans the applied ops
out to the note's other live subscribers with a monotonic `server_seq`. On connect a client
gets a full `Welcome` snapshot and rebuilds — there is no infinite op log. See
`crates/keeplin-srv/src/collab.md`.

---

## 5. Operability

- **Bounded pool**: `DB_MAX_CONNECTIONS` + acquire/idle/max-lifetime timeouts.
- **Graceful shutdown**: drains REST on `SIGTERM`/`Ctrl-C`, watchdog force-exits after
  `SHUTDOWN_GRACE_SECS` (collaborative WebSockets are long-lived).
- **Rate limiting**: optional per-IP token bucket (`RATE_LIMIT_PER_MIN`); `/health` exempt.
- **Metrics**: `GET /api/metrics` (aggregate counts + live sessions).
- **Maintenance loop**: hourly journal pruning (`CHANGES_RETENTION_DAYS`) and line-tombstone
  GC (`LINES_GC_DAYS`).
- **Logs**: pretty by default, JSON with `LOG_JSON=true`.

Terminate TLS at a reverse proxy (`wss://`/`https://`) — tokens travel in the `Authorization`
header (preferred) or the WS query string (fallback).

---

## 6. Where to read next

- Per-file companions: every `src/*.rs` has a `src/*.md`.
- Data layer & schema: `src/store.md` and `migrations/*.md`.
- Collaboration: `src/collab.md`, `src/protocol.md`.
- Relay: `src/sync.md`.
- Auth & permissions: `src/auth.md`, `src/permissions.md`.
- Operability: `src/main.md`, `src/ratelimit.md`, `src/config.md`.
- User-facing API and setup: `README.md`.
- The client half lives in [jsunyermias/keeplin](https://github.com/jsunyermias/keeplin)
  (`keeplin-core/src/collab/`).
