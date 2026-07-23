# Graph Report - keeplin-srv  (2026-07-23)

## Corpus Check
- 107 files · ~153,062 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1644 nodes · 3507 edges · 82 communities (77 shown, 5 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 37 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `da204c1c`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- AppError
- AppState
- collab.rs
- collab.rs
- quotas.rs
- integration.rs
- keeplin-srv operator runbook
- soak.rs
- Cipher
- sync.rs
- resolve_note_access
- materialize.rs
- ratelimit.rs
- mod.rs
- auth_mw
- Mailer
- `http.rs` — the REST router and handlers
- `permissions.rs` — note capabilities
- `sync.rs` — the device sync relay
- `tests/collab.rs` — collaborative channel & hardening tests
- `tests/integration.rs` — device relay tests (real `DbBackend`)
- `Dockerfile` — reproducible server image
- [Unreleased]
- `collab.rs` — the collaborative session engine
- `ratelimit.rs` — per-IP token-bucket rate limiter
- keeplin-srv — Architecture overview
- `auth.rs` — passwords, tokens, and the auth middleware
- `src/crypto.rs` — at-rest encryption of note titles and line content
- `main.rs` — keeplin-srv entry point
- `src/reencrypt.rs` — one-off at-rest re-encrypt pass
- `store.rs` — the PostgreSQL data-access layer
- `bus.rs` — cross-instance coordination (issue #45)
- `error.rs` — the API error type
- `protocol.rs` — collaborative wire types
- `state.rs` — shared application state
- `tests/collab_client_e2e.rs` — real daemon client ↔ real server
- `tests/materialize.rs` — domain-entity materialisation tests
- `tests/quotas.rs` — per-user quota enforcement tests
- `docker-compose.yml` — Postgres + server stack
- finding.md
- `ci.yml` — continuous integration
- `0002_collab.sql` — the collaborative note model
- `0004_domain_entities.sql` — server-materialised notebooks, tags, resources
- `0005_permissions.sql` — note capability bitset
- `0013_try_timestamptz.sql` — safe text→timestamptz cast for the history access window
- `src/bin/reencrypt.rs` — `keeplin-reencrypt` CLI wrapper
- `config.rs` — runtime configuration
- `lib.rs` — keeplin-srv library root
- `tests/collab_client_resources_e2e.rs` — out-of-band resource blob e2e (real client)
- `tests/reencrypt.rs` — re-encrypt pass tests
- `0001_initial.sql` — accounts, devices, and the relay journal
- `0003_note_metadata.sql` — full note metadata
- `0006_notebook_permissions.sql` — notebook shares + cascade
- 0010 — cross-instance collaboration bus (issue #45)
- 0012 — email flows: verification + password reset (issue #49)
- 0011 — login brute-force lockout
- dr-drill.sh
- reencrypt.rs
- notes
- 0004_domain_entities.sql
- 0001_initial.sql
- 0010_collab_bus.sql
- email_tokens
- notes
- note_shares
- notebook_shares
- 0007_per_user_batch_dedup.sql
- reencrypt.rs
- notes
- 0008_changes_history_index.sql
- 0009_changes_entity_index.sql
- `src/mail.rs` — delegated email delivery (mail webhook)
- `tests/soak.rs` — multi-instance collaborative soak/load drill
- `{{lib.rs | main.rs}}` — {{crate name}} {{crate root | entry point}}
- 0004_domain_entities.sql
- `0015_resource_media_meta.sql` — plaintext media metadata on resources
- `tests/collab_client_reconnect_e2e.rs` — reconnect/rebuild e2e (real client)
- `tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries
- `scripts/check-docs.sh` — contractual-docs CI check
- CLAUDE.md
- check-docs.sh

## God Nodes (most connected - your core abstractions)
1. `AppError` - 157 edges
2. `Store` - 98 edges
3. `impl Store` - 92 edges
4. ``http.rs` — the REST router and handlers` - 83 edges
5. `AppState` - 81 edges
6. `AuthedUser` - 37 edges
7. `send()` - 36 edges
8. ``store.rs` — the PostgreSQL data-access layer` - 31 edges
9. ``collab.rs` — the collaborative session engine` - 29 edges
10. `user()` - 28 edges

## Surprising Connections (you probably didn't know these)
- `handle_msg()` --calls--> `resolve_note_access()`  [INFERRED]
  crates/keeplin-srv/src/collab.rs → crates/keeplin-srv/src/permissions.rs
- `main()` --calls--> `router()`  [INFERRED]
  crates/keeplin-srv/src/main.rs → crates/keeplin-srv/src/http.rs
- `spawn_server()` --calls--> `router()`  [INFERRED]
  crates/keeplin-srv/tests/collab_e2e_common/mod.rs → crates/keeplin-srv/src/http.rs
- `spawn_instance()` --calls--> `router()`  [INFERRED]
  crates/keeplin-srv/tests/collab.rs → crates/keeplin-srv/src/http.rs
- `spawn_rate_limited()` --calls--> `router()`  [INFERRED]
  crates/keeplin-srv/tests/collab.rs → crates/keeplin-srv/src/http.rs

## Import Cycles
- None detected.

## Communities (82 total, 5 thin omitted)

### Community 0 - "AppError"
Cohesion: 0.06
Nodes (45): AppError, Error, IntoResponse, Response, String, cascade_notebook_to_notes_tx(), incoming_wins(), replace_note_shares_from_notebook_tx() (+37 more)

### Community 1 - "AppState"
Cohesion: 0.09
Nodes (99): Bytes, AuthedUser, access_cutoff(), change_password(), create_device(), create_note(), create_notebook_share(), create_share() (+91 more)

### Community 2 - "collab.rs"
Cohesion: 0.09
Nodes (59): AtomicU64, advances_writer(), announce_presence(), apply_op(), clear_presence(), deliver_event(), deliver_presence(), handle_msg() (+51 more)

### Community 3 - "collab.rs"
Cohesion: 0.18
Nodes (55): capability_grants_enforce_hierarchy_and_escalation(), concurrent_updates_resolve_deterministically(), create_note(), deleting_a_device_revokes_its_collab_token(), deleting_a_device_revokes_its_token(), export_body(), forged_writer_is_rejected(), gc_compacts_old_tombstones() (+47 more)

### Community 4 - "quotas.rs"
Cohesion: 0.12
Nodes (30): dev_insecure(), env_parse(), is_weak_secret(), resolve_jwt_secret(), Config, Option, Self, String (+22 more)

### Community 5 - "integration.rs"
Cohesion: 0.16
Nodes (48): router(), delete_account_requires_password_and_cascades(), device(), device_connecting_later_receives_backlog(), duplicate_batches_are_deduplicated(), email_flows_answer_501_when_unconfigured(), email_is_normalized_and_validated(), email_verification_and_password_reset_flows() (+40 more)

### Community 6 - "keeplin-srv operator runbook"
Cohesion: 0.04
Nodes (45): At-rest encryption, Collaborative protocol (`GET /api/ws?token=<jwt>`), Connecting a keeplin-daemon, Device sync relay (`GET /api/sync`), Docker, Environment variables, Keeplin Server, License (+37 more)

### Community 7 - "soak.rs"
Cohesion: 0.11
Nodes (31): handle_collab_op(), handle_collab_presence(), handle_sync_batch(), Arc, Result, run(), spawn(), main() (+23 more)

### Community 8 - "Cipher"
Cohesion: 0.11
Nodes (24): Aes256Gcm, main(), parse_args(), Result, disabled_is_passthrough(), nonce_is_random_per_value(), reads_legacy_plaintext_when_enabled(), round_trips_and_tags() (+16 more)

### Community 9 - "sync.rs"
Cohesion: 0.14
Nodes (27): authenticate(), changes_frame(), deliver_backlog(), handle_incoming(), handler(), materialize(), relay_loop(), Arc (+19 more)

### Community 10 - "resolve_note_access"
Cohesion: 0.19
Nodes (5): higher_bits_imply_lower_ones(), read_alone_implies_nothing_more(), Capabilities, Self, unknown_bits_are_masked_off()

### Community 11 - "materialize.rs"
Cohesion: 0.25
Nodes (28): a_never_connected_device_does_not_block_pruning(), concurrent_notebook_edits_converge_deterministically(), deleted_resource_frees_quota_and_blob_is_purgeable(), deleting_a_notebook_removes_it_from_listings(), device(), epoch(), get_json(), login() (+20 more)

### Community 12 - "ratelimit.rs"
Cohesion: 0.14
Nodes (21): ConnectInfo, burst_then_throttle_then_refill(), disabled_always_allows(), idle_buckets_are_swept_after_the_interval(), ip(), rate_limit_mw(), Arc, Bucket (+13 more)

### Community 13 - "mod.rs"
Cohesion: 0.15
Nodes (20): CollabBackend, collab_client_writes_note_through_to_the_server(), PgPool, reconnecting_client_rebuilds_note_from_snapshot(), PgPool, resource_blob_travels_out_of_band_through_the_real_client(), PgPool, collab_device() (+12 more)

### Community 14 - "auth_mw"
Cohesion: 0.13
Nodes (21): Body, auth_mw(), create_token(), dummy_password_hash(), hash_password(), Arc, Claims, Error (+13 more)

### Community 15 - "Mailer"
Cohesion: 0.19
Nodes (9): Client, Mailer, MailKind, DateTime, Option, Result, Self, String (+1 more)

### Community 16 - "`http.rs` — the REST router and handlers"
Cohesion: 0.02
Nodes (81): CAPABILITIES, Coverage checklist, fn access_cutoff, fn change_password, fn compatible_with, fn create_device, fn create_note, fn create_notebook_share (+73 more)

### Community 17 - "`permissions.rs` — note capabilities"
Cohesion: 0.07
Nodes (26): accessors, can_* accessors, consts, Coverage checklist, fn all, fn bits, fn contains, fn empty (+18 more)

### Community 18 - "`sync.rs` — the device sync relay"
Cohesion: 0.09
Nodes (21): Constants, Coverage checklist, fn authenticate, fn changes_frame, fn deliver_backlog, fn handle_incoming, fn handler, fn join (+13 more)

### Community 19 - "`tests/collab.rs` — collaborative channel & hardening tests"
Cohesion: 0.40
Nodes (4): Coverage checklist, Graph context, Overview, `tests/collab.rs` — collaborative channel & hardening tests

### Community 20 - "`tests/integration.rs` — device relay tests (real `DbBackend`)"
Cohesion: 0.04
Nodes (45): Coverage checklist, Email-flow tests (issue #49), fn delete_account_requires_password_and_cascades, fn device, fn device_connecting_later_receives_backlog, fn duplicate_batches_are_deduplicated, fn email_flows_answer_501_when_unconfigured, fn email_is_normalized_and_validated (+37 more)

### Community 21 - "`Dockerfile` — reproducible server image"
Cohesion: 0.20
Nodes (9): `Dockerfile` — reproducible server image, Notes & gotchas, Purpose, Related files, Runtime contract, Stages, Usage, Why the runtime image is tiny (+1 more)

### Community 22 - "[Unreleased]"
Cohesion: 0.20
Nodes (9): [0.1.0], 2026-07 production-readiness audit follow-up, Added, Added, Added, Changed, Changelog, Security (+1 more)

### Community 23 - "`collab.rs` — the collaborative session engine"
Cohesion: 0.06
Nodes (34): `collab.rs` — the collaborative session engine, Constants, Coverage checklist, fn advances_writer, fn announce_presence, fn apply_op, fn broadcast, fn clear_presence (+26 more)

### Community 24 - "`ratelimit.rs` — per-IP token-bucket rate limiter"
Cohesion: 0.09
Nodes (21): Coverage checklist, fn bucket_count, fn burst_then_throttle_then_refill, fn check, fn disabled_always_allows, fn enabled, fn idle_buckets_are_swept_after_the_interval, fn ip (+13 more)

### Community 25 - "keeplin-srv — Architecture overview"
Cohesion: 0.25
Nodes (7): 1. What keeplin-srv is, 2. The data model (PostgreSQL), 3. The surfaces (request flow), 4. Collaboration in one paragraph, 5. Operability, 6. Where to read next, keeplin-srv — Architecture overview

### Community 26 - "`auth.rs` — passwords, tokens, and the auth middleware"
Cohesion: 0.13
Nodes (14): `auth.rs` — passwords, tokens, and the auth middleware, Coverage checklist, fn auth_mw, fn create_token, fn dummy_password_hash, fn from_request_parts, fn hash_password, fn verify_password (+6 more)

### Community 27 - "`src/crypto.rs` — at-rest encryption of note titles and line content"
Cohesion: 0.10
Nodes (19): Constants, Coverage checklist, `crypto.rs` — at-rest encryption of note titles and line content, fn bad_key_length_rejected, fn decrypt, fn disabled_is_passthrough, fn enabled, fn encrypt (+11 more)

### Community 28 - "`main.rs` — keeplin-srv entry point"
Cohesion: 0.22
Nodes (8): Coverage checklist, fn main, fn maintenance_loop, fn run_retention, fn shutdown_signal, Graph context, `main.rs` — keeplin-srv entry point, Overview

### Community 29 - "`src/reencrypt.rs` — one-off at-rest re-encrypt pass"
Cohesion: 0.18
Nodes (10): Coverage checklist, fn reencrypt_column, fn run, Graph context, impl Default for Options, Options, Stats, TableStats (+2 more)

### Community 30 - "`store.rs` — the PostgreSQL data-access layer"
Cohesion: 0.05
Nodes (36): Coverage checklist, fn cascade_notebook_to_notes_tx, fn decode, fn delete_ops, fn encode, fn incoming_wins, fn new, fn replace_note_shares_from_notebook_tx (+28 more)

### Community 31 - "`bus.rs` — cross-instance coordination (issue #45)"
Cohesion: 0.18
Nodes (10): `bus.rs` — cross-instance coordination (issue #45), Channel constants, Coverage checklist, fn handle_collab_op, fn handle_collab_presence, fn handle_sync_batch, fn run, fn spawn (+2 more)

### Community 32 - "`error.rs` — the API error type"
Cohesion: 0.18
Nodes (10): Coverage checklist, `error.rs` — the API error type, fn client_message, fn into_response, fn status, Graph context, impl AppError, impl IntoResponse for AppError (+2 more)

### Community 33 - "`protocol.rs` — collaborative wire types"
Cohesion: 0.12
Nodes (16): Coverage checklist, fn last_writer, Graph context, impl LineOp, LineId, CollabClientMsg, CollabServerMsg, Cursor (+8 more)

### Community 34 - "`state.rs` — shared application state"
Cohesion: 0.25
Nodes (7): Coverage checklist, fn new, Graph context, impl AppState, AppState, Overview, `state.rs` — shared application state

### Community 35 - "`tests/collab_client_e2e.rs` — real daemon client ↔ real server"
Cohesion: 0.33
Nodes (5): Coverage checklist, fn collab_client_writes_note_through_to_the_server, Graph context, Overview, `tests/collab_client_e2e.rs` — real daemon client ↔ real server

### Community 36 - "`tests/materialize.rs` — domain-entity materialisation tests"
Cohesion: 0.08
Nodes (25): Coverage checklist, fn a_never_connected_device_does_not_block_pruning, fn concurrent_notebook_edits_converge_deterministically, fn deleted_resource_frees_quota_and_blob_is_purgeable, fn deleting_a_notebook_removes_it_from_listings, fn device, fn epoch, fn get_json (+17 more)

### Community 37 - "`tests/quotas.rs` — per-user quota enforcement tests"
Cohesion: 0.11
Nodes (17): Coverage checklist, fn device, fn login, fn note_quota_blocks_creation_past_the_limit, fn note_quota_disabled_by_default, fn post_note, fn put_blob, fn quota_config (+9 more)

### Community 38 - "`docker-compose.yml` — Postgres + server stack"
Cohesion: 0.29
Nodes (6): `docker-compose.yml` — Postgres + server stack, Notes & gotchas, Purpose, Related files, Usage, What it defines

### Community 39 - "finding.md"
Cohesion: 0.29
Nodes (6): Context, Impact, Problem, Severity, Suggested fix / options, Where

### Community 40 - "`ci.yml` — continuous integration"
Cohesion: 0.29
Nodes (6): `ci.yml` — continuous integration, Notes & gotchas, Purpose, Related files, The `test` job, When it runs

### Community 41 - "`0002_collab.sql` — the collaborative note model"
Cohesion: 0.29
Nodes (6): `0002_collab.sql` — the collaborative note model, Notes & gotchas, Purpose, Related files, The model in one paragraph, What it defines

### Community 42 - "`0004_domain_entities.sql` — server-materialised notebooks, tags, resources"
Cohesion: 0.29
Nodes (6): `0004_domain_entities.sql` — server-materialised notebooks, tags, resources, How the server uses these, Notes & gotchas, Purpose, Related files, What it defines

### Community 43 - "`0005_permissions.sql` — note capability bitset"
Cohesion: 0.29
Nodes (6): `0005_permissions.sql` — note capability bitset, Capability bits, Not here yet, Purpose, Related files, What it changes

### Community 44 - "`0013_try_timestamptz.sql` — safe text→timestamptz cast for the history access window"
Cohesion: 0.29
Nodes (6): `0013_try_timestamptz.sql` — safe text→timestamptz cast for the history access window, Forward-only, Index note, Related files, What it does, Why

### Community 45 - "`src/bin/reencrypt.rs` — `keeplin-reencrypt` CLI wrapper"
Cohesion: 0.29
Nodes (6): `bin/reencrypt.rs` — `keeplin-reencrypt` CLI wrapper, Coverage checklist, fn main, fn parse_args, Graph context, Overview

### Community 46 - "`config.rs` — runtime configuration"
Cohesion: 0.12
Nodes (15): `config.rs` — runtime configuration, Coverage checklist, fn a_strong_secret_is_accepted, fn dev_insecure, fn env_parse, fn from_env, fn is_weak_secret, fn resolve_jwt_secret (+7 more)

### Community 47 - "`lib.rs` — keeplin-srv library root"
Cohesion: 0.40
Nodes (4): Coverage checklist, Graph context, `lib.rs` — keeplin-srv library root, Overview

### Community 48 - "`tests/collab_client_resources_e2e.rs` — out-of-band resource blob e2e (real client)"
Cohesion: 0.33
Nodes (5): Coverage checklist, fn resource_blob_travels_out_of_band_through_the_real_client, Graph context, Overview, `tests/collab_client_resources_e2e.rs` — out-of-band resource blob e2e (real client)

### Community 49 - "`tests/reencrypt.rs` — re-encrypt pass tests"
Cohesion: 0.15
Nodes (12): Coverage checklist, fn dry_run_reports_but_does_not_modify, fn raw_values, fn reencrypts_pre_key_rows_and_server_still_serves_plaintext, fn refuses_to_run_without_a_key, fn seed_note, fn spawn_server, fn test_config (+4 more)

### Community 50 - "`0001_initial.sql` — accounts, devices, and the relay journal"
Cohesion: 0.33
Nodes (5): `0001_initial.sql` — accounts, devices, and the relay journal, Notes & gotchas, Purpose, Related files, What it defines

### Community 51 - "`0003_note_metadata.sql` — full note metadata"
Cohesion: 0.33
Nodes (5): `0003_note_metadata.sql` — full note metadata, Notes & gotchas, Purpose, Related files, What it defines

### Community 52 - "`0006_notebook_permissions.sql` — notebook shares + cascade"
Cohesion: 0.33
Nodes (5): `0006_notebook_permissions.sql` — notebook shares + cascade, Purpose, Related files, The cascade (application code, not a trigger), What it defines

### Community 53 - "0010 — cross-instance collaboration bus (issue #45)"
Cohesion: 0.40
Nodes (4): 0010 — cross-instance collaboration bus (issue #45), `collab_events` — the op-fan-out outbox, `collab_presence` — merged presence, Concurrency

### Community 54 - "0012 — email flows: verification + password reset (issue #49)"
Cohesion: 0.40
Nodes (4): 0012 — email flows: verification + password reset (issue #49), Delegated delivery — keeplin is not a mail client, Flows, Token model

### Community 55 - "0011 — login brute-force lockout"
Cohesion: 0.50
Nodes (3): 0011 — login brute-force lockout, Design notes, Semantics

### Community 58 - "reencrypt.rs"
Cohesion: 0.02
Nodes (92): fn advance_cursor, fn append_changes, fn apply_notebook_shares_to_note, fn cascade_notebook_to_notes, fn changes_after, fn clear_login_failures, fn consume_email_token, fn count_live_notes_for_user (+84 more)

### Community 59 - "notes"
Cohesion: 0.12
Nodes (17): fn create_note, fn export_body, fn insert_op, fn join, fn recv_until, fn send, fn share, fn spawn_instance (+9 more)

### Community 60 - "0004_domain_entities.sql"
Cohesion: 0.17
Nodes (12): Capability-model tests (Front B), fn capability_grants_enforce_hierarchy_and_escalation, fn move_note, fn move_note_status, fn nil_notebook_id_patch_means_inbox_and_keeps_shares, fn note_move_requires_write_on_destination_notebook, fn note_status, fn notebook_owner_can_manage_child_notes_they_do_not_own (+4 more)

### Community 61 - "0001_initial.sql"
Cohesion: 0.20
Nodes (10): fn deleting_a_device_revokes_its_collab_token, fn deleting_a_device_revokes_its_token, fn gc_compacts_old_tombstones, fn health_and_readiness_probes, fn metrics_reports_counts, fn rate_limit_throttles_and_spares_health, fn spawn_rate_limited, fn version_endpoint_advertises_capabilities (+2 more)

### Community 62 - "0010_collab_bus.sql"
Cohesion: 0.29
Nodes (7): fn concurrent_updates_resolve_deterministically, fn join_receives_welcome_snapshot, fn move_reorders_lines, fn ops_and_presence_propagate_across_instances, fn ops_propagate_between_participants, fn stale_op_is_ignored, Protocol tests

### Community 63 - "email_tokens"
Cohesion: 0.29
Nodes (7): fn forged_writer_is_rejected, fn import_then_export_roundtrip, fn outsider_cannot_join, fn presence_shows_other_participants, fn revoking_a_share_stops_edits_mid_session, fn viewer_can_watch_but_not_edit, Permission tests

### Community 64 - "notes"
Cohesion: 0.29
Nodes (6): Purpose, Related files, Safety, `scripts/dr-drill.sh` — disaster-recovery restore drill, Usage, What it does

### Community 67 - "0007_per_user_batch_dedup.sql"
Cohesion: 0.40
Nodes (4): `0007_per_user_batch_dedup.sql` — scope batch dedup to the owning user, Purpose, Related files, What it changes

### Community 68 - "reencrypt.rs"
Cohesion: 0.33
Nodes (14): dry_run_reports_but_does_not_modify(), raw_values(), reencrypts_pre_key_rows_and_server_still_serves_plaintext(), refuses_to_run_without_a_key(), Option, PgPool, SocketAddr, String (+6 more)

### Community 69 - "notes"
Cohesion: 0.25
Nodes (7): Contract, Database & compatibility, Linked issues, Summary, Type of change, Verification, What changed

### Community 70 - "0008_changes_history_index.sql"
Cohesion: 0.33
Nodes (5): `0008_changes_history_index.sql` — per-user history indexes on the change journal, Purpose, Related files, Trade-off, What it defines

### Community 71 - "0009_changes_entity_index.sql"
Cohesion: 0.40
Nodes (4): `0009_changes_entity_index.sql` — re-index history per entity (user-agnostic), Purpose, Related files, What it changes

### Community 72 - "`src/mail.rs` — delegated email delivery (mail webhook)"
Cohesion: 0.15
Nodes (12): Coverage checklist, fn as_str, fn enabled, fn new, fn send, Graph context, impl Mailer, impl MailKind (+4 more)

### Community 73 - "`tests/soak.rs` — multi-instance collaborative soak/load drill"
Cohesion: 0.14
Nodes (13): Coverage checklist, fn editor, fn env_or, fn export_body, fn merge_vv, fn soak_two_instances_under_concurrent_editors, fn spawn_instance, fn test_config (+5 more)

### Community 74 - "`{{lib.rs | main.rs}}` — {{crate name}} {{crate root | entry point}}"
Cohesion: 0.05
Nodes (34): Configuration / key reference, Graph context, Notes & gotchas, `{{path/to/file}}` — {{what it configures / generates}}, Purpose, Related files, What it {{generates | defines | runs}}, Dependency graph (intra-crate) (+26 more)

### Community 75 - "0004_domain_entities.sql"
Cohesion: 0.33
Nodes (5): `0014_tag_system.sql` — transport-only `system` marker on tags, Forward-only, Related files, What it does, Why

### Community 76 - "`0015_resource_media_meta.sql` — plaintext media metadata on resources"
Cohesion: 0.33
Nodes (5): `0015_resource_media_meta.sql` — plaintext media metadata on resources, Forward-only, Related files, What it does, Why

### Community 78 - "`tests/collab_client_reconnect_e2e.rs` — reconnect/rebuild e2e (real client)"
Cohesion: 0.33
Nodes (5): Coverage checklist, fn reconnecting_client_rebuilds_note_from_snapshot, Graph context, Overview, `tests/collab_client_reconnect_e2e.rs` — reconnect/rebuild e2e (real client)

### Community 79 - "`tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries"
Cohesion: 0.15
Nodes (12): CONVERGE_TRIES, Coverage checklist, fn collab_device, fn login, fn register, fn spawn_server, fn test_config, fn wait_local_body (+4 more)

### Community 82 - "`scripts/check-docs.sh` — contractual-docs CI check"
Cohesion: 0.22
Nodes (8): Behaviour, Known caveat, Purpose, Refresh procedure after large refactors, Related files, `scripts/check-docs.sh` — contractual-docs CI check, What it checks, What it deliberately does NOT verify

## Knowledge Gaps
- **759 isolated node(s):** `dr-drill.sh script`, `Severity`, `Where`, `Problem`, `Impact` (+754 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **5 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `AppState` connect `AppState` to `AppError`, `collab.rs`, `collab.rs`, `quotas.rs`, `integration.rs`, `soak.rs`, `sync.rs`, `ratelimit.rs`, `auth_mw`, `Mailer`?**
  _High betweenness centrality (0.102) - this node is a cross-community bridge._
- **Why does `router()` connect `integration.rs` to `AppState`, `collab.rs`, `quotas.rs`, `reencrypt.rs`, `soak.rs`, `materialize.rs`, `mod.rs`?**
  _High betweenness centrality (0.050) - this node is a cross-community bridge._
- **Why does `AppError` connect `AppError` to `Cipher`, `AppState`, `collab.rs`, `auth_mw`?**
  _High betweenness centrality (0.046) - this node is a cross-community bridge._
- **What connects `dr-drill.sh script`, `Severity`, `Where` to the rest of the system?**
  _759 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `AppError` be split into smaller, more focused modules?**
  _Cohesion score 0.06439288043984717 - nodes in this community are weakly interconnected._
- **Should `AppState` be split into smaller, more focused modules?**
  _Cohesion score 0.0859073359073359 - nodes in this community are weakly interconnected._
- **Should `collab.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.08502939846223428 - nodes in this community are weakly interconnected._