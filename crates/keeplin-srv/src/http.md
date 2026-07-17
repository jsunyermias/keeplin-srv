# `http.rs` — the REST router and handlers

## Purpose

Builds the axum `Router` and implements every REST/JSON handler: accounts, devices, notes
CRUD, sharing, and import/export, plus `/health`, `/ready`, and `/api/metrics`. Wires the auth middleware
onto protected routes and the rate limiter onto everything except `/health`.

## Router shape

```
/health                         (get)   — liveness (unauthenticated, NOT rate-limited)
/ready                          (get)   — readiness: DB round-trip, 503 if down (unauthenticated)
/version                        (get)   — protocol version + capabilities (unauthenticated)
── everything below is rate-limited (per-IP) ──
/api/register                   (post)
/api/login                      (post)  — returns { token, device_id }
── everything below also requires auth_mw (Bearer token + live device) ──
/api/metrics                    (get)   — aggregate counters (auth required, issue #22)
/api/devices                    (post|get|delete) — add / list / revoke ALL (sign out everywhere)
/api/devices/:id                (delete)          — revoke one device
/api/account/password           (post)            — change password (needs current)
/api/account                    (delete)          — delete the account + everything it owns (needs password)
/api/notes                      (post|get)
/api/notes/:id                  (get|patch|delete)
/api/notes/:id/share            (post|get)        — grant / list shares
/api/notes/:id/share/:user_id   (delete)
/api/notes/:id/transfer         (post)            — hand ownership to another user
/api/notes/:id/history          (get)             — per-entity history for all with access (#27)
/api/notes/:id/export           (get)
/api/import                     (post)
── domain entities the server materialises from the relay (read side) ──
/api/notebooks                  (get)   — live notebooks (cold rehydration)
/api/notebooks/:id/share        (post|get)   — grant / list; grant cascades onto child notes
/api/notebooks/:id/share/:user  (delete)     — revoke; re-cascades onto child notes
/api/notebooks/:id/transfer     (post)       — hand notebook ownership to another user
/api/notebooks/:id/history      (get)        — per-entity history for all with access (#27)
/api/tags                       (get)   — live tags
/api/resources                  (get)   — live resource metadata
/api/notes/:id/tags             (get)   — live tag ids on a note
/api/resources/:id/data         (get|put) — download / streaming upload of the binary
── WebSocket surfaces (auth inside the handler) ──
/api/ws                         (get)   — collaborative channel (collab.rs)
/api/sync                       (get)   — device relay (sync.rs)
```

## Public API (handlers)

| Handler | Route | Notes |
|---------|-------|-------|
| `health` | `GET /health` | liveness: returns `"ok"`; never rate-limited |
| `ready` | `GET /ready` | readiness: DB round-trip; `200 ready` or `503` if the database is unreachable (issue #36); never rate-limited |
| `version` | `GET /version` | `{ name, version, protocol_version, capabilities[] }` — a client negotiates behaviour instead of guessing (issues #39/#114); never rate-limited. `PROTOCOL_VERSION` + `compatible_with()` (exact match) are defined **once here** and mirrored by keeplin-core's `src/compat.rs`, which enforces them at client startup (`DbBackend::new`, `CollabBackend::start`): incompatible → the client fails loudly and never syncs; missing endpoint (old server) → the client warns and continues. Bump both constants together on a breaking wire change, then bump the pinned keeplin-core `rev` in Cargo.toml and run this test suite (it drives the real client) |
| `metrics` | `GET /api/metrics` | row counts + live session/connection numbers (**requires a valid token** — issue #22) |
| `register` | `POST /api/register` | `{email, password, display_name?}`; email is normalized (lowercased/trimmed) and structurally validated (issue #43); 409 on dup email; min 8-char password |
| `login` | `POST /api/login` | normalizes the email the same way (case-insensitive), verifies password, creates a device, returns a token. Brute-force lockout: after `LOGIN_MAX_FAILURES` recent failures for an email (existing or not — no oracle), attempts get `429` for `LOGIN_LOCKOUT_SECS`; a successful login clears the counter (migration `0011`) |
| `create_device` / `list_devices` | `/api/devices` | add a device (returns its token) / list |
| `delete_device` | `DELETE /api/devices/:id` | revokes that device's token immediately |
| `delete_all_devices` | `DELETE /api/devices` | revoke **all** the caller's devices — sign out everywhere (issue #31) |
| `change_password` | `POST /api/account/password` | `{current_password, new_password}`; verifies current, min 8-char new (issue #31). Existing JWTs stay valid — follow with `DELETE /api/devices` to also sign out everywhere |
| `delete_account` | `DELETE /api/account` | `{password}`; verifies the current password, then deletes the user row. Every owned entity (devices, notes, notebooks, tags, resources, shares, journal) cascades away — irreversible (issue #31) |
| `verify_request` / `verify_confirm` | `POST /api/account/verify/{request,confirm}` | email verification (issue #49): request (auth) re-sends the token via the mail webhook (`501` unconfigured); confirm (unauth, `{token}`) stamps `email_verified_at`. Auto-sent on registration; `EMAIL_VERIFICATION_REQUIRED` refuses login for unverified accounts |
| `reset_request` / `reset_confirm` | `POST /api/account/reset/{request,confirm}` | password reset (issue #49): request (unauth, `{email}`) answers a uniform `200` whether or not the account exists (no oracle) and posts a single-use hashed expiring token to the mail webhook (`501` unconfigured); confirm (`{token, new_password}`) sets the password, revokes **every** device, and clears the login lockout |
| `create_note` / `list_notes` | `/api/notes` | create (Inbox by default) / owned + shared. `GET` takes optional `?limit=&cursor=` (issue #29): a bare array as before, plus an `X-Next-Cursor` response header when a full page is returned — follow it to page. Omitting `limit` returns everything (back-compatible); `limit` is capped at 500 |
| `get_note` | `GET /api/notes/:id` | returns metadata **plus the materialised body**; a body over `MAX_NOTE_BODY_BYTES` is refused with `413` (issue #44) |
| `update_note` / `delete_note` | `PATCH`/`DELETE` | metadata patch (needs `write`; a move into a notebook additionally needs `write` on the **destination** notebook, since the note adopts its grants; a `notebook_id` of `null` **or the nil UUID** is a move to the Inbox — keeplin-core models the Inbox as the nil uuid — with no destination check and no cascade) / owner-only soft delete |
| `create_share` / `list_shares` / `delete_share` | `/api/notes/:id/share…` | grant `{user_id\|user_email, capabilities}` (needs `share_write`, capped to the granter's own caps); list (needs `share_read`); revoke (needs `share_write`, or self) |
| `transfer_ownership` | `/api/notes/:id/transfer` | owner-only; `{user_id\|user_email}` — moves `owner_id`, drops any share row for the new owner |
| `note_history` / `notebook_history` | `GET /api/{notes,notebooks}/:id/history?limit=` | **per-entity** past versions, newest first: for a server-materialised note/notebook every user with **read access** sees every collaborator's edits (issue #27); a relay-only entity is private to the account (read per-user). `[{ timestamp, device_id, entity? }]`, `entity` null = tombstone. `limit` defaults to 100, capped at 10 000; bounded by `CHANGES_RETENTION_DAYS` (on `received_at`) and, under `HISTORY_VISIBILITY=access`, by the collaborator's access-grant time — compared against the **payload's own** `updated_at`/`deleted_at` (not journal `received_at`), so a reinstalled client re-pushing its journal from epoch cannot leak pre-access versions (honest-client boundary, see SECURITY.md) |
| `import_note` / `export_note` | `/api/import`, `…/export` | split a flat body into versioned lines / join live lines |
| `list_notebooks` / `list_tags` / `list_resources` | `GET /api/{notebooks,tags,resources}` | live entities the server materialised from the relay (for cold rehydration); paginated like `list_notes` |
| `list_note_tags` | `GET /api/notes/:id/tags` | live tag ids attached to a note |
| `get_resource_data` / `put_resource_data` | `GET`/`PUT /api/resources/:id/data` | download / upload the binary; `PUT` capped by `MAX_UPLOAD_BYTES` (413 over it), `404` if metadata is unknown, `507` if it would exceed the user's storage quota |

## Per-user quotas

Two optional quotas (both `0` = off) are enforced at their REST write point, returning `507
Insufficient Storage` (`AppError::QuotaExceeded`):

- **`MAX_NOTES_PER_USER`** — `create_note` counts the user's live notes first and refuses past the
  limit.
- **`MAX_USER_STORAGE_BYTES`** — `put_resource_data` sums the user's other resource blobs and refuses
  if adding the incoming body would exceed the limit (an overwrite is measured by its new size, not
  double-counted). Blob byte totals and note counts come from `store` (`user_blob_bytes_excluding`,
  `count_live_notes_for_user`).

## Pagination (issue #29)

The list endpoints (`/api/notes`, `/api/notebooks`, `/api/tags`, `/api/resources`) accept
`?limit=N&cursor=…`:

- The **body shape is unchanged** — always a bare JSON array — so pre-pagination clients keep
  working. Pagination is opt-in.
- Omitting `limit` returns **every** row (the old behaviour). With `limit`, at most `N` rows
  (capped at `MAX_PAGE_LIMIT = 500`) come back, and when the page is full the server sets an
  **`X-Next-Cursor`** response header. Re-request with `cursor=<that value>` to get the next page;
  the absence of the header means the list is exhausted.
- The cursor is opaque (`store::PageCursor`, `"<micros>_<uuid>"`) and drives **keyset** paging on
  `(created_at, id)` (or `(updated_at, id)` for notes), so deep pages stay cheap and the walk is
  stable under concurrent inserts. A malformed cursor is a `400`.

## Body materialisation

The note **body is not stored** — it is derived. `materialize_body` reads the note's line
order and lines and joins the live (non-tombstoned) lines with `\n`. `get_note` and
`export_note` both return this, so a non-collaborative client sees a normal flat note while
the server keeps the collaborative line model underneath.

## Design notes

- `/health` and `/ready` sit outside the rate-limited sub-router so orchestrator probes are
  never throttled. `/health` is liveness (no dependencies); `/ready` is readiness (DB round-trip).
- `update_note`'s `PATCH` body deserialises present-but-null fields as "clear" and absent
  fields as "unchanged" (`present` deserializer → `NotePatch`).
- Import seeds each line's version vector with the importer's **device** component, consistent
  with how collaborative ops are signed.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `router()` — defined here (EXTRACTED; 13 cross-file edge(s))
- `update_note()` — defined here (EXTRACTED; 6 cross-file edge(s))
- `delete_note()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `create_share()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `list_shares()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `transfer_ownership()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `create_notebook_share()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `list_notebook_shares()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `note_history()` — defined here (EXTRACTED; 5 cross-file edge(s))
- `notebook_history()` — defined here (EXTRACTED; 5 cross-file edge(s))

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/auth.rs` — passwords, tokens, and the auth middleware (EXTRACTED: references×30; e.g. `AuthedUser`)
- `crates/keeplin-srv/src/error.rs` — the API error type (EXTRACTED: references×41; e.g. `AppError`)
- `crates/keeplin-srv/src/mail.rs` — delegated email delivery (mail webhook) (EXTRACTED: references×1; e.g. `MailKind`)
- `crates/keeplin-srv/src/permissions.rs` — note capabilities (EXTRACTED: calls×15, references×1; e.g. `resolve_note_access()`, `resolve_notebook_access()`, `Access`)
- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×43; e.g. `AppState`)
- `crates/keeplin-srv/src/store.rs` — the PostgreSQL data-access layer (EXTRACTED: references×19; e.g. `PageCursor`, `User`, `UserDevice`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/collab.rs` — the collaborative session engine (EXTRACTED: calls×2; e.g. `winner()`, `line_winner()`)
- `crates/keeplin-srv/src/main.rs` — keeplin-srv entry point (EXTRACTED: calls×1; e.g. `main()`)
- `crates/keeplin-srv/tests/collab.rs` — collaborative channel & hardening tests (EXTRACTED: calls×3; e.g. `spawn_instance()`, `spawn_rate_limited()`, `spawn_server_with_state()`)
- `crates/keeplin-srv/tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries (EXTRACTED: calls×1; e.g. `spawn_server()`)
- `crates/keeplin-srv/tests/integration.rs` — device relay tests (real `DbBackend`) (EXTRACTED: calls×3; e.g. `spawn_instance()`, `spawn_server()`, `spawn_server_with_config()`)
- `crates/keeplin-srv/tests/materialize.rs` — domain-entity materialisation tests (EXTRACTED: calls×1; e.g. `spawn_server()`)
- `crates/keeplin-srv/tests/quotas.rs` — per-user quota enforcement tests (EXTRACTED: calls×1; e.g. `spawn()`)
- `crates/keeplin-srv/tests/reencrypt.rs` — re-encrypt pass tests (EXTRACTED: calls×1; e.g. `spawn_server()`)
- `crates/keeplin-srv/tests/soak.rs` — multi-instance collaborative soak/load drill (EXTRACTED: calls×1; e.g. `spawn_instance()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- Authorisation is checked in the handler **before** any data access, via the `permissions` resolvers; history/list responses never bypass them.
- `PROTOCOL_VERSION` + `compatible_with()` (exact match) are the single server-side statement of wire compatibility, mirrored by keeplin-core's `compat.rs`; bump both together.
- `/health`, `/ready`, `/version` stay outside auth and rate limiting.
- The `HISTORY_VISIBILITY=access` collaborator cutoff is passed as the payload-timestamp (`authored`) bound, never as a `received_at` bound (journal re-delivery would leak pre-access versions).
- Body-size caps (`MAX_UPLOAD_BYTES`, `MAX_NOTE_BODY_BYTES`) and per-user quotas are enforced before allocation/storage.

## Related files

- `auth.md` — the middleware and token issuance.
- `permissions.md` — the capability model + `resolve_note_access` used by note/share handlers.
- `store.md` — every query these handlers run.
- `ratelimit.md` — the layer applied to all routes but `/health` and `/ready`.
