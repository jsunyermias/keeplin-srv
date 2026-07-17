# Security — keeplin-srv

## Threat model at a glance

| Data | Protection |
|------|------------|
| Passwords | Argon2id hashes; login timing equalised for unknown emails (no enumeration); DB-backed brute-force lockout (`LOGIN_MAX_FAILURES`) |
| Device tokens | JWTs signed with `JWT_SECRET` (server refuses to start on a weak/placeholder secret); revocation is immediate — REST, `/api/sync` and `/api/ws` all re-check the device row |
| Note content & titles | **At rest**: optional AES-256-GCM column encryption (`AT_REST_KEY`) — a DB dump/backup shows ciphertext. **In memory**: plaintext on the server; collaborative merge requires it (see below) |
| Relay (`/api/sync`) payloads | Opaque to the server; keeplin-core encrypts client-side, so relay-mode note bodies are end-to-end encrypted |
| Email-flow tokens | 32 random bytes; only the SHA-256 is stored; single-use, expiring, consumed atomically; issuance capped per user (anti mail-bombing) |
| Error responses | Database/internal detail is logged server-side and never returned to clients |

## What the server can read — and why

Collaborative editing (`/api/ws`) merges line operations **on the server**, so
collaborative notes are processed in plaintext in server memory and are visible
to the server operator. This is inherent to server-side CRDT merging, not an
oversight. Mitigations, in order of strength:

1. **Relay-only mode**: don't use the collaborative channel; relay payloads are
   end-to-end encrypted by the client and the server never sees note contents.
2. **`AT_REST_KEY`**: encrypt titles and line content at rest, so database
   dumps, stolen backups, and SQL access see ciphertext. Does **not** protect
   against a compromised running server or the operator (both hold the key).
   Back the key up separately from the database — a backup bundle holding both
   the dump and the key is plaintext for whoever steals it. Rows written before
   the key was enabled stay plaintext until the one-off `keeplin-reencrypt`
   pass migrates them (see `RUNBOOK.md`, "Key rotation & re-encryption").
   There is **no live key rotation**: rotating means a maintenance-window
   re-encrypt under the new key with the old key still readable, as described
   in the runbook.

## Operational hardening checklist

- `JWT_SECRET`: long and random (`openssl rand -hex 32`); rotation invalidates
  every token (all users sign in again).
- `AT_REST_KEY`: enable for any deployment whose backups leave the box.
- `REGISTRATION_ENABLED=false` on single-tenant/private deployments.
- `MAIL_WEBHOOK_URL` + `EMAIL_VERIFICATION_REQUIRED=true` for public,
  multi-user deployments (account recovery + verified ownership).
- TLS terminates at a reverse proxy (`wss://`); pass tokens in the
  `Authorization` header, not the query string.
- Rate-limit at the proxy (per-IP limiting in the server is per-instance and
  off by default); the login lockout is DB-backed and replica-safe regardless.
- Postgres reachable only from the server (the example compose binds loopback).

## Review status & known limits

- The account/permission surface (auth middleware, capability grants, note and
  notebook sharing, ownership transfer, history visibility, device revocation)
  and the newer security code (at-rest crypto, lockout, email flows) have been
  through two internal code audits (2026-07); every finding is fixed or tracked
  in the issue tracker.
- **No external penetration test has been performed.** For a public deployment
  hosting third-party data, commission one — an internal review is not a
  substitute.
- Presence names and cursor positions of collaborators are visible to everyone
  with read access to a note (by design).
- **`HISTORY_VISIBILITY=access` windows honest clients, not adversarial ones.**
  The collaborator window compares the *payload's own* causal timestamp
  (`updated_at` / `deleted_at` inside the Change) against the share's grant
  time, so journal re-delivery (a reinstalled device re-pushing its journal
  from epoch) can no longer leak pre-access versions. But that timestamp is
  client-asserted: a malicious device could forge a future `updated_at` on a
  pre-access snapshot and slip it into the window. The policy is an honest-
  client privacy boundary, not a cryptographic one — do not rely on it against
  a hostile account that had (or colluded with) write access.

## Reporting a vulnerability

Open a GitHub security advisory on this repository (preferred), or a private
report to the maintainer. Please do not open public issues for exploitable
problems.
