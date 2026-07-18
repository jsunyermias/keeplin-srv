# `tests/integration.rs` — device relay tests (real `DbBackend`)

Self-contained companion for `crates/keeplin-srv/tests/integration.rs`. It documents
**every code block of the source file, in source order** — a reader with only this file
must be able to understand the suite without opening anything else, so project-wide
conventions are deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section
here; grep it in either direction. Each section covers **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context** (compressed for
straightforward tests).

---

## Overview

**Identification** — file-level block: the imports. Marker `// md:Overview`.

**What it does** — End-to-end tests of the `/api/sync` device relay driven by the
**real client** — keeplin-core's `DbBackend` speaking the genuine wire protocol (the
`auth` handshake sent on construction, `send_changes` envelopes, `receive_changes`
draining) — through a real keeplin-srv on a throwaway `#[sqlx::test]` PostgreSQL
database. Mirrors keeplin-core's own `ws_sync.rs` suite but against the production
relay, adding what the toy relay lacked: authentication, persistence, offline
catch-up. Also covers the REST account surface, history endpoints (issue #27), the
email flows (issue #49), the login lockout, email normalisation (issue #43), the
body-size cap (issue #44), at-rest encryption (keeplin#110) and pagination
(issue #29).

**Dependencies** — keeplin-core (`DbBackend`, models, repository/sync traits),
`keeplin_srv` (`Config`, `router`, `AppState`, `bus::spawn`), `axum` (the mock mail
webhook), `reqwest`, `sqlx`, `tempfile`, `serde_json`, `uuid`, `chrono`.

**Used by** — `cargo test`; CI.

**Repeated context** — Coverage split: the collaborative channel is covered by
`tests/collab.rs`; keeplin-core's internal `DbBackend` behaviour is tested in its own
crate — these tests exercise the **relay**, using `DbBackend` as a faithful client.

---

## Helpers

Each helper is one block with its own marker; compressed entries:

### fn test_config

Marker `// md:fn test_config`. The standard test `Config` literal. **Used by**
`spawn_server`, `spawn_instance` and the tests that tweak a knob before
`spawn_server_with_config`.

### fn spawn_server

Marker `// md:fn spawn_server`. Real router on an ephemeral port with
`ConnectInfo`; no bus (single instance).

### fn spawn_instance

Marker `// md:fn spawn_instance`. Same, **plus `bus::spawn`** — a bus-enabled
instance for the cross-instance relay test (issue #45).

### fn register / fn login

Markers `// md:fn register`, `// md:fn login`. REST account setup (register asserts
200); `login` returns the device sync token.

### fn device

Marker `// md:fn device`. A server-mode `DbBackend` (the real keeplin client) on a
leaked temp SQLite file, pointed at `ws://addr/api/sync` — its constructor performs
the `auth` handshake.

### fn epoch

Marker `// md:fn epoch`. Unix epoch — the "everything" bound for
`get_changes_since`.

### fn push

Marker `// md:fn push`. Sends every local change of a device to the relay (no
grace sleep — tests that need persistence add their own waits/polls).

### fn sync_until

Marker `// md:fn sync_until`.
`async fn sync_until(dev, id, want_body: Option<&str>) -> bool` — up to 50 rounds of
`receive_changes` (each drains ~100 ms) + `apply_change`, until note `id` exists
and (when given) its body matches. The workhorse convergence poll; also used
negatively (isolation tests assert it returns `false`).

---

## Relay tests

### fn note_syncs_live_between_two_devices

Marker `// md:fn note_syncs_live_between_two_devices`. A creates + pushes a note;
device B (same account) receives it live through the relay with title and body
intact.

### fn relay_batch_propagates_across_instances

Marker `// md:fn relay_batch_propagates_across_instances`. Two **bus-enabled**
instances, A's device on one and B's on the other: a batch pushed to instance A is
delivered **live** to B on instance B via the `sync_batch` NOTIFY wake (issue #45)
— not just on reconnect.

### fn update_propagates_and_converges

Marker `// md:fn update_propagates_and_converges`. After the create syncs, A's
update (new title/body/timestamp) converges on B (`sync_until` with the v2 body).

### fn device_connecting_later_receives_backlog

Marker `// md:fn device_connecting_later_receives_backlog`. A pushes; only then
does B log in and connect: the note arrives from the **journal** (offline
catch-up), not live fan-out.

### fn users_do_not_see_each_others_changes

Marker `// md:fn users_do_not_see_each_others_changes`. Two different accounts:
B never receives A's changes (per-user journal/fan-out isolation) —
`sync_until` must come back `false`.

### fn duplicate_batches_are_deduplicated

Marker `// md:fn duplicate_batches_are_deduplicated`. A pushes its identical local
journal twice (two envelopes); B still converges to exactly one note — journal
dedup by `(user, batch_id, index)` plus the client's idempotent `apply_change`.

### fn sender_never_receives_its_own_changes_back

Marker `// md:fn sender_never_receives_its_own_changes_back`. After pushing, A's
`receive_changes` drains empty — echo suppression by origin device id.

### fn invalid_token_gets_no_data

Marker `// md:fn invalid_token_gets_no_data`. A garbage token "connects" (the WS
upgrade succeeds) but the server closes after the failed handshake: no changes ever
arrive.

---

## REST surface tests

### fn register_login_and_device_listing

Marker `// md:fn register_login_and_device_listing`. Duplicate registration → 409;
`POST /api/devices` with an existing token mints a second device+token; the
listing shows 2; wrong password → 401.

### fn history_endpoints_serve_versions_from_the_server_journal

Marker `// md:fn history_endpoints_serve_versions_from_the_server_journal`.
A authors note create/edit/delete plus a notebook rename and pushes; polls
`GET /api/notes/:id/history` until the batch lands (journaling is async after
`send_changes`). Asserts: newest-first `[tombstone (entity: null), v2, v1]`, each
stamped with the sync device id; `?limit=1` caps the reply; notebook history
mirrors the shape. Cross-account: the relay-only **note** is private (B's read is
scoped to B's own empty journal → `[]`), while the **materialised** notebook is
access-gated (B → 403).

### fn password_change_and_logout_everywhere

Marker `// md:fn password_change_and_logout_everywhere`. Issue #31: wrong current
password → 401; correct change works; old password stops logging in, new one
works; `DELETE /api/devices` revokes the caller's own token immediately (next
request → 401).

### fn delete_account_requires_password_and_cascades

Marker `// md:fn delete_account_requires_password_and_cascades`. Issue #31: wrong
password → 401 and the account survives; correct password deletes it — the token
dies (device row cascaded), and the email is registrable afresh (user row +
unique email gone).

### fn list_notes_paginates_with_cursor

Marker `// md:fn list_notes_paginates_with_cursor`. Issue #29: 7 notes, pages of
3 following `X-Next-Cursor` → exactly 3 pages, every note exactly once (the id
tiebreaker keeps the keyset walk total under `updated_at` ties); no `limit` →
full list and no header; a garbage cursor → 400.

### fn metrics_render_prometheus_format

Marker `// md:fn metrics_render_prometheus_format`. `?format=prometheus` renders
the text exposition (content-type `text/plain`, `# TYPE keeplin_users gauge`,
`keeplin_users 1`); the default stays JSON.

---

## Email-flow tests (issue #49)

### fn spawn_mail_webhook

Marker `// md:fn spawn_mail_webhook`. Helper: a mock of the operator's mail
webhook — an in-process axum route capturing every posted payload into a shared
`Vec` (the "inbox").

### fn webhook_token

Marker `// md:fn webhook_token`. Helper: poll the captured payloads for the most
recent token of a given `kind` (bounded), panicking if none arrives.

### fn email_verification_and_password_reset_flows

Marker `// md:fn email_verification_and_password_reset_flows`. The full delegated
lifecycle with `EMAIL_VERIFICATION_REQUIRED`: registration fires the verification
mail; unverified login → 400 even with the right password; confirming the token
(unauthenticated — the token is the proof) unlocks login. Reset: request →
uniform 200, webhook receives the token; confirm sets the new password, **revokes
every device** (pre-reset token → 401) and old password → 401 / new → 200; a
consumed token cannot replay (400); an unknown email gets the same uniform 200
with **no** mail sent (no oracle — inbox length unchanged).

### fn email_flows_answer_501_when_unconfigured

Marker `// md:fn email_flows_answer_501_when_unconfigured`. Without
`MAIL_WEBHOOK_URL`, reset-request and verify-request answer 501 — the explicit
deferral.

---

## Hardening tests

### fn login_lockout_blocks_brute_force

Marker `// md:fn login_lockout_blocks_brute_force`. `LOGIN_MAX_FAILURES=3`,
2-second lockout: three 401s arm the lock; then even the **correct** password →
429; after expiry the correct password works and clears the counter (a single new
failure is a plain 401 again); an unknown email accumulates identically (same
401s, same 429) — lockout is not an existence oracle (issue #32).

### fn email_is_normalized_and_validated

Marker `// md:fn email_is_normalized_and_validated`. Issue #43: register with
mixed case + whitespace; lowercase login works; a case-variant re-registration →
409 (same account); a malformed email → 400.

### fn oversized_note_body_is_refused

Marker `// md:fn oversized_note_body_is_refused`. Issue #44: with a 32-byte
`max_note_body_bytes`, a small note reads fine while a 100-char note's read →
413 (the body is measured before being built).

### fn note_content_is_encrypted_at_rest

Marker `// md:fn note_content_is_encrypted_at_rest`. keeplin#110: with
`AT_REST_KEY` set, the API returns plaintext transparently while the raw
`notes.title` / `lines.content` columns hold `enc:v1:` ciphertext containing no
plaintext substrings.

---

## History-visibility tests (issue #27)

### fn spawn_server_with_config

Marker `// md:fn spawn_server_with_config`. Helper: `spawn_server` with a custom
`Config` (used by the lockout/cap/encryption/email/visibility tests).

### fn notebook_history

Marker `// md:fn notebook_history`. Helper: authenticated
`GET /api/notebooks/:id/history` returning the version array.

### fn notebook_history_is_visible_to_shared_collaborators

Marker `// md:fn notebook_history_is_visible_to_shared_collaborators`. Default
(`creation`) policy: A materialises + renames a notebook, shares it with B
(capability 1 = read; the share POST is polled because materialisation is async).
B sees the owner's **two** versions — history is per-entity, not per-user.

### fn history_visibility_since_access_windows_a_collaborator

Marker `// md:fn history_visibility_since_access_windows_a_collaborator`. With
`HISTORY_VISIBILITY=access`: v1 pushed **before** the share, v2 after (with a
fresh honest `updated_at`); B sees only v2. Then the **reinstall/re-push
loophole**: A re-pushes its whole journal from epoch — new journal rows, fresh
`received_at`, pre-access causal `updated_at`. The owner's unwindowed view grows
(duplicates included, v1 visible), but B **still** sees only v2 — the window
filters on the payload's own causal timestamp, so re-delivery cannot leak
pre-access versions (the honest-client boundary, `SECURITY.md`).

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of
the navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1;
refresh with `graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `spawn_server()` / `spawn_instance()` / `spawn_server_with_config()` — defined here (EXTRACTED)
- `test_config()`, `register()`, `login()`, `device()`, `epoch()`, `push()`, `sync_until()`, `spawn_mail_webhook()`, `webhook_token()`, `notebook_history()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/config.rs` — runtime configuration (EXTRACTED: references×2; e.g. `Config`)
- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: calls×3; e.g. `router()`)

**Direct dependents** (files whose symbols reference this one)

- (none in the graph) (EXTRACTED)

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | imports | `// md:Overview` |
| 2 | `fn test_config` | `// md:fn test_config` |
| 3 | `fn spawn_server` | `// md:fn spawn_server` |
| 4 | `fn spawn_instance` | `// md:fn spawn_instance` |
| 5 | `fn register` | `// md:fn register` |
| 6 | `fn login` | `// md:fn login` |
| 7 | `fn device` | `// md:fn device` |
| 8 | `fn epoch` | `// md:fn epoch` |
| 9 | `fn push` | `// md:fn push` |
| 10 | `fn sync_until` | `// md:fn sync_until` |
| 11 | `fn note_syncs_live_between_two_devices` | `// md:fn note_syncs_live_between_two_devices` |
| 12 | `fn relay_batch_propagates_across_instances` | `// md:fn relay_batch_propagates_across_instances` |
| 13 | `fn update_propagates_and_converges` | `// md:fn update_propagates_and_converges` |
| 14 | `fn device_connecting_later_receives_backlog` | `// md:fn device_connecting_later_receives_backlog` |
| 15 | `fn users_do_not_see_each_others_changes` | `// md:fn users_do_not_see_each_others_changes` |
| 16 | `fn duplicate_batches_are_deduplicated` | `// md:fn duplicate_batches_are_deduplicated` |
| 17 | `fn sender_never_receives_its_own_changes_back` | `// md:fn sender_never_receives_its_own_changes_back` |
| 18 | `fn invalid_token_gets_no_data` | `// md:fn invalid_token_gets_no_data` |
| 19 | `fn register_login_and_device_listing` | `// md:fn register_login_and_device_listing` |
| 20 | `fn history_endpoints_serve_versions_from_the_server_journal` | `// md:fn history_endpoints_serve_versions_from_the_server_journal` |
| 21 | `fn password_change_and_logout_everywhere` | `// md:fn password_change_and_logout_everywhere` |
| 22 | `fn delete_account_requires_password_and_cascades` | `// md:fn delete_account_requires_password_and_cascades` |
| 23 | `fn list_notes_paginates_with_cursor` | `// md:fn list_notes_paginates_with_cursor` |
| 24 | `fn metrics_render_prometheus_format` | `// md:fn metrics_render_prometheus_format` |
| 25 | `fn spawn_mail_webhook` | `// md:fn spawn_mail_webhook` |
| 26 | `fn webhook_token` | `// md:fn webhook_token` |
| 27 | `fn email_verification_and_password_reset_flows` | `// md:fn email_verification_and_password_reset_flows` |
| 28 | `fn email_flows_answer_501_when_unconfigured` | `// md:fn email_flows_answer_501_when_unconfigured` |
| 29 | `fn login_lockout_blocks_brute_force` | `// md:fn login_lockout_blocks_brute_force` |
| 30 | `fn email_is_normalized_and_validated` | `// md:fn email_is_normalized_and_validated` |
| 31 | `fn oversized_note_body_is_refused` | `// md:fn oversized_note_body_is_refused` |
| 32 | `fn note_content_is_encrypted_at_rest` | `// md:fn note_content_is_encrypted_at_rest` |
| 33 | `fn spawn_server_with_config` | `// md:fn spawn_server_with_config` |
| 34 | `fn notebook_history` | `// md:fn notebook_history` |
| 35 | `fn notebook_history_is_visible_to_shared_collaborators` | `// md:fn notebook_history_is_visible_to_shared_collaborators` |
| 36 | `fn history_visibility_since_access_windows_a_collaborator` | `// md:fn history_visibility_since_access_windows_a_collaborator` |
