# `src/crypto.rs` — at-rest encryption of note titles and line content

## Purpose

Optional at-rest encryption (issue keeplin#110) of the two sensitive columns the server
materialises in plaintext: `notes.title` and `lines.content`. AES-256-GCM with a fresh random
96-bit nonce per value, keyed from `AT_REST_KEY` (base64, exactly 32 bytes). This protects data
**at rest** (a DB dump, stolen backup, or SQL read access sees ciphertext); it does **not**
protect against a compromised running server or the operator — collaborative merging needs the
plaintext in server memory, so the server necessarily holds the key.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `Cipher` | struct | The at-rest cipher. Cheap to clone. Holds `Option<Aes256Gcm>`; `None` = encryption disabled (pass-through). |
| `ENC_PREFIX` | `pub const &str` | `"enc:v1:"` — the tag prefixing every encrypted stored value. Public so `src/reencrypt.rs` selects untagged rows (`NOT LIKE 'enc:v1:%'`) without duplicating the literal. |

## Public API

| Function | Description |
|----------|-------------|
| `Cipher::from_key(Option<&str>) -> Result<Self, String>` | `None`/empty disables encryption. A present-but-invalid key (bad base64, wrong length) is an **error** so the server refuses to start rather than silently storing plaintext. |
| `enabled() -> bool` | Whether a key is loaded. |
| `encrypt(&str) -> Result<String, AppError>` | Disabled → plaintext unchanged. Enabled → `enc:v1:<base64(nonce‖ciphertext)>` with a fresh random nonce (two encryptions of the same value differ). |
| `decrypt(&str) -> Result<String, AppError>` | Untagged value → returned as-is (plaintext, or pre-key row). Tagged → decrypted; wrong key / corruption is a loud `AppError::Internal`, never a silent wrong answer. |

## Stored-value format and migration invariants

- A stored value is either plaintext (no tag) or `enc:v1:<base64(12-byte nonce ‖ GCM ciphertext+tag)>`.
- **Both forms always decrypt correctly**, so enabling the key on a live database is safe: old
  rows stay readable, new writes are encrypted. The one-off migration of old rows is
  `src/reencrypt.rs` / the `keeplin-reencrypt` binary — see `RUNBOOK.md` ("Key rotation &
  re-encryption").
- An `enc:v1:` value with the key **unset** is an error (`decrypt` refuses): the deployment
  lost its key, which must be surfaced, not masked.
- The tag string is versioned (`v1`); a future algorithm change adds `enc:v2:` and keeps `v1`
  readable.
- Encryption is applied/removed **only** in `src/store.rs` (single choke point) so no handler
  can accidentally read or write the wrong form.

## Design notes

- Per-value random nonce instead of deterministic: titles/lines repeat often; deterministic
  nonces would leak equality of contents.
- Opt-in via env (unset = disabled) keeps existing deployments working with zero migration.

## Graph context

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `Cipher` — defined here (EXTRACTED; 4 cross-file edge(s))
- `.encrypt()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.decrypt()` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.from_key()` — defined here (EXTRACTED; file-local)
- `.enabled()` — defined here (EXTRACTED; file-local)
- `test_key()` — defined here (EXTRACTED; file-local)
- `disabled_is_passthrough()` — defined here (EXTRACTED; file-local)
- `round_trips_and_tags()` — defined here (EXTRACTED; file-local)
- `nonce_is_random_per_value()` — defined here (EXTRACTED; file-local)
- `reads_legacy_plaintext_when_enabled()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/error.rs` — the API error type (EXTRACTED: imports_from×1, references×2; e.g. `AppError`)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/reencrypt.rs` — one-off at-rest re-encrypt pass (EXTRACTED: references×2; e.g. `reencrypt_column()`, `run()`)
- `crates/keeplin-srv/src/store.rs` — the PostgreSQL data-access layer (EXTRACTED: references×2; e.g. `Store`, `.with_cipher()`)

**Invariants** (restated on purpose; a change to this file must keep these true)

- A stored value is either plaintext (untagged) or `enc:v1:<base64(nonce‖ciphertext)>`; both must always decrypt correctly so the key can be enabled on a live database.
- Fresh random 96-bit nonce per value — two encryptions of the same plaintext must differ.
- A present-but-invalid key is a startup error (never silent plaintext); an `enc:v1:` value with no key configured is a loud decrypt error.
- The tag literal lives once as `ENC_PREFIX`; `src/reencrypt.rs` selects rows by it — do not duplicate the string.

## Related files

- `src/store.rs` — the only caller of `encrypt`/`decrypt` (write/read choke point).
- `src/reencrypt.rs` — the one-off pass that migrates pre-key plaintext rows.
- `src/config.rs` — `AT_REST_KEY` loading; `src/main.rs` validates the key at startup.
- `SECURITY.md` — where at-rest encryption sits in the threat model.
