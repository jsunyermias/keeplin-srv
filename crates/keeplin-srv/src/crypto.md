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

## Related files

- `src/store.rs` — the only caller of `encrypt`/`decrypt` (write/read choke point).
- `src/reencrypt.rs` — the one-off pass that migrates pre-key plaintext rows.
- `src/config.rs` — `AT_REST_KEY` loading; `src/main.rs` validates the key at startup.
- `SECURITY.md` — where at-rest encryption sits in the threat model.
