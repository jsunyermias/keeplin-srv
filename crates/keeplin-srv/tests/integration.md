# `tests/integration.rs` — device relay tests (real `DbBackend`)

## What is tested

End-to-end tests of the `/api/sync` device relay driven by the **real client**: keeplin-core's
`DbBackend` (a dev-dependency) speaking the genuine wire protocol — the `auth` handshake on
construction, `send_changes` envelopes, `receive_changes` draining — through a `keeplin-srv`
instance backed by a throwaway PostgreSQL database (`#[sqlx::test]`). This mirrors keeplin's own
`ws_sync.rs`, but against the production relay, adding what the test-only relay lacked:
authentication, persistence, and offline catch-up. Also covers the REST auth surface.

## Test cases

### Live relay

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `note_syncs_live_between_two_devices` | A creates + pushes a note | B receives it over the relay |
| `update_propagates_and_converges` | A updates the note | B converges to the new body |

### Persistence & isolation

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `device_connecting_later_receives_backlog` | B connects after A pushed | B gets the persisted backlog |
| `users_do_not_see_each_others_changes` | two different users | B never receives A's changes |
| `duplicate_batches_are_deduplicated` | push the same batch twice | B converges once; no duplication |
| `sender_never_receives_its_own_changes_back` | A pushes, A drains | A sees nothing echoed |
| `invalid_token_gets_no_data` | garbage token | connection yields no changes |

### HTTP surface

| Test function | Scenario | Expected outcome |
|---------------|----------|------------------|
| `register_login_and_device_listing` | register/login/add device/list | 409 on dup email, 401 on bad password, 2 devices listed |
| `history_endpoints_serve_versions_from_the_server_journal` | A pushes note create/edit/delete + a notebook rename, then queries `GET /api/{notes,notebooks}/:id/history` | versions newest-first with tombstone `entity: null`, `?limit=` caps the reply, another account sees an empty list |
| `notebook_history_is_visible_to_shared_collaborators` | owner shares a notebook (default `creation` visibility) | the collaborator sees the owner's **full** history |
| `history_visibility_since_access_windows_a_collaborator` | `HISTORY_VISIBILITY=access`: v1 pushed before the share, v2 after; then the owner **re-pushes the whole journal from epoch** (the reinstall scenario — fresh journal rows, fresh `received_at`, pre-access `updated_at`) | the collaborator sees only v2 both before **and after** the re-push (the window filters on the payload's own causal timestamp, so re-delivery cannot leak v1); the owner always sees everything, duplicates included |

## Fixtures and helpers

| Utility | Purpose |
|---------|---------|
| `spawn_server` | boot the router on an ephemeral port with `ConnectInfo` |
| `register` / `login` | REST account setup; `login` returns a device sync token |
| `device` | build a server-mode `DbBackend` pointed at `ws://addr/api/sync` |
| `push` / `sync_until` | push all local changes / drain-and-apply until a note converges |

## Coverage gaps

- The collaborative note channel (`/api/ws`) is covered by `tests/collab.rs`.
- keeplin-core's internal `DbBackend` behaviour (version-vector merge, offline logs) is tested
  in that crate, not here — these tests exercise the *relay*, using `DbBackend` as a faithful
  client.

## Related files

- `../src/sync.rs` — the relay under test.
- `keeplin/keeplin-core/tests/ws_sync.md` — the client-side sibling suite.
