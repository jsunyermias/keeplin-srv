# `http.rs` — the REST router and handlers

## Purpose

Builds the axum `Router` and implements every REST/JSON handler: accounts, devices, notes
CRUD, sharing, and import/export, plus `/health` and `/api/metrics`. Wires the auth middleware
onto protected routes and the rate limiter onto everything except `/health`.

## Router shape

```
/health                         (get)   — unauthenticated, NOT rate-limited
── everything below is rate-limited (per-IP) ──
/api/metrics                    (get)   — aggregate counters
/api/register                   (post)
/api/login                      (post)  — returns { token, device_id }
── everything below also requires auth_mw (Bearer token + live device) ──
/api/devices                    (post|get)
/api/devices/:id                (delete)          — revoke a device
/api/notes                      (post|get)
/api/notes/:id                  (get|patch|delete)
/api/notes/:id/share            (post)
/api/notes/:id/share/:user_id   (delete)
/api/notes/:id/export           (get)
/api/import                     (post)
── domain entities the server materialises from the relay (read side) ──
/api/notebooks                  (get)   — live notebooks (cold rehydration)
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
| `health` | `GET /health` | returns `"ok"`; never rate-limited |
| `metrics` | `GET /api/metrics` | row counts + live session/connection numbers |
| `register` | `POST /api/register` | `{email, password, display_name?}`; 409 on dup email; min 8-char password |
| `login` | `POST /api/login` | verifies password, creates a device, returns a token |
| `create_device` / `list_devices` | `/api/devices` | add a device (returns its token) / list |
| `delete_device` | `DELETE /api/devices/:id` | revokes that device's token immediately |
| `create_note` / `list_notes` | `/api/notes` | create (Inbox by default) / owned + shared |
| `get_note` | `GET /api/notes/:id` | returns metadata **plus the materialised body** |
| `update_note` / `delete_note` | `PATCH`/`DELETE` | metadata patch / owner-only soft delete |
| `create_share` / `delete_share` | `/api/notes/:id/share…` | owner-only; `{user_id\|user_email, role}` |
| `import_note` / `export_note` | `/api/import`, `…/export` | split a flat body into versioned lines / join live lines |
| `list_notebooks` / `list_tags` / `list_resources` | `GET /api/{notebooks,tags,resources}` | live entities the server materialised from the relay (for cold rehydration) |
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

## Body materialisation

The note **body is not stored** — it is derived. `materialize_body` reads the note's line
order and lines and joins the live (non-tombstoned) lines with `\n`. `get_note` and
`export_note` both return this, so a non-collaborative client sees a normal flat note while
the server keeps the collaborative line model underneath.

## Design notes

- `/health` is deliberately outside the rate-limited sub-router so orchestrator liveness
  probes are never throttled.
- `update_note`'s `PATCH` body deserialises present-but-null fields as "clear" and absent
  fields as "unchanged" (`present` deserializer → `NotePatch`).
- Import seeds each line's version vector with the importer's **device** component, consistent
  with how collaborative ops are signed.

## Related files

- `auth.md` — the middleware and token issuance.
- `permissions.md` — role checks used by note/share handlers.
- `store.md` — every query these handlers run.
- `ratelimit.md` — the layer applied to all routes but `/health`.
