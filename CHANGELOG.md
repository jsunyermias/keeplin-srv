# Changelog

All notable changes to keeplin-srv are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

The wire protocol has its own version, exposed at `GET /version`
(`protocol_version`), bumped on a breaking change to the relay/collab message
shapes independently of the crate version.

## [Unreleased]

### Security
- Normalize (lowercase/trim) and validate the email on register/login, share and
  transfer, so login is case-insensitive and an address maps to one account (#43).
- Collapse database/internal errors to a generic `500` body (full detail logged
  server-side) instead of returning the raw error text (#46).
- Refuse to start on a missing/weak/placeholder `JWT_SECRET`; `KEEPLIN_DEV_INSECURE=1`
  allows a weak secret for local dev only (#19).
- Revoke a deleted device's token on the collaborative WebSocket, not just REST (#20).
- Require auth for `GET /api/metrics` (#22).
- Equalise login timing for missing vs. wrong-password to close user enumeration (#32).
- Harden the example `docker-compose` (loopback Postgres, required `JWT_SECRET`) (#38).

### Added
- `MAX_NOTE_BODY_BYTES` (default 25 MiB, `0` disables): refuse to materialise a
  note body larger than the cap with `413` instead of building it in memory (#44).
- `REGISTRATION_ENABLED` to close open signups (#21).
- `RESOURCE_PURGE_DAYS`: server-side purge of deleted resource blobs (#24).
- `GET /ready` readiness probe (DB round-trip, `503` when down) + Dockerfile HEALTHCHECK (#36).
- `POST /api/account/password`, `DELETE /api/devices` (sign out everywhere), and
  `DELETE /api/account` (password-confirmed account deletion; cascades to all owned
  data) (#31).
- `HISTORY_VISIBILITY` (`creation`|`access`) visibility window for shared history (#27).
- `GET /version` capability/version handshake (#39).
- Keyset pagination on the list endpoints (`/api/notes`, `/api/notebooks`, `/api/tags`,
  `/api/resources`): opt-in `?limit=&cursor=` with an `X-Next-Cursor` header; the array
  response shape is unchanged, so old clients keep working (#29).

### Changed
- Per-user rate-limiter bucket map is swept of idle buckets (bounded memory) (#33).
- Journal batch dedup is per-user (`migration 0007`) (#26).
- Journal pruning ignores never-connected devices (#23).
- Storage quota excludes soft-deleted resources so deletes free quota (#24).
- History is **per-entity**: every user with read access sees all collaborators'
  edits; relay-only entities stay per-account (`migrations 0008`/`0009`) (#27).
- Collaborative channel re-resolves access per op (live share revocation) (#30).
- Bounded per-connection outbound queue + WebSocket keepalive/idle timeout (#34, #35).
- `gc_line_tombstones` row-locks the note order against concurrent collab writes (#25).
- `keeplin-core` pin bumped to the current keeplin `main` (v0.1.0 baseline) (#28).

## [0.1.0]

- Initial server: accounts/devices, capability-based note & notebook sharing,
  the device sync relay, the collaborative line-editing channel, server-side
  history, import/export, per-user quotas, and operational endpoints.
