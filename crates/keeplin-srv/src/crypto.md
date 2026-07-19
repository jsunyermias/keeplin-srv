# `crypto.rs` — at-rest encryption of note titles and line content

Self-contained companion for `crates/keeplin-srv/src/crypto.rs`. It documents **every code block of
the source file, in source order, with its complete code embedded** — a reader with only this file must be able to
understand `crypto.rs` without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `crypto.rs` carries exactly one marker comment of the
form `// md:<Header> > … > <Block header>`, whose path is the header chain of the section
documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview` at
the top of the file.

**Code** — complete and verbatim:

```rust
// md:Overview
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use base64::Engine as _;

use crate::error::AppError;
```

**What it does** — Optional at-rest encryption (issue keeplin#110) of the two sensitive
columns the server materialises in plaintext: `notes.title` and `lines.content`.
Collaborative editing needs the plaintext in the server's **memory** to merge line ops,
so this does **not** make notes end-to-end encrypted; it protects the data **at rest**
in PostgreSQL. A database dump, a stolen backup, or SQL read access sees ciphertext,
not note contents. It does not defend against a compromised running server or a
malicious operator — both hold the key. Where this sits in the threat model is spelled
out in `SECURITY.md`.

Design:

- AES-256-GCM with a fresh random 96-bit nonce per value.
- The key comes from `AT_REST_KEY` (base64, exactly 32 bytes). If unset, encryption is
  **disabled** and values are stored as-is — opt-in, so an existing deployment keeps
  working with zero migration.
- A stored value is tagged `enc:v1:<base64(nonce‖ciphertext)>`; untagged values are
  plaintext. **Both forms always decrypt correctly**, so enabling the key on a running
  database is safe: old rows stay readable and new writes are encrypted. The one-off
  `keeplin-reencrypt` binary (`src/reencrypt.rs`, `src/bin/reencrypt.rs`) migrates old
  plaintext rows to `enc:v1:` — see `RUNBOOK.md` ("Key rotation & re-encryption").
- The tag is versioned (`v1`); a future algorithm change adds `enc:v2:` and keeps `v1`
  readable.

**Dependencies** — `aes_gcm` (external): AES-256-GCM AEAD, nonce generation via
`OsRng`. `base64` (external): the storage encoding. Internal: `crate::error::AppError`
(`error.rs`) for encrypt/decrypt failures.

**Used by** — `store.rs` is the **only** caller of `encrypt`/`decrypt` (single choke
point: no handler can accidentally read or write the wrong form); `state.rs` builds the
`Cipher` in `AppState::new`; `main.rs` validates the key at startup;
`reencrypt.rs` + `bin/reencrypt.rs` run the migration pass; `tests/reencrypt.rs`
exercises the whole flow end to end.

**Repeated context** — Fail-fast configuration convention: a present-but-invalid key
must abort startup (never silent plaintext); an `enc:v1:` value with the key unset is a
loud decrypt error (the deployment lost its key — surface it, don't mask it). The
per-value random nonce exists because titles/lines repeat often; a deterministic nonce
would leak equality of contents.

---

## Constants

**Identification** — logical section: the tag and encoding constants; marker
`// md:Constants`.

**Code** — complete and verbatim:

```rust
// md:Constants
pub const ENC_PREFIX: &str = "enc:v1:";
const TAG: &str = ENC_PREFIX;
const B64: base64::engine::general_purpose::GeneralPurpose =
    base64::engine::general_purpose::STANDARD;
```

**What it does** — `ENC_PREFIX` is the storage tag prefixing every encrypted value. It
is public so the re-encrypt pass (`src/reencrypt.rs`) can select the rows that still
lack it (`NOT LIKE 'enc:v1:%'`) without duplicating the literal — the string lives
exactly once. `TAG` is the module-internal alias; `B64` the standard (padded) base64
engine used for the stored blob.

**Dependencies** — `base64` (external).

**Used by** — `encrypt`/`decrypt` (this file); `reencrypt.rs` (row selection and the
rewrite loop); `tests/reencrypt.rs` (asserts every row is tagged after the pass).

**Repeated context** — Versioned-tag rule: `v1` denotes AES-256-GCM +
`base64(nonce‖ct)`; introducing `enc:v2:` must keep `v1` decryptable forever (rows
migrate forward only via the explicit re-encrypt pass).

---

## Cipher

**Identification** — struct; marker `// md:Cipher`.

**Code** — complete and verbatim:

```rust
// md:Cipher
#[derive(Clone)]
pub struct Cipher {
    cipher: Option<Aes256Gcm>,
}
```

**What it does** — The at-rest cipher handle. `None` = encryption disabled
(pass-through); `Some` = a parsed AES-256-GCM key. Cheap to clone; the `Store` holds
one and it travels with it. The field is private so the only operations are the four
methods below — callers cannot extract the key.

**Dependencies** — `aes_gcm::Aes256Gcm` (external).

**Used by** — `store.rs` (`Store` field `cipher`, set via `Store::with_cipher`),
`state.rs::AppState::new` (construction), `reencrypt.rs::run` (the pass requires an
**enabled** cipher), `bin/reencrypt.rs` (builds one from the environment),
`tests/reencrypt.rs`.

**Repeated context** — Single-choke-point rule: encryption is applied/removed **only**
in `store.rs`. The `Cipher` being embedded in the `Store` (rather than passed around)
is how that rule stays structural instead of disciplinary.

---

## impl Cipher

**Identification** — inherent impl block; marker `// md:impl Cipher`. Contains
`fn from_key`, `fn enabled`, `fn encrypt`, `fn decrypt` (next sections).

**Code** — container: members documented as sub-blocks below: fn from_key, fn enabled, fn encrypt, fn decrypt.

**What it does** — Constructor plus the pass-through-aware encrypt/decrypt pair.

**Dependencies** — `Cipher` (this file).

**Used by** — see the method sections.

**Repeated context** — none beyond the methods' own (below).

### fn from_key

**Identification** — associated function; marker `// md:impl Cipher > fn from_key`.

**Code** — complete and verbatim:

```rust
    // md:impl Cipher > fn from_key
    pub fn from_key(key: Option<&str>) -> Result<Self, String> {
        let raw = match key.map(str::trim).filter(|k| !k.is_empty()) {
            None => return Ok(Self { cipher: None }),
            Some(k) => k,
        };
        let bytes = B64
            .decode(raw)
            .map_err(|_| "AT_REST_KEY must be valid base64".to_string())?;
        if bytes.len() != 32 {
            return Err(format!(
                "AT_REST_KEY must decode to 32 bytes (got {})",
                bytes.len()
            ));
        }
        let key = Key::<Aes256Gcm>::from_slice(&bytes);
        Ok(Self {
            cipher: Some(Aes256Gcm::new(key)),
        })
    }
```

**What it does** — Builds the cipher from the optional base64 `AT_REST_KEY`.
`None`, empty, or whitespace-only → encryption disabled (`Ok`, pass-through). A
present key must be valid base64 decoding to exactly 32 bytes; anything else is an
`Err(String)` describing the problem — a **configuration error**, so the server
refuses to start rather than silently storing plaintext. The error type is `String`
(not `AppError`) because it is consumed at boot/CLI time, before an HTTP context
exists.

**Dependencies** — `base64` decode (external), `aes_gcm::Key`/`Aes256Gcm::new`
(external).

**Used by** — `main.rs` (startup validation), `state.rs::AppState::new` (with
`expect` — main already validated, so it never fires in a real boot),
`bin/reencrypt.rs`, tests (this file's `mod tests`, `tests/reencrypt.rs`).

**Repeated context** — Fail-fast configuration: present-but-invalid `AT_REST_KEY`
aborts startup. Trimming means a key of only whitespace counts as unset, matching how
operators comment out env vars.

### fn enabled

**Identification** — method; marker `// md:impl Cipher > fn enabled`.

**Code** — complete and verbatim:

```rust
    // md:impl Cipher > fn enabled
    pub fn enabled(&self) -> bool {
        self.cipher.is_some()
    }
```

**What it does** — Whether a key is loaded. Used to distinguish pass-through mode
from real encryption without exposing the key material.

**Dependencies** — none.

**Used by** — `reencrypt.rs::run` (refuses to run when disabled: "success" while
doing nothing would be a silent misfire); this file's tests.

**Repeated context** — Disabled is a **valid** production mode (encryption is
opt-in); only the re-encrypt pass treats it as an error, because its entire job
presumes a key.

### fn encrypt

**Identification** — method; marker `// md:impl Cipher > fn encrypt`.

**Code** — complete and verbatim:

```rust
    // md:impl Cipher > fn encrypt
    pub fn encrypt(&self, plaintext: &str) -> Result<String, AppError> {
        let Some(cipher) = &self.cipher else {
            return Ok(plaintext.to_string());
        };
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = cipher
            .encrypt(&nonce, plaintext.as_bytes())
            .map_err(|_| AppError::Internal("at-rest encryption failed".into()))?;
        let mut blob = nonce.to_vec();
        blob.extend_from_slice(&ct);
        Ok(format!("{TAG}{}", B64.encode(blob)))
    }
```

**What it does** — Encrypts a value for storage. Disabled → returns the plaintext
unchanged (pass-through). Enabled → generates a fresh random 96-bit nonce from
`OsRng`, AES-256-GCM-encrypts, and returns `enc:v1:<base64(nonce‖ciphertext)>` (the
12 nonce bytes concatenated before the ciphertext+GCM tag, then base64). Two
encryptions of the same plaintext differ (random nonce). Failure maps to
`AppError::Internal("at-rest encryption failed")` — in practice AES-GCM encryption
of valid inputs does not fail.

**Dependencies** — `Aes256Gcm::generate_nonce`/`encrypt` (external), `B64`/`TAG`
(this file), `AppError` (`error.rs`).

**Used by** — `store.rs` write paths for `notes.title` and `lines.content` (note
create/update, import, line insert/update); `reencrypt.rs::reencrypt_column` (the
rewrite); this file's tests.

**Repeated context** — Nonce discipline: 96-bit random nonce per value, never
reused, never derived from content — repeated titles/lines must not produce equal
ciphertexts (equality leak).

### fn decrypt

**Identification** — method; marker `// md:impl Cipher > fn decrypt`.

**Code** — complete and verbatim:

```rust
    // md:impl Cipher > fn decrypt
    pub fn decrypt(&self, stored: &str) -> Result<String, AppError> {
        let Some(rest) = stored.strip_prefix(TAG) else {
            return Ok(stored.to_string());
        };
        let cipher = self
            .cipher
            .as_ref()
            .ok_or_else(|| AppError::Internal("encrypted value but AT_REST_KEY is unset".into()))?;
        let blob = B64
            .decode(rest)
            .map_err(|_| AppError::Internal("at-rest ciphertext is not valid base64".into()))?;
        if blob.len() < 12 {
            return Err(AppError::Internal("at-rest ciphertext too short".into()));
        }
        let (nonce_bytes, ct) = blob.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let pt = cipher
            .decrypt(nonce, ct)
            .map_err(|_| AppError::Internal("at-rest decryption failed (wrong key?)".into()))?;
        String::from_utf8(pt).map_err(|_| AppError::Internal("at-rest plaintext not utf-8".into()))
    }
```

**What it does** — Decrypts a stored value. Untagged (no `enc:v1:` prefix) → returned
as-is: it is plaintext, either because encryption is disabled or because the row
predates the key (pre-migration row). Tagged → requires the key
(`AppError::Internal("encrypted value but AT_REST_KEY is unset")` otherwise — the
deployment lost its key and that must be loud), then base64-decodes, splits the
12-byte nonce from the ciphertext (short blobs rejected), AES-256-GCM-decrypts
(wrong key / corruption → loud `Internal`, never a silent wrong answer), and
UTF-8-validates the plaintext.

**Dependencies** — `TAG`/`B64` (this file), `Nonce`/`Aes256Gcm::decrypt` (external),
`AppError` (`error.rs`).

**Used by** — `store.rs` read paths for `notes.title` and `lines.content` (get/list
note, get lines, materialize, export); `reencrypt.rs::reencrypt_column` (verifying
readability during the pass); this file's tests.

**Repeated context** — Mixed-state safety: because untagged values pass through,
enabling the key on a live database is safe — old rows stay readable while new
writes are encrypted, and the re-encrypt pass migrates the remainder at the
operator's pace. GCM authentication means tampering or the wrong key is detected,
not returned as garbage.

---

## mod tests

**Identification** — `#[cfg(test)]` unit-test module; marker `// md:mod tests`. Its
test functions are the subsections below; the shared helper first.

**Code** — container: members documented as sub-blocks below: fn test_key, fn disabled_is_passthrough, fn round_trips_and_tags, fn nonce_is_random_per_value, fn reads_legacy_plaintext_when_enabled, fn wrong_key_fails_loudly, fn bad_key_length_rejected.

**What it does** — Unit tests of the cipher in isolation (no database, no server).
They pin the module's contract: pass-through when disabled, tagged+recoverable when
enabled, nonce randomness, legacy plaintext readability, loud wrong-key failure, and
key validation.

**Dependencies** — `super::*` (this file); `B64` for building test keys.

**Used by** — `cargo test` only; no runtime callers (test code by definition).

**Repeated context** — These tests are the executable form of the invariants listed
in *Overview*; a change that breaks one is a contract change, not a test problem.

### fn test_key

**Identification** — test helper; marker `// md:mod tests > fn test_key`.
`fn test_key() -> String`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn test_key
    fn test_key() -> String {
        B64.encode([7u8; 32])
    }
```

**What it does** — A fixed valid key: base64 of 32 bytes of `7`. Deterministic on
purpose — the tests need a *valid* key, not a secret one.

**Dependencies** — `B64` (this file).

**Used by** — the enabled-cipher tests below.

**Repeated context** — none.

### fn disabled_is_passthrough

**Identification** — `#[test]`; marker `// md:mod tests > fn disabled_is_passthrough`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn disabled_is_passthrough
    #[test]
    fn disabled_is_passthrough() {
        let c = Cipher::from_key(None).unwrap();
        assert!(!c.enabled());
        assert_eq!(c.encrypt("hello").unwrap(), "hello");
        assert_eq!(c.decrypt("hello").unwrap(), "hello");
    }
```

**What it does** — `from_key(None)` yields a disabled cipher: `enabled()` is false and
both `encrypt` and `decrypt` return their input unchanged.

**Dependencies** — `Cipher` (this file).

**Used by** — `cargo test`.

**Repeated context** — Pins the opt-in contract: no key, no behaviour change.

### fn round_trips_and_tags

**Identification** — `#[test]`; marker `// md:mod tests > fn round_trips_and_tags`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn round_trips_and_tags
    #[test]
    fn round_trips_and_tags() {
        let c = Cipher::from_key(Some(&test_key())).unwrap();
        let ct = c.encrypt("secret note line").unwrap();
        assert!(ct.starts_with(TAG), "stored value must be tagged");
        assert_ne!(ct, "secret note line");
        assert_eq!(c.decrypt(&ct).unwrap(), "secret note line");
    }
```

**What it does** — With a key: `encrypt` output starts with the tag, differs from the
plaintext, and `decrypt` recovers the original exactly.

**Dependencies** — `Cipher`, `TAG`, `test_key` (this file).

**Used by** — `cargo test`.

**Repeated context** — Pins the stored-value format (`enc:v1:` + recoverability).

### fn nonce_is_random_per_value

**Identification** — `#[test]`; marker
`// md:mod tests > fn nonce_is_random_per_value`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn nonce_is_random_per_value
    #[test]
    fn nonce_is_random_per_value() {
        let c = Cipher::from_key(Some(&test_key())).unwrap();
        assert_ne!(c.encrypt("x").unwrap(), c.encrypt("x").unwrap());
    }
```

**What it does** — Encrypting the same plaintext twice yields different stored
values — the equality-leak defence.

**Dependencies** — `Cipher`, `test_key` (this file).

**Used by** — `cargo test`.

**Repeated context** — Pins the fresh-random-nonce rule.

### fn reads_legacy_plaintext_when_enabled

**Identification** — `#[test]`; marker
`// md:mod tests > fn reads_legacy_plaintext_when_enabled`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn reads_legacy_plaintext_when_enabled
    #[test]
    fn reads_legacy_plaintext_when_enabled() {
        let c = Cipher::from_key(Some(&test_key())).unwrap();
        assert_eq!(
            c.decrypt("old plaintext title").unwrap(),
            "old plaintext title"
        );
    }
```

**What it does** — With a key loaded, an untagged (legacy plaintext) value decrypts
to itself: enabling the key on an existing database must not break old rows.

**Dependencies** — `Cipher`, `test_key` (this file).

**Used by** — `cargo test`.

**Repeated context** — Pins the mixed-state safety that makes key-enablement a
zero-downtime operation.

### fn wrong_key_fails_loudly

**Identification** — `#[test]`; marker `// md:mod tests > fn wrong_key_fails_loudly`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn wrong_key_fails_loudly
    #[test]
    fn wrong_key_fails_loudly() {
        let a = Cipher::from_key(Some(&B64.encode([1u8; 32]))).unwrap();
        let b = Cipher::from_key(Some(&B64.encode([2u8; 32]))).unwrap();
        let ct = a.encrypt("data").unwrap();
        assert!(b.decrypt(&ct).is_err());
    }
```

**What it does** — A value encrypted under key A fails to decrypt under key B (GCM
authentication) — an error, never silent garbage.

**Dependencies** — `Cipher`, `B64` (this file).

**Used by** — `cargo test`.

**Repeated context** — Pins the loud-failure rule for wrong key / corruption.

### fn bad_key_length_rejected

**Identification** — `#[test]`; marker `// md:mod tests > fn bad_key_length_rejected`.

**Code** — complete and verbatim:

```rust
    // md:mod tests > fn bad_key_length_rejected
    #[test]
    fn bad_key_length_rejected() {
        assert!(Cipher::from_key(Some(&B64.encode([0u8; 16]))).is_err());
        assert!(Cipher::from_key(Some("not base64!!!")).is_err());
    }
```

**What it does** — `from_key` rejects a 16-byte key and a non-base64 string — the
fail-fast configuration contract.

**Dependencies** — `Cipher`, `B64` (this file).

**Used by** — `cargo test`.

**Repeated context** — Pins "present-but-invalid key is a startup error".

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

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

## Coverage checklist

Every code block of `crypto.rs`, in source order, each documented above (five points)
and carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `ENC_PREFIX` / `TAG` / `B64` | `// md:Constants` | Constants |
| 3 | `struct Cipher` | `// md:Cipher` | Cipher |
| 4 | `impl Cipher` | `// md:impl Cipher` | impl Cipher |
| 5 | `fn from_key` | `// md:impl Cipher > fn from_key` | impl Cipher › fn from_key |
| 6 | `fn enabled` | `// md:impl Cipher > fn enabled` | impl Cipher › fn enabled |
| 7 | `fn encrypt` | `// md:impl Cipher > fn encrypt` | impl Cipher › fn encrypt |
| 8 | `fn decrypt` | `// md:impl Cipher > fn decrypt` | impl Cipher › fn decrypt |
| 9 | `mod tests` | `// md:mod tests` | mod tests |
| 10 | `fn test_key` | `// md:mod tests > fn test_key` | mod tests › fn test_key |
| 11 | `fn disabled_is_passthrough` | `// md:mod tests > fn disabled_is_passthrough` | mod tests › fn disabled_is_passthrough |
| 12 | `fn round_trips_and_tags` | `// md:mod tests > fn round_trips_and_tags` | mod tests › fn round_trips_and_tags |
| 13 | `fn nonce_is_random_per_value` | `// md:mod tests > fn nonce_is_random_per_value` | mod tests › fn nonce_is_random_per_value |
| 14 | `fn reads_legacy_plaintext_when_enabled` | `// md:mod tests > fn reads_legacy_plaintext_when_enabled` | mod tests › fn reads_legacy_plaintext_when_enabled |
| 15 | `fn wrong_key_fails_loudly` | `// md:mod tests > fn wrong_key_fails_loudly` | mod tests › fn wrong_key_fails_loudly |
| 16 | `fn bad_key_length_rejected` | `// md:mod tests > fn bad_key_length_rejected` | mod tests › fn bad_key_length_rejected |
