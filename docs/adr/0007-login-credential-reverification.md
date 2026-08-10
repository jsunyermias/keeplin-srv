# 0007 — A login must re-verify its credential inside the operation that mints the session

- Status: accepted
- Date: 2026-08-10
- Decision owners: maintainer of `jsunyermias/keeplin-srv`
- Scope: keeplin-srv
- Issue: [keeplin-srv#151](https://github.com/jsunyermias/keeplin-srv/issues/151)
- Acceptance PR: [keeplin-srv#157](https://github.com/jsunyermias/keeplin-srv/pull/157)
- Supersedes: [ADR 0005](0005-serializable-participant-set.md) in part — its enumeration of nine
  serializable HTTP handlers becomes ten with the addition of `login`, and only that. ADR 0005's
  invariant 2 (the writer set over note rows, note shares, notebook rows, notebook shares and
  ownership) is untouched: `login` writes a device row, which is not in that set, so this decision
  does not enlarge the writer inventory or its derivation.
- Extends: [ADR 0002](0002-authorization-mutation-atomicity.md)'s rule — authorization and the
  operation it authorizes are one transactionally consistent decision — to a credential it did not
  enumerate. ADR 0002's re-verification shape, refusal shapes, three-attempt bound, `503` on
  exhaustion and notice ordering are unchanged.
- Superseded by: none

## Context and problem

`Verified at`: `keeplin-srv@c5a0461`, the current tip of the working branch, itself `origin/main`.

`login` verifies a password and then, in two later steps that share no transaction with the
verification, creates a device and mints a token. `reset_confirm` sets a new password and revokes
every existing device. A revocation cannot reach a device that does not exist yet, so a login whose
verification has already passed when a reset commits goes on to create a *fresh* device authenticated
by a password the user has just reset — and returns a working token the total revocation never saw.

### The fact that forces this decision, read from the tree rather than reasoned

`login` (`crates/keeplin-srv/src/http.rs:433-498`) runs, in order and outside any transaction:

```rust
if !auth::verify_password(&body.password, &user.password_hash)? { /* 467: reject */ }
// ...
let device = state.store.create_device(user.id, &body.device_name).await?;   // 480-483
let token = auth::create_token(user.id, device.id, /* ... */)?;              // 485-492
```

`reset_confirm` (`http.rs:730-750`) runs, each call on the pool at `READ COMMITTED`:

```rust
state.store.update_password(user_id, &hash).await?;    // 744
state.store.delete_all_devices(user_id).await?;        // 745
```

Between `login`'s line 467 and its line 480 there is no re-read of the credential and no transaction.
`delete_all_devices` deletes the rows that exist when it runs; the device `login` creates at 483 is
not among them. The reset returns `200` and nothing records that a session survived it.

**The window is the Argon2 verification, not a scheduler accident.** `verify_password` at 467 is
Argon2 with this repository's configured parameters, measured at **452 ms per call** for
[keeplin-srv#149](https://github.com/jsunyermias/keeplin-srv/issues/149) with the crate the server
uses. An attacker holding a stolen password holds a ~half-second window per attempt and may retry it
at will; this is a comfortably reachable interleaving, not a theoretical one.

### The precedent this decision follows, already in the tree

`delete_account` (`http.rs:604-635`) had the same check-then-act shape and was closed by ADR 0005 as
one of its nine. It verifies the password outside the transaction, captures the verified hash, and
re-checks it inside the serializable operation:

```rust
let verified_password_hash = stored.password_hash;
serializable(state.clone(), "delete_account", |state, conn| { Box::pin(async move {
    let stored = state.store.get_user_by_id_on(conn, user.user_id).await?.ok_or(AppError::NotFound)?;
    if stored.password_hash != verified_password_hash { return Err(AppError::InvalidToken); }  // 401
    state.store.delete_user_on(conn, user.user_id).await
})}).await?;
```

Its interleaving is pinned by the case `changed_password_is_reverified_for_delete_account`
(`tests/authorization.rs:511`, `Refusal(401)`). `login` currently carries an exemption instead
(`tests/authorization.rs:518`, `Exempt("credential verification is the operation and there is no
earlier authenticated guard")`). Independent review checked that reason against the handler and found
it false: verification and device creation are two steps, and a password change fits between them.
The exemption is what this decision removes.

## Why an accepted decision record is required

`login` is an authentication path, and AGENTS.md requires an accepted ADR before implementation for
authentication changes and for adding a participant to a protection an accepted ADR enumerates. ADR
0005 states its participant set as a closed enumeration ("the enumerated set becomes nine") and warns
in its own part three that a new participant must not "quietly join the set of writers nobody
enumerated." Adding `login` is exactly such a change and is recorded here rather than slipped into
code against an accepted enumeration.

## Threat model

**Asset.** The promise a password reset makes: that the old credential no longer yields a session.

**Adversary.** A holder of a stolen or leaked password, racing the legitimate owner's reset. No timing
skill is required beyond retrying within a ~452 ms window, and no privilege beyond the stolen
password.

**Consequence.** A session minted from the old credential survives a completed reset. Unlike
`delete_account`'s residual — where the actor is deleting their own account and a stale password
merely lets that deletion proceed — here the surviving artifact is an authenticated session in an
adversary's hands. That difference is why the residual below is stated explicitly rather than folded
into the precedent.

**Out of scope.** Session revocation mechanisms that outlive a transaction (a credential-generation
counter carried in the token) — that is issue #151's Option B, a larger change with a migration and a
token-claim decision, and it is not this decision.

## Options considered

The issue records three. The maintainer selected Option A. The other two are recorded so the choice
is legible.

### Option A — re-verify the credential inside the operation transaction (adopted)

`login`, after verifying the password, re-reads the user inside a `serializable` transaction, refuses
with `401` if `password_hash` no longer matches what it verified, and otherwise creates the device in
that same transaction. This is the `delete_account` shape exactly, reusing the existing `serializable`
helper (`http.rs:43-115`) and its three-attempt bound. Cost is a string comparison and a device
insert moved inside a transaction; no second Argon2 call, no schema change, no token-format change.

### Option B — a credential-generation counter (not adopted here)

A counter on the user, carried in the token and checked on use, would close this and every future
"a token outlived its credential" variant, including ones no transaction can observe. It is larger: a
migration, a token-claim change, and a decision about whether existing tokens survive. Recorded as
the more complete answer and left available; not this decision.

### Option C — reorder or re-revoke the reset, or lock the pair (not adopted)

Revoking devices again after a delay, or ordering the reset differently, narrows the window rather
than closing it and is recorded because it will be proposed.

## Decision and justification

**Adopt Option A.** `login` joins the serializable participant set as the tenth handler, for the
credential-atomicity reason ADR 0002 states, applied to the login credential. Concretely:

1. `login` keeps its pre-transaction password verification (the Argon2 call stays outside the
   transaction; it must not be repeated per retry and must not hold a transaction open for 452 ms —
   the concern [keeplin-srv#149](https://github.com/jsunyermias/keeplin-srv/issues/149) raised).
2. It captures the verified `password_hash`, then enters `serializable(state, "login", …)` and, inside
   it, re-reads the user (`get_user_by_email_on` / `get_user_by_id_on`, both already present) and
   refuses with `AppError::InvalidToken` (`401`) if the stored hash no longer equals the verified one.
3. In the same transaction it creates the device through a new executor-aware `create_device_on`
   (the store has `_on` reads today but no `_on` device insert; this adds one, as
   [keeplin-srv#145](https://github.com/jsunyermias/keeplin-srv/issues/145) added `_on` forms for its
   reads). The token is minted from the returned device after commit.
4. `reset_confirm` is **unchanged**. The re-verification is a value comparison against a
   `READ COMMITTED` writer, which is sufficient for the pinned interleaving exactly as it is for
   `delete_account`: when the reset commits before `login`'s transaction takes its snapshot, the
   re-read observes the new hash and refuses. ADR 0005's "a serializable transaction does not observe
   a concurrent read-committed writer" concerns predicate-lock antidependencies, not a direct
   read of an already-committed row.
5. The `login` row in the `MUTATING_HANDLER_INTERLEAVINGS` inventory
   (`tests/authorization.rs:518`) changes from `Exempt(…)` to `Refusal(401)` naming the new test,
   satisfying the issue's criterion 4 and keeping the inventory honest.

### What this closes, and the residual it does not — stated rather than implied

The adopted mechanism closes the window the issue is about: the ~452 ms Argon2 interval, and every
schedule in which the reset commits before `login`'s transaction begins its re-read. That is the
window criterion 1 pins and the one an attacker actually has.

It does **not** close a second, far narrower window: if `login`'s transaction takes its re-read
snapshot *before* the reset's `update_password` commits, and the reset's `delete_all_devices` then
runs before `login`'s device insert commits, `login` observes the old hash, passes the re-check, and
commits a device the revocation missed. Because `reset_confirm` writes at `READ COMMITTED` and
`login` writes a *new* device row rather than updating the user row, no write/write conflict forms and
`SERIALIZABLE` raises no `40001`. This residual is the same one `delete_account` carries, reduced from
the Argon2 window to the sub-millisecond gap between two database statements.

Closing the residual as well would require `login` to take a `SELECT … FOR UPDATE` row lock on the
user inside its transaction so that `reset_confirm`'s `update_password` serializes against it — ADR
0005's rejected-but-available Option 3 (an anchor row locked by every participant), applied to the
user row. That strengthening also touches `reset_confirm` and is therefore a wider change than the
`delete_account` mirror the maintainer selected. **This ADR adopts the `delete_account` mirror and
records the residual; whether to add the `FOR UPDATE` strengthening is called out for the maintainer
in "Open question" below rather than decided here.**

## Invariants

1. `login` executes its credential re-verification and its device creation in one `SERIALIZABLE`
   transaction, through the existing `serializable` helper and its three-attempt bound.
2. A `login` whose verified `password_hash` no longer matches the stored one at re-read time refuses
   with `401` and creates no device.
3. The handler interleaving inventory names `login`'s reverification test instead of carrying an
   exemption; removing the reverification fails that test rather than only changing an error message.
4. ADR 0005's invariant 2 writer set and its derivation are unchanged: `login` is not a writer of
   note rows, note shares, notebook rows, notebook shares or ownership.
5. ADR 0002's re-verification rule, refusal shapes, three-attempt bound, `503` on exhaustion and
   notice ordering are unchanged.

## Compatibility, migration, and rollback

No schema change, no migration, no wire or format change. `keeplin-core` is untouched and its pin does
not move. Token format and existing tokens are unaffected — this decision does not read, revoke or
re-issue any token already minted (issue #151, criterion 5): a session valid before the change stays
valid, and one invalid stays invalid.

The only client-visible changes are on the `login` endpoint: a `login` racing a concurrent reset may
now receive `401` (retry with the current password), and `login` gains `503` as a possible response
under serialization retry exhaustion, exactly as `delete_account` did under ADR 0005. Both are new
statuses on that endpoint.

Rollback restores the exemption and reopens the window this decision closes.

## Open question for the maintainer

Whether to adopt the `delete_account` mirror alone (residual accepted, as recorded above) or to add
the `SELECT … FOR UPDATE` strengthening on the user row that also closes the narrow between-statements
residual at the cost of a `reset_confirm` change. The verification plan below is written for the
adopted mirror; if the strengthening is chosen, row 1 gains the second interleaving (snapshot-before-
reset-commit) as a pinned refusal and `reset_confirm`'s change is covered by rows 6 and 8.

## Verification plan

| # | Evidence | Kind | What fails if the decision is violated |
|---|---|---|---|
| 1 | A deterministic interleaving pauses `login` after password verification (at the `serializable` helper's `before_operation` checkpoint), runs `reset_confirm` to completion, resumes `login`, and asserts the resulting token is **not** usable | negative, forced interleaving | **Fails on the current tree** — that is the defect (issue criteria 1 and 2) |
| 2 | Removing the in-transaction `password_hash` re-check (the mutation) makes row 1's test fail | mutation | Fails if the guard is asserted by spelling rather than behaviour; the mutation must change behaviour, not only an error string (issue criterion 3) |
| 3 | The `login` row in `MUTATING_HANDLER_INTERLEAVINGS` names row 1's test and carries no exemption | structural | Fails if the inventory still exempts `login` (issue criterion 4) |
| 4 | A `login` **not** racing a reset still succeeds and returns a usable token; a `login` with a wrong password still returns `401` and creates no device | regression | Fails if the reverification refuses a legitimate login or admits a wrong one |
| 5 | Existing tokens are unaffected: a token minted before the change authenticates after it | compatibility | Fails if the change touched token format or validity (issue criterion 5) |
| 6 | `cargo test --workspace` against PostgreSQL, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check` and `./scripts/check-docs.sh` all pass | repository | Fails on regression, warnings, formatting drift, or companion drift (issue criterion 6) |

Row 1 is expected to fail before implementation, for the reason
[keeplin-srv#138](https://github.com/jsunyermias/keeplin-srv/issues/138) records: a verification plan
whose rows all pass on the current tree is describing the present rather than deciding anything. Row 1
requires a forced rendezvous through the existing `http_test_hooks` checkpoints, not a hoped-for
interleaving.

## Equivalent decision in the other repository

None. `keeplin` has neither this HTTP layer nor PostgreSQL, and the decision changes no shared wire,
format or `keeplin-core` surface.
