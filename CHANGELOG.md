# Changelog

All notable changes to keeplin-srv are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

The wire protocol has its own version, exposed at `GET /version`
(`protocol_version`), bumped on a breaking change to the relay/collab message
shapes independently of the crate version.

## [Unreleased]

### 2026-07 production-readiness audit follow-up

- **AT_REST_KEY re-encrypt pass**: new `keeplin-reencrypt` binary
  (`src/reencrypt.rs`) migrates pre-key plaintext `notes.title` /
  `lines.content` rows to `enc:v1:` — idempotent, batched, resumable,
  live-server safe, `--dry-run`. RUNBOOK gains a "Key rotation &
  re-encryption" section (no live rotation; key backups separate from DB
  backups); SECURITY.md updated.
- **Protocol handshake**: `compatible_with()` mirrored next to
  `PROTOCOL_VERSION` in `src/http.rs` (exact match, one place per repo);
  the pinned keeplin-core now checks `GET /version` at startup — an
  incompatible server fails the client loudly, a missing endpoint warns
  and continues. Bump procedure documented in the README.
- **HISTORY_VISIBILITY=access loophole closed**: the collaborator window
  now compares the payload's own `updated_at`/`deleted_at` (safe cast via
  migration `0013`) instead of journal `received_at`, so a reinstalled
  client re-pushing its journal from epoch can no longer leak pre-access
  versions. Residual limit (client-asserted timestamps) documented in
  SECURITY.md.
- **Docs de-staled**: README describes the actual relay/collab split
  (collab has landed; with `collab_api_url` note bodies ride `/api/ws`);
  `tests/materialize.md` no longer claims the client ships binaries in
  the `Change`. New `collab_client_resources_e2e` drives the out-of-band
  blob path through the real client (client-side upload-race fix adopted
  via the keeplin pin bump).
- **`legacy/` removed**: the dead Express+Yjs prototype (with its insecure
  default JWT secret) is deleted; git history preserves it.
- **Graphify integration**: committed knowledge graph
  (`graphify-out/graph.json` + `GRAPH_REPORT.md`), mandatory
  `## Graph context` section in every companion `.md` (dependencies /
  dependents with inline summaries + restated invariants), CI-enforced by
  `scripts/check-docs.sh`, doc templates mirrored in `docs/templates/`,
  and a README section on the two-layer (graph → companion docs)
  navigation model.

### Added
- Multi-instance soak/load drill (`tests/soak.rs`, run with `--ignored`): N concurrent
  editors across two bus-connected instances + a mid-session replica kill, asserting
  cross-instance byte-identical convergence and survivor writability.
### Added
- Prometheus text format on `GET /api/metrics?format=prometheus` (JSON stays the
  default); RUNBOOK gains monitoring/alerting guidance and a scripted, verified
  disaster-recovery drill (`scripts/dr-drill.sh`); new `SECURITY.md` documents the
  threat model, hardening checklist, and review status.
- Anti mail-bombing cap: at most 5 live (unexpired, unused) email-flow tokens per
  user and kind; excess requests are refused without flooding the inbox.
- Email flows (#49): **email verification** (`POST /api/account/verify/{request,confirm}`,
  auto-sent on registration; `EMAIL_VERIFICATION_REQUIRED` gates login) and
  **password reset** (`POST /api/account/reset/{request,confirm}`; single-use
  hashed expiring tokens, uniform responses, revokes every device on reset).
  Delivery is **delegated to the operator's mail webhook** (`MAIL_WEBHOOK_URL`)
  — keeplin never speaks SMTP; without a webhook the flows answer `501`
  (migration `0012`; new capability `email_flows`).

### Security
- Login brute-force lockout: `LOGIN_MAX_FAILURES` recent failures for an email
  answer `429` for `LOGIN_LOCKOUT_SECS` (defaults 10 / 300s; `0` disables).
  Database-backed (migration `0011`) so the counter is shared across replicas;
  uniform for existing and unknown emails (no account-existence oracle).
- Optional at-rest encryption of note titles and line content (`AT_REST_KEY`,
  AES-256-GCM), so a database dump/backup shows ciphertext, not note contents
  (keeplin#110). Opt-in and backward compatible: unset stores plaintext, and
  enabling it keeps pre-existing plaintext rows readable.
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
- **Horizontal scaling**: the collaborative channel and the device relay now work
  across multiple replicas, coordinated over Postgres `LISTEN/NOTIFY` (no new
  infrastructure). Collab ops and presence fan out to subscribers on sibling
  instances via a `collab_events` outbox + `collab_presence` table (migration
  `0010`); the relay wakes a user's devices on other instances to re-scan the
  journal. The order read-modify-write runs under a per-note advisory lock so
  concurrent edits on different replicas cannot lose an update (#45).
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
