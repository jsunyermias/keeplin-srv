# Graph Report - keeplin-srv  (2026-07-17)

## Corpus Check
- 99 files · ~77,350 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 1215 nodes · 3081 edges · 86 communities (76 shown, 10 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 37 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `a33ce1a2`
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
- login_attempts
- `src/mail.rs` — delegated email delivery (mail webhook)
- `tests/soak.rs` — multi-instance collaborative soak/load drill
- `{{lib.rs | main.rs}}` — {{crate name}} {{crate root | entry point}}
- {{Title}} — {{one-line framing}}
- `{{path/to/module.rs}}` — {{one-line purpose}}
- `{{tests/file.rs}}` — {{what it tests}}
- `tests/collab_client_reconnect_e2e.rs` — reconnect/rebuild e2e (real client)
- `tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries
- `{{path/to/file}}` — {{what it configures / generates}}
- Documentation templates (mirrored from keeplin)
- `scripts/check-docs.sh` — contractual-docs CI check
- CLAUDE.md
- check-docs.sh

## God Nodes (most connected - your core abstractions)
1. `AppError` - 157 edges
2. `Store` - 98 edges
3. `AppState` - 81 edges
4. `AuthedUser` - 37 edges
5. `send()` - 36 edges
6. `user()` - 28 edges
7. `spawn_server()` - 24 edges
8. `create_note()` - 24 edges
9. `register()` - 23 edges
10. `login()` - 23 edges

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

## Communities (86 total, 10 thin omitted)

### Community 0 - "AppError"
Cohesion: 0.06
Nodes (45): AppError, Error, IntoResponse, Response, String, cascade_notebook_to_notes_tx(), ChangeRow, CollabEvent (+37 more)

### Community 1 - "AppState"
Cohesion: 0.09
Nodes (98): Bytes, AuthedUser, access_cutoff(), change_password(), ChangePasswordBody, create_device(), create_note(), create_notebook_share() (+90 more)

### Community 2 - "collab.rs"
Cohesion: 0.09
Nodes (59): AtomicU64, advances_writer(), announce_presence(), apply_op(), clear_presence(), CollabRegistry, CollabSession, deliver_event() (+51 more)

### Community 3 - "collab.rs"
Cohesion: 0.18
Nodes (55): capability_grants_enforce_hierarchy_and_escalation(), concurrent_updates_resolve_deterministically(), create_note(), deleting_a_device_revokes_its_collab_token(), deleting_a_device_revokes_its_token(), export_body(), forged_writer_is_rejected(), gc_compacts_old_tombstones() (+47 more)

### Community 4 - "quotas.rs"
Cohesion: 0.32
Nodes (18): device(), login(), note_quota_blocks_creation_past_the_limit(), note_quota_disabled_by_default(), post_note(), put_blob(), quota_config(), register() (+10 more)

### Community 5 - "integration.rs"
Cohesion: 0.10
Nodes (60): Config, dev_insecure(), env_parse(), is_weak_secret(), resolve_jwt_secret(), Option, Self, String (+52 more)

### Community 6 - "keeplin-srv operator runbook"
Cohesion: 0.04
Nodes (45): At-rest encryption, Collaborative protocol (`GET /api/ws?token=<jwt>`), Connecting a keeplin-daemon, Device sync relay (`GET /api/sync`), Docker, Environment variables, Keeplin Server, License (+37 more)

### Community 7 - "soak.rs"
Cohesion: 0.11
Nodes (31): handle_collab_op(), handle_collab_presence(), handle_sync_batch(), Arc, Result, run(), spawn(), main() (+23 more)

### Community 8 - "Cipher"
Cohesion: 0.11
Nodes (24): Aes256Gcm, main(), parse_args(), Result, Cipher, disabled_is_passthrough(), nonce_is_random_per_value(), reads_legacy_plaintext_when_enabled() (+16 more)

### Community 9 - "sync.rs"
Cohesion: 0.14
Nodes (27): authenticate(), changes_frame(), deliver_backlog(), FanoutBatch, FanoutMsg, handle_incoming(), handler(), materialize() (+19 more)

### Community 10 - "resolve_note_access"
Cohesion: 0.13
Nodes (6): Access, Capabilities, higher_bits_imply_lower_ones(), read_alone_implies_nothing_more(), Self, unknown_bits_are_masked_off()

### Community 11 - "materialize.rs"
Cohesion: 0.25
Nodes (28): a_never_connected_device_does_not_block_pruning(), concurrent_notebook_edits_converge_deterministically(), deleted_resource_frees_quota_and_blob_is_purgeable(), deleting_a_notebook_removes_it_from_listings(), device(), epoch(), get_json(), login() (+20 more)

### Community 12 - "ratelimit.rs"
Cohesion: 0.14
Nodes (21): ConnectInfo, Bucket, burst_then_throttle_then_refill(), disabled_always_allows(), idle_buckets_are_swept_after_the_interval(), ip(), LimiterState, rate_limit_mw() (+13 more)

### Community 13 - "mod.rs"
Cohesion: 0.15
Nodes (20): CollabBackend, collab_client_writes_note_through_to_the_server(), PgPool, reconnecting_client_rebuilds_note_from_snapshot(), PgPool, resource_blob_travels_out_of_band_through_the_real_client(), PgPool, collab_device() (+12 more)

### Community 14 - "auth_mw"
Cohesion: 0.13
Nodes (21): Body, auth_mw(), Claims, create_token(), dummy_password_hash(), hash_password(), Arc, Error (+13 more)

### Community 15 - "Mailer"
Cohesion: 0.19
Nodes (9): Client, Mailer, MailKind, DateTime, Option, Result, Self, String (+1 more)

### Community 16 - "`http.rs` — the REST router and handlers"
Cohesion: 0.18
Nodes (10): Body materialisation, Design notes, Graph context, `http.rs` — the REST router and handlers, Pagination (issue #29), Per-user quotas, Public API (handlers), Purpose (+2 more)

### Community 17 - "`permissions.rs` — note capabilities"
Cohesion: 0.18
Nodes (10): Design notes, Enforcement rules, Graph context, Key types, Notebook permissions & the destructive cascade, `permissions.rs` — note capabilities, Public API, Purpose (+2 more)

### Community 18 - "`sync.rs` — the device sync relay"
Cohesion: 0.18
Nodes (10): Delivery mechanism, Design notes, Graph context, Keepalive, Key types, Materialisation (`materialize`), Purpose, Related files (+2 more)

### Community 19 - "`tests/collab.rs` — collaborative channel & hardening tests"
Cohesion: 0.18
Nodes (10): Coverage gaps, Fixtures and helpers, Graph context, Hardening, Permissions & safety, Protocol, Related files, Test cases (+2 more)

### Community 20 - "`tests/integration.rs` — device relay tests (real `DbBackend`)"
Cohesion: 0.18
Nodes (10): Coverage gaps, Fixtures and helpers, Graph context, HTTP surface, Live relay, Persistence & isolation, Related files, Test cases (+2 more)

### Community 21 - "`Dockerfile` — reproducible server image"
Cohesion: 0.20
Nodes (9): `Dockerfile` — reproducible server image, Notes & gotchas, Purpose, Related files, Runtime contract, Stages, Usage, Why the runtime image is tiny (+1 more)

### Community 22 - "[Unreleased]"
Cohesion: 0.22
Nodes (8): [0.1.0], Added, Added, Added, Changed, Changelog, Security, [Unreleased]

### Community 23 - "`collab.rs` — the collaborative session engine"
Cohesion: 0.20
Nodes (9): `collab.rs` — the collaborative session engine, Concurrency discipline, Connection flow, Design notes, Graph context, Key types, Op validation & resolution, Purpose (+1 more)

### Community 24 - "`ratelimit.rs` — per-IP token-bucket rate limiter"
Cohesion: 0.20
Nodes (9): Graph context, Key types, Memory, Notes & gotchas, Public API, Purpose, `ratelimit.rs` — per-IP token-bucket rate limiter, Related files (+1 more)

### Community 25 - "keeplin-srv — Architecture overview"
Cohesion: 0.25
Nodes (7): 1. What keeplin-srv is, 2. The data model (PostgreSQL), 3. The surfaces (request flow), 4. Collaboration in one paragraph, 5. Operability, 6. Where to read next, keeplin-srv — Architecture overview

### Community 26 - "`auth.rs` — passwords, tokens, and the auth middleware"
Cohesion: 0.22
Nodes (8): `auth.rs` — passwords, tokens, and the auth middleware, Design notes, Graph context, Key types, Public API, Purpose, Related files, Token revocation

### Community 27 - "`src/crypto.rs` — at-rest encryption of note titles and line content"
Cohesion: 0.22
Nodes (8): Design notes, Graph context, Key types, Public API, Purpose, Related files, `src/crypto.rs` — at-rest encryption of note titles and line content, Stored-value format and migration invariants

### Community 28 - "`main.rs` — keeplin-srv entry point"
Cohesion: 0.22
Nodes (8): Design notes, Graceful shutdown, Graph context, `main.rs` — keeplin-srv entry point, Maintenance loop, Purpose, Related files, Startup / wiring

### Community 29 - "`src/reencrypt.rs` — one-off at-rest re-encrypt pass"
Cohesion: 0.22
Nodes (8): Design notes, Graph context, Key types, Public API, Purpose, Related files, `src/reencrypt.rs` — one-off at-rest re-encrypt pass, The pass — batching, resumability, live-server safety

### Community 30 - "`store.rs` — the PostgreSQL data-access layer"
Cohesion: 0.22
Nodes (8): Database schema, Design notes, Graph context, Key types, Public API (by area), Purpose, Related files, `store.rs` — the PostgreSQL data-access layer

### Community 31 - "`bus.rs` — cross-instance coordination (issue #45)"
Cohesion: 0.25
Nodes (7): `bus.rs` — cross-instance coordination (issue #45), Graph context, How it works, Ordering & correctness, Purpose, Related files, Why an outbox for ops

### Community 32 - "`error.rs` — the API error type"
Cohesion: 0.25
Nodes (7): Design notes, `error.rs` — the API error type, Graph context, Key types, Public API, Purpose, Related files

### Community 33 - "`protocol.rs` — collaborative wire types"
Cohesion: 0.25
Nodes (7): Graph context, Key types, Notes & gotchas, `protocol.rs` — collaborative wire types, Purpose, Related files, The wire protocol

### Community 34 - "`state.rs` — shared application state"
Cohesion: 0.25
Nodes (7): Design notes, Graph context, Key types, Public API, Purpose, Related files, `state.rs` — shared application state

### Community 35 - "`tests/collab_client_e2e.rs` — real daemon client ↔ real server"
Cohesion: 0.25
Nodes (7): Fixtures and helpers, Graph context, Notes & gotchas, Related files, Test cases, `tests/collab_client_e2e.rs` — real daemon client ↔ real server, What is tested

### Community 36 - "`tests/materialize.rs` — domain-entity materialisation tests"
Cohesion: 0.25
Nodes (7): Coverage gaps, Fixtures and helpers, Graph context, Related files, Test cases, `tests/materialize.rs` — domain-entity materialisation tests, What is tested

### Community 37 - "`tests/quotas.rs` — per-user quota enforcement tests"
Cohesion: 0.25
Nodes (7): Fixtures and helpers, Graph context, Notes, Related files, Test cases, `tests/quotas.rs` — per-user quota enforcement tests, What is tested

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
Nodes (6): Behaviour contract, Graph context, Purpose, Related files, `src/bin/reencrypt.rs` — `keeplin-reencrypt` CLI wrapper, Usage

### Community 46 - "`config.rs` — runtime configuration"
Cohesion: 0.29
Nodes (6): `config.rs` — runtime configuration, Configuration / key reference, Graph context, Notes & gotchas, Purpose, Related files

### Community 47 - "`lib.rs` — keeplin-srv library root"
Cohesion: 0.29
Nodes (6): Design notes, Graph context, `lib.rs` — keeplin-srv library root, Module map, Purpose, Related files

### Community 48 - "`tests/collab_client_resources_e2e.rs` — out-of-band resource blob e2e (real client)"
Cohesion: 0.29
Nodes (6): Fixtures and helpers, Graph context, Related files, Test cases, `tests/collab_client_resources_e2e.rs` — out-of-band resource blob e2e (real client), What is tested

### Community 49 - "`tests/reencrypt.rs` — re-encrypt pass tests"
Cohesion: 0.29
Nodes (6): Fixtures and helpers, Graph context, Related files, Test cases, `tests/reencrypt.rs` — re-encrypt pass tests, What is tested

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
Cohesion: 0.33
Nodes (14): dry_run_reports_but_does_not_modify(), raw_values(), reencrypts_pre_key_rows_and_server_still_serves_plaintext(), refuses_to_run_without_a_key(), Option, PgPool, SocketAddr, String (+6 more)

### Community 59 - "notes"
Cohesion: 0.67
Nodes (5): lines, note_line_order, note_shares, notes, users

### Community 60 - "0004_domain_entities.sql"
Cohesion: 0.40
Nodes (5): note_tags, notebooks, resource_blobs, resources, tags

### Community 61 - "0001_initial.sql"
Cohesion: 0.70
Nodes (4): changes, device_cursors, user_devices, users

### Community 72 - "`src/mail.rs` — delegated email delivery (mail webhook)"
Cohesion: 0.22
Nodes (8): Design notes, Graph context, Key types, Public API, Purpose, Related files, `src/mail.rs` — delegated email delivery (mail webhook), Wire payload

### Community 73 - "`tests/soak.rs` — multi-instance collaborative soak/load drill"
Cohesion: 0.25
Nodes (7): Coverage gaps, Fixtures and helpers, Graph context, Phases and assertions, Related files, `tests/soak.rs` — multi-instance collaborative soak/load drill, What is tested

### Community 74 - "`{{lib.rs | main.rs}}` — {{crate name}} {{crate root | entry point}}"
Cohesion: 0.25
Nodes (8): Dependency graph (intra-crate), Design notes, Graph context, `{{lib.rs | main.rs}}` — {{crate name}} {{crate root | entry point}}, Module map, Purpose, Related files, Startup / wiring

### Community 75 - "{{Title}} — {{one-line framing}}"
Cohesion: 0.25
Nodes (7): {{1. The concept / the model}}, {{2. How it works across the system}}, {{3. Guarantees and non-guarantees}}, {{4. Operational implications}}, Related documents, {{Title}} — {{one-line framing}}, Trade-offs & rejected alternatives

### Community 76 - "`{{path/to/module.rs}}` — {{one-line purpose}}"
Cohesion: 0.25
Nodes (8): Design notes, Graph context, Key types, {{Module-specific mechanism}}, `{{path/to/module.rs}}` — {{one-line purpose}}, Public API, Purpose, Related files

### Community 77 - "`{{tests/file.rs}}` — {{what it tests}}"
Cohesion: 0.25
Nodes (8): Coverage gaps, {{Feature area}}, Fixtures and helpers, Graph context, Related files, Test cases, `{{tests/file.rs}}` — {{what it tests}}, What is tested

### Community 78 - "`tests/collab_client_reconnect_e2e.rs` — reconnect/rebuild e2e (real client)"
Cohesion: 0.29
Nodes (6): Fixtures and helpers, Graph context, Related files, Test cases, `tests/collab_client_reconnect_e2e.rs` — reconnect/rebuild e2e (real client), What is tested

### Community 79 - "`tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries"
Cohesion: 0.29
Nodes (6): Graph context, Helpers, Invariant, Purpose, Related files, `tests/collab_e2e_common/mod.rs` — shared harness for the real-client e2e binaries

### Community 80 - "`{{path/to/file}}` — {{what it configures / generates}}"
Cohesion: 0.29
Nodes (7): Configuration / key reference, Graph context, Notes & gotchas, `{{path/to/file}}` — {{what it configures / generates}}, Purpose, Related files, What it {{generates | defines | runs}}

### Community 81 - "Documentation templates (mirrored from keeplin)"
Cohesion: 0.33
Nodes (6): Documentation templates (mirrored from keeplin), House style, Placeholders in the templates, The convention in one sentence, The two-layer navigation model, Which template to use

### Community 82 - "`scripts/check-docs.sh` — contractual-docs CI check"
Cohesion: 0.33
Nodes (5): Behaviour, Purpose, Refresh procedure after large refactors, Related files, `scripts/check-docs.sh` — contractual-docs CI check

## Knowledge Gaps
- **349 isolated node(s):** `notes`, `notebooks`, `tags`, `note_tags`, `note_shares` (+344 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **10 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `AppState` connect `AppState` to `AppError`, `collab.rs`, `collab.rs`, `integration.rs`, `soak.rs`, `sync.rs`, `ratelimit.rs`, `auth_mw`, `Mailer`?**
  _High betweenness centrality (0.185) - this node is a cross-community bridge._
- **Why does `router()` connect `integration.rs` to `AppState`, `collab.rs`, `quotas.rs`, `soak.rs`, `materialize.rs`, `mod.rs`, `reencrypt.rs`?**
  _High betweenness centrality (0.092) - this node is a cross-community bridge._
- **Why does `AppError` connect `AppError` to `Cipher`, `AppState`, `collab.rs`, `auth_mw`?**
  _High betweenness centrality (0.083) - this node is a cross-community bridge._
- **What connects `notes`, `notebooks`, `tags` to the rest of the system?**
  _349 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `AppError` be split into smaller, more focused modules?**
  _Cohesion score 0.06439288043984717 - nodes in this community are weakly interconnected._
- **Should `AppState` be split into smaller, more focused modules?**
  _Cohesion score 0.09415992812219227 - nodes in this community are weakly interconnected._
- **Should `collab.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.08502939846223428 - nodes in this community are weakly interconnected._