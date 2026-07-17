# `src/mail.rs` — delegated email delivery (mail webhook)

## Purpose

keeplin-srv is **not a mail client**: it never speaks SMTP and holds no provider SDK. When an
email flow (verification, password reset — issue #49) needs a message delivered, `Mailer`
POSTs a small JSON payload to the operator's own mail service (`MAIL_WEBHOOK_URL`), which
composes and sends the actual email with whatever provider it likes.

## Key types

| Type | Kind | Description |
|------|------|-------------|
| `MailKind` | enum | `VerifyEmail` / `PasswordReset`; `as_str()` yields the wire tags `verify_email` / `password_reset` |
| `Mailer` | struct | cheap-to-clone webhook poster: optional URL, optional bearer, a `reqwest::Client` |

## Public API

| Function | Description |
|----------|-------------|
| `Mailer::new(url, token)` | build from config (`MAIL_WEBHOOK_URL`, `MAIL_WEBHOOK_TOKEN`) |
| `enabled() -> bool` | delivery configured? Flows check this up front and answer `501` when not — an explicit deferral, never silent mail loss |
| `send(kind, to, display_name, token, expires_at) -> Result<(), String>` | POST `{ kind, to, display_name, token, expires_at }` (+ optional bearer). Non-2xx or unreachable → `Err`, so the caller surfaces `500` rather than pretending the mail went out (the flow token would be stranded) |

## Wire payload

```json
{
  "kind": "verify_email" | "password_reset",
  "to": "user@example.com",
  "display_name": "User",
  "token": "<single-use token to embed in the link>",
  "expires_at": "2026-07-13T00:00:00Z"
}
```

## Design notes

- Delegation keeps secrets and provider churn out of keeplin: the operator's webhook owns
  templates, sender reputation, and the provider account.
- Errors are strings (not `AppError`) because the HTTP layer decides how to surface them per
  flow.

## Graph context

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

**Invariants** (restated on purpose; a change to this file must keep these true)

- keeplin-srv never speaks SMTP: delivery is always delegated to the operator's webhook.
- When no webhook is configured, `enabled()` is false and the email flows answer `501` — an explicit deferral, never silent mail loss.
- A non-2xx/unreachable webhook is an `Err` so the caller surfaces `500`; pretending the mail went out would strand the single-use token.

## Related files

- `src/http.rs` — the verify/reset endpoints that call `Mailer::send`.
- `src/config.rs` — `MAIL_WEBHOOK_URL` / `MAIL_WEBHOOK_TOKEN` / `EMAIL_TOKEN_TTL_SECS`.
- `src/store.rs` — hashed single-use email tokens the payload's `token` pairs with.
- `tests/integration.rs` — `email_verification_and_password_reset_flows` drives a fake webhook.
