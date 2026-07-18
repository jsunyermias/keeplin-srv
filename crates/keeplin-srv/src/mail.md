# `mail.rs` — delegated email delivery (mail webhook)

Self-contained companion for `crates/keeplin-srv/src/mail.rs`. It documents **every code
block of the source file, in source order** — a reader with only this file must be able to
understand `mail.rs` without opening anything else, so project-wide conventions are
deliberately re-explained here (hyper-redundancy is intended).

**How to navigate**: every block in `mail.rs` carries exactly one marker comment of the
form `// md:<Header> > … > <Block header>`, whose path is the header chain of the section
documenting it here (starting below the file title). Grep the marker text to jump
code → doc; grep the section's block name (or the marker path) in the `.rs` to jump
doc → code. Each block section covers five fixed points: **Identification**,
**What it does**, **Dependencies**, **Used by**, **Repeated context**.

---

## Overview

**Identification** — file-level block: the module's imports. Marker `// md:Overview` at
the top of the file.

```rust
use chrono::{DateTime, Utc};
```

**What it does** — Delegated email delivery (issue #49). keeplin-srv is **not a mail
client**: it never speaks SMTP and holds no provider SDK. When an email flow
(verification, password reset) needs a message delivered, the server POSTs a small JSON
payload to the operator's own mail service — the **mail webhook** — which composes and
sends the actual email with whatever provider it likes:

```json
{
  "kind": "verify_email" | "password_reset",
  "to": "user@example.com",
  "display_name": "User",
  "token": "<single-use token to embed in the link>",
  "expires_at": "2026-07-13T00:00:00Z"
}
```

`MAIL_WEBHOOK_TOKEN`, when set, is sent as a bearer so the webhook can authenticate the
server. Without `MAIL_WEBHOOK_URL` the mailer is disabled and the flows answer `501` —
an explicit deferral, never silent mail loss. Delegation keeps secrets and provider
churn out of keeplin: the operator's webhook owns templates, sender reputation, and the
provider account.

**Dependencies** — `chrono` (external): the `expires_at` timestamp. `reqwest`
(external): the HTTP client used in `Mailer`. `serde_json` (external): the payload
literal.

**Used by** — `state.rs` holds a `Mailer` in `AppState`; `http.rs` drives it from the
email-flow handlers (`send_flow_mail`, registration, `resend_verification`,
`request_password_reset`). `store.rs` mints/consumes the paired single-use tokens
(`create_email_token` / `consume_email_token`) keyed by `MailKind`.
`tests/integration.rs` (`email_verification_and_password_reset_flows`) drives a fake
webhook end to end.

**Repeated context** — The email-flow token model: the server stores only a **hash** of
each single-use flow token (`store.rs`), embeds the raw token in the webhook payload,
and the user proves receipt by presenting it back (`/api/verify-email`,
`/api/reset-password`); tokens expire after `EMAIL_TOKEN_TTL_SECS`. Flows must be
**visibly deferred** when unconfigured: `enabled()` false → HTTP `501`
(`AppError::NotImplemented`), so an operator can't lose mail silently.

---

## MailKind

**Identification** — enum; marker `// md:MailKind`.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailKind {
    VerifyEmail,
    PasswordReset,
}
```

**What it does** — The two email flows the server can ask the webhook to deliver:
address-verification mail and password-reset mail. `Copy`+`Eq` because it is used both
as a wire tag (via `as_str`) and as a database discriminator for the flow tokens.

**Dependencies** — none beyond `std`/derives.

**Used by** — `http.rs` (chooses the kind per flow handler), `store.rs`
(`create_email_token` / `consume_email_token` store and match on it so a verify token
cannot be replayed as a reset token), `Mailer::send` (this file).

**Repeated context** — Flow tokens are kind-scoped: consuming a token requires the same
`MailKind` it was minted with — the two flows are separate credential spaces.

---

## impl MailKind

**Identification** — inherent impl block; marker `// md:impl MailKind`. Contains
`fn as_str` (next section).

**What it does** — Wire-tag rendering only.

**Dependencies** — `MailKind` (this file).

**Used by** — see `fn as_str`.

**Repeated context** — none beyond the method's own (below).

### fn as_str

**Identification** — method; marker `// md:impl MailKind > fn as_str`.

```rust
pub fn as_str(self) -> &'static str
```

**What it does** — Renders the wire tag the webhook receives in `kind`:
`VerifyEmail` → `"verify_email"`, `PasswordReset` → `"password_reset"`. Total; no
failure mode. These strings are a contract with the operator's webhook (and are also
the stored discriminator values in the `email_tokens` table).

**Dependencies** — `MailKind` (this file).

**Used by** — `Mailer::send` (this file) for the payload; `store.rs` for the
token-table discriminator.

**Repeated context** — Renaming a tag breaks every deployed webhook and invalidates
stored flow tokens; treat the strings as frozen.

---

## Mailer

**Identification** — struct; marker `// md:Mailer`.

```rust
#[derive(Clone)]
pub struct Mailer {
    webhook_url: Option<String>,
    webhook_token: Option<String>,
    http: reqwest::Client,
}
```

**What it does** — Posts email-flow payloads to the operator's mail webhook. Holds the
optional webhook URL (`None` = delivery disabled), the optional bearer used to
authenticate the server to the webhook, and a `reqwest::Client`. Cheap to clone
(`reqwest::Client` is internally reference-counted); `AppState` stores one and shares
it.

**Dependencies** — `reqwest::Client` (external).

**Used by** — `state.rs::AppState` (field `mailer`, built in `AppState::new`);
`http.rs` flow handlers via `state.mailer`.

**Repeated context** — Fields are private on purpose: the only operations are
`enabled()` and `send(…)`, so no caller can bypass the disabled check or leak the
bearer.

---

## impl Mailer

**Identification** — inherent impl block; marker `// md:impl Mailer`. Contains
`fn new`, `fn enabled`, `fn send` (next sections).

**What it does** — Construction plus the two-method delivery API.

**Dependencies** — `Mailer` (this file).

**Used by** — see the method sections.

**Repeated context** — none beyond the methods' own (below).

### fn new

**Identification** — associated function; marker `// md:impl Mailer > fn new`.

```rust
pub fn new(webhook_url: Option<String>, webhook_token: Option<String>) -> Self
```

**What it does** — Builds the mailer from configuration (`MAIL_WEBHOOK_URL`,
`MAIL_WEBHOOK_TOKEN`, both optional) with a fresh `reqwest::Client`. Infallible; a
`None` URL simply produces a disabled mailer.

**Dependencies** — `reqwest::Client::new` (external).

**Used by** — `state.rs::AppState::new` (the only caller).

**Repeated context** — Configuration is loaded once at boot (`config.rs`); the mailer
never re-reads the environment.

### fn enabled

**Identification** — method; marker `// md:impl Mailer > fn enabled`.

```rust
pub fn enabled(&self) -> bool
```

**What it does** — Whether delivery is configured (`webhook_url.is_some()`). Flow
handlers check this **up front** and answer `501` (`AppError::NotImplemented`) when it
is false, so the deferral is visible to callers instead of mail being lost silently.

**Dependencies** — none.

**Used by** — `http.rs`: the registration handler (skips sending verification mail),
`resend_verification` and `request_password_reset` (both return `501` when false).

**Repeated context** — "Explicit deferral, never silent mail loss" is the module's core
rule; this method is the gate that implements it.

### fn send

**Identification** — async method; marker `// md:impl Mailer > fn send`.

```rust
pub async fn send(
    &self,
    kind: MailKind,
    to: &str,
    display_name: &str,
    token: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), String>
```

**What it does** — Delivers one flow message: POSTs the JSON payload
`{ kind, to, display_name, token, expires_at }` to the webhook URL, attaching
`Authorization: Bearer <MAIL_WEBHOOK_TOKEN>` when configured. Outcomes:

- No URL configured → `Err("mail webhook not configured")` (defensive; callers should
  have checked `enabled()` first).
- 2xx response → `Ok(())`.
- Non-2xx → `Err("mail webhook answered <status>")`.
- Transport failure → `Err("mail webhook unreachable: <cause>")`.

An unreachable/erroring webhook is an **error**, not a shrug, so the caller can surface
a `500` rather than pretend the mail went out — otherwise the freshly minted single-use
token would be stranded (stored but never delivered to the user).

**Dependencies** — `MailKind::as_str` (this file), `reqwest` request builder
(external), `serde_json::json!` (external).

**Used by** — `http.rs::send_flow_mail`, the single helper the flow handlers
(registration, `resend_verification`, `request_password_reset`) call.

**Repeated context** — Errors are plain `String`s (not `AppError`) because the HTTP
layer decides how to surface a delivery failure per flow: registration logs and
continues (the account exists; the user can re-request the mail), while the explicit
flows fail the request.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- `MailKind` — defined here (EXTRACTED; 3 cross-file edge(s))
- `Mailer` — defined here (EXTRACTED; 1 cross-file edge(s))
- `.as_str()` — defined here (EXTRACTED; file-local)
- `.new()` — defined here (EXTRACTED; file-local)
- `.enabled()` — defined here (EXTRACTED; file-local)
- `.send()` — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- (none in the graph) (EXTRACTED)

**Direct dependents** (files whose symbols reference this one)

- `crates/keeplin-srv/src/http.rs` — the REST router and handlers (EXTRACTED: references×1; e.g. `send_flow_mail()`)
- `crates/keeplin-srv/src/state.rs` — shared application state (EXTRACTED: references×1; e.g. `AppState`)
- `crates/keeplin-srv/src/store.rs` — the PostgreSQL data-access layer (EXTRACTED: references×2; e.g. `.consume_email_token()`, `.create_email_token()`)

## Coverage checklist

Every code block of `mail.rs`, in source order, each documented above (five points) and
carrying its marker in the code:

| # | Block (source order) | Marker in code | Documented in section |
|---|----------------------|----------------|-----------------------|
| 1 | imports (`use …`) | `// md:Overview` | Overview |
| 2 | `enum MailKind` | `// md:MailKind` | MailKind |
| 3 | `impl MailKind` | `// md:impl MailKind` | impl MailKind |
| 4 | `fn as_str` | `// md:impl MailKind > fn as_str` | impl MailKind › fn as_str |
| 5 | `struct Mailer` | `// md:Mailer` | Mailer |
| 6 | `impl Mailer` | `// md:impl Mailer` | impl Mailer |
| 7 | `fn new` | `// md:impl Mailer > fn new` | impl Mailer › fn new |
| 8 | `fn enabled` | `// md:impl Mailer > fn enabled` | impl Mailer › fn enabled |
| 9 | `fn send` | `// md:impl Mailer > fn send` | impl Mailer › fn send |
