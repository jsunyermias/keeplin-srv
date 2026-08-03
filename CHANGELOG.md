# Changelog

All notable changes to keeplin-srv are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

The wire protocol has its own version, exposed at `GET /version`
(`protocol_version`), bumped on a breaking change to the relay/collab message
shapes independently of the crate version.

## [Unreleased]

### Deterministic convergence for the review loop (keeplin ADR 0004)

- **The implementation↔review loop now terminates on a computed condition.** Previously the
  only mechanical gate was `.github/scripts/check-review-governance.js`, which never inspected
  findings: what stood for "the loop finished" was the pull-request checkbox `Blocking findings
  are resolved and conversations are closed`, an assertion by the agents inside the loop. No
  repository state held finding identity, round count or round-to-round comparison, so settled
  findings returned as new and a stalled loop was indistinguishable from a progressing one.
- **New `.github/scripts/check-review-loop.js`**, byte-identical to `keeplin`'s, now driven by the default-branch trusted evaluator. A finding blocks only when
  *reified* — named as a test, property, contract assertion or `check-docs` check that fails;
  anything not reducible to a failing check is `advisory`, recorded but not blocking.
  Convergence is required checks green **and** zero open reified findings.
- **Findings are identified and durable.** The new `## Review ledger` section of
  `.github/pull_request_template.md` carries a stable ID and one state per finding (`open` /
  `resolved` / `dismissed` / `advisory`). A `dismissed` finding cites the priority decision or
  accepted ADR that settles it and does not reopen when re-raised.
- **The stagnation brake measures state, not a clock.** The loop-state hash is
  `sha256(normalized diff ‖ open reified finding IDs ‖ red check names)`. A repeated hash, or a
  blocking set that has not shrunk for `REVIEW_LOOP_STAGNATION_LIMIT` rounds (3), escalates to
  the maintainer naming the exact stuck item and demands an entry in the new
  `docs/review-stalls.md`.
- **Round 1 of independent review (Codex / GPT-5.5) found six reifiable defects; five are
  fixed here and one is open.** Convergence now runs in its own `converge` job gated on
  `needs: [test, graph]`, because a step inside `Check, Test & Lint` asserted "required checks
  are green" before `cargo test`, Clippy, audit and the graph job had run; an unfinished check
  is now reported as `awaiting-checks` and blocks, rather than being read as green. A body with
  no ledger section is round zero per the ADR's migration contract, not malformed. A stall
  record must now name every blocker in the `## Open` table, not merely mention the pull
  request somewhere in the file. The loop-state hash joins with `\x1f` instead of a comma,
  which had made `{"a,b","c"}` and `{"a","b,c"}` collide — check-run names contain commas, and
  this repository's own is `Check, Test & Lint`. Escaped pipes in ledger cells no longer shift
  the state column.
- **Round 1 left one finding open and blocking: the stagnation brake read its own history from
  the editable pull-request body**, so deleting `Round log` rows reset the streak. Closing it
  needed loop state persisted where an agent cannot rewrite it, which crossed ADR 0004's
  recorded compatibility note and so awaited a maintainer decision. It was reified as a failing
  test rather than reclassified as advisory, and is closed by the ADR 0008 entry below.
- **Review round 2 (Codex / GPT-5.5) reopened four round-1 findings and added three.** The
  convergence check now takes `needs.test.result`/`needs.graph.result` as positive evidence
  instead of inferring greenness from the check-run API, so skipped, neutral, absent and
  unknown no longer read as green. Blocker matching requires explicit delimited tokens
  (`F-0010` is not `F-001`), the loop-state hash is SHA-256 over canonical JSON rather than
  delimiter framing, and table parsing follows CommonMark backslash parity. ADR 0005 is
  rejected; ADR 0006 proposed a trusted default-branch writer and stayed unimplemented while
  proposed, so F-002/F-008/F-009 stayed open behind a deliberately red test until the accepted
  ADR 0008 closed them. The suite is green again as of that entry.
- **ADR 0008 implemented.** The head-controlled `converge` job is gone; the authoritative
  evaluator is the default-branch `review-loop-evaluator.yml`, which never checks out or
  executes pull-request content. A finding reaches `resolved`/`dismissed` only against a
  maintainer authorization naming the finding and target state, with resolution evidence
  bound to the evaluated head, workflow and App. Every pull-request workflow is read-only,
  proven by a canary that fails the build unless `PATCH /check-runs` returns 403.
  Terminal journal truncation stays undetected and is stated as such; the three-probe
  follow-up is tracked in `docs/review-loop-spike.md`.
- **Round 9 gave this repository its first full independent review (Kimi K3), and Codex's
  round-9 review of `keeplin` found F-018 in the mirrored evaluator.** An authorized tombstone
  retired a finding ID without reserving it, so the ID could return as `advisory` against a
  newest record that no longer mentioned it, request no authorization and converge. Reification
  is now remembered across every surviving journal record; the truncation bound is unchanged.
  This repository's own review found only documentation drift, corrected here: the `ci.yml`
  companion described the removed `converge` job and a superseded hash construction, omitted the
  canary step and claimed a deliberately red test that no longer exists.
- **F-020 is open and blocking, awaiting a maintainer decision.** `check-review-governance.js`
  runs inside the head-controlled `ci.yml`, so the trusted evaluator has no evidence it ran.
  keeplin ADR 0009 proposes moving governance into the default-branch evaluator and stays
  unimplemented while proposed.
- Independent review is untouched. `ci.yml` is read-only; only the trusted
  default-branch evaluator holds write scopes. No server behavior, migration, wire surface or `keeplin-core` pin is
  affected.

### Graphify graph moved to a CI artifact (keeplin#148)

- `graphify-out/` is no longer versioned. CI generates it with `graphifyy==0.9.25`,
  validates the focused corpus and same-tree reproducibility, then publishes
  `knowledge-graph-<commit SHA>` for 14 days.
- `.graphifyignore` excludes companions, templates and generated/build/vendor trees while
  retaining the selected architecture, security and ADR documents. The former pre-commit
  auto-refresh hook was removed because commits no longer carry generated graph files.

### Hard format limits imported from keeplin-core (keeplin#130)

- **The limits are no longer this crate's to define.** `src/collab.rs` drops its local
  `MAX_LINE_LEN = 10_000` / `MAX_LINES_PER_NOTE = 100_000` and imports
  `keeplin_core::format::{MAX_LINE_BYTES, MAX_LINES_PER_NOTE}` — 2¹² = 4 096 UTF‑8
  **bytes** per line and 2¹⁶ = 65 536 lines per note — together with the wire codes
  `too_long` / `too_many_lines`. keeplin-core is the single source of truth and this
  crate pins it to an exact `rev`, so server and client cannot drift. **Breaking, no
  migration**: lines and notes over the new limits are refused.
- **The lines-per-note limit now counts live lines** (`Store::count_live_lines_on`, a
  `deleted_at IS NULL` count on the advisory-locked connection) instead of
  `order.order.len()`, which included tombstones. A note whose order vector was full of
  deleted lines could otherwise be refused while the client, counting the materialised
  body, still considered it under the limit.
- **New notes-per-notebook cap**: `PATCH /api/notes/:id` refuses a move into a notebook
  already holding `keeplin_core::format::MAX_NOTES_PER_NOTEBOOK` (2²⁴) live notes with
  `413` (`Store::count_live_notes_in_notebook`). That `PATCH` is the only path by which
  a note enters a notebook server-side.
- **`CollabServerMsg::Error` gained an optional `note_id`** (`#[serde(default)]` +
  `skip_serializing_if`, so `protocol_version` is **unchanged** and old clients parse
  both shapes). Every `OpOutcome::Invalid` now names its note, which is what lets the
  client drop that note's mirror and rejoin instead of keeping an edit the server
  refused.
- **New cross-repo compatibility test** `tests/core_compat.rs`: round-trips every
  `CollabClientMsg` and `CollabServerMsg` variant (and all four `LineOp`s) against
  keeplin-core's real types in both directions, and asserts the three format limits,
  the three limit codes and `PROTOCOL_VERSION` against keeplin-core. Runs without a
  database.
- **New collab boundary tests** in `tests/collab.rs`: a 4 096-byte line is accepted and
  4 097 is rejected with `too_long` **and the note id**, without persisting; and a
  delete frees line-count capacity, pinning the live-lines counting rule.

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
- **Graphify integration (historical; graph storage superseded by keeplin#148)**: introduced a committed knowledge graph
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
