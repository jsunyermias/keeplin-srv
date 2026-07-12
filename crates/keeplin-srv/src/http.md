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
| `version` | `GET /version` | `{ name, version, protocol_version, capabilities[] }` — a client negotiates behaviour instead of guessing (issues #39/#114); never rate-limited |
| `metrics` | `GET /api/metrics` | row counts + live session/connection numbers (**requires a valid token** — issue #22) |
| `register` | `POST /api/register` | `{email, password, display_name?}`; 409 on dup email; min 8-char password |
| `login` | `POST /api/login` | verifies password, creates a device, returns a token |
| `create_device` / `list_devices` | `/api/devices` | add a device (returns its token) / list |
| `delete_device` | `DELETE /api/devices/:id` | revokes that device's token immediately |
| `delete_all_devices` | `DELETE /api/devices` | revoke **all** the caller's devices — sign out everywhere (issue #31) |
| `change_password` | `POST /api/account/password` | `{current_password, new_password}`; verifies current, min 8-char new (issue #31). Existing JWTs stay valid — follow with `DELETE /api/devices` to also sign out everywhere |
| `delete_account` | `DELETE /api/account` | `{password}`; verifies the current password, then deletes the user row. Every owned entity (devices, notes, notebooks, tags, resources, shares, journal) cascades away — irreversible (issue #31) |
| `create_note` / `list_notes` | `/api/notes` | create (Inbox by default) / owned + shared. `GET` takes optional `?limit=&cursor=` (issue #29): a bare array as before, plus an `X-Next-Cursor` response header when a full page is returned — follow it to page. Omitting `limit` returns everything (back-compatible); `limit` is capped at 500 |
| `get_note` | `GET /api/notes/:id` | returns metadata **plus the materialised body** |
| `update_note` / `delete_note` | `PATCH`/`DELETE` | metadata patch (needs `write`; a move into a notebook additionally needs `write` on the **destination** notebook, since the note adopts its grants; a `notebook_id` of `null` **or the nil UUID** is a move to the Inbox — keeplin-core models the Inbox as the nil uuid — with no destination check and no cascade) / owner-only soft delete |
| `create_share` / `list_shares` / `delete_share` | `/api/notes/:id/share…` | grant `{user_id\|user_email, capabilities}` (needs `share_write`, capped to the granter's own caps); list (needs `share_read`); revoke (needs `share_write`, or self) |
| `transfer_ownership` | `/api/notes/:id/transfer` | owner-only; `{user_id\|user_email}` — moves `owner_id`, drops any share row for the new owner |
| `note_history` / `notebook_history` | `GET /api/{notes,notebooks}/:id/history?limit=` | **per-entity** past versions, newest first: for a server-materialised note/notebook every user with **read access** sees every collaborator's edits (issue #27); a relay-only entity is private to the account (read per-user). `[{ timestamp, device_id, entity? }]`, `entity` null = tombstone. `limit` defaults to 100, capped at 10 000; bounded by `CHANGES_RETENTION_DAYS` and, under `HISTORY_VISIBILITY=access`, by the collaborator's access-grant time |
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

## Related files

- `auth.md` — the middleware and token issuance.
- `permissions.md` — the capability model + `resolve_note_access` used by note/share handlers.
- `store.md` — every query these handlers run.
- `ratelimit.md` — the layer applied to all routes but `/health` and `/ready`.
