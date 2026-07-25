# `tests/core_compat.rs` — the keeplin ↔ keeplin-srv wire/format contract, asserted

Self-contained companion for `crates/keeplin-srv/tests/core_compat.rs`. It documents
**every code block of the source file, in source order, with its complete code embedded**
— a reader with only this file must be able to understand and modify the test without
opening anything else, so project-wide conventions are deliberately re-explained here
(hyper-redundancy is intended).

**How to navigate**: every block carries exactly one marker comment
`// md:<Header> > … > <Block header>` whose path is the header chain of its section here;
grep it in either direction. Each block section covers, in this fixed order:
**Identification**, **Code**, **What it does**, **Dependencies**, **Used by**,
**Repeated context**.

---

## Overview

**Identification** — file-level block: the imports; marker `// md:Overview`.

**Code** — complete and verbatim:

```rust
// md:Overview
use chrono::{TimeZone, Utc};
use keeplin_core::collab::protocol as core_protocol;
use keeplin_core::storage::note_log::VersionVector;
use keeplin_srv::protocol as srv_protocol;
use serde_json::Value;
use uuid::Uuid;
```

**What it does** — This file is the **cross-repo compatibility test**: the mechanical guarantee that
keeplin-srv and the keeplin client agree on every shared wire/format surface. The
project's rule (both repositories' `CLAUDE.md`) is that keeplin-core is the single
source of truth for shared types and constants, that keeplin-srv pins it to an exact
immutable `rev`, and that **a change to a shared surface is not complete until a test
round-trips every protocol message and shared constant against keeplin-core's real
types**. This is that test.

It needs no database and no server: it constructs each `keeplin_srv::protocol`
message, serialises it, parses it with the corresponding
`keeplin_core::collab::protocol` type, re-serialises, and compares the JSON — in both
directions. Structural drift (a renamed field, a changed tag, a type that stopped
being optional) fails here as a JSON mismatch rather than in production as a message
one side silently cannot read.

**Dependencies** — `chrono` (fixed timestamps), `keeplin_core::collab::protocol` (the client's real wire types), `keeplin_core::storage::note_log::VersionVector` (the vv type both sides already share), `keeplin_srv::protocol` (this crate's mirror), `serde_json` (the comparison medium), `uuid`. Expects the two protocol modules to stay serde-compatible; that is the whole subject of the file.

**Used by** — CI (`cargo test --workspace`) — it runs without `DATABASE_URL`, so it also fails fast in any environment where the database-backed tests are skipped.

**Repeated context** — keeplin-core is the single source of truth for shared wire/format
types and constants; keeplin-srv imports them and pins the crate to an exact git `rev`,
never a branch. A breaking change to a shared surface bumps `PROTOCOL_VERSION` on both
sides in lockstep; an additive optional field does not.

---

## fn fixed_uuid

**Identification** — helper; marker `// md:fn fixed_uuid`.

**Code** — complete and verbatim:

```rust
// md:fn fixed_uuid
fn fixed_uuid(byte: u8) -> Uuid {
    Uuid::from_bytes([byte; 16])
}
```

**What it does** — Builds a deterministic UUID from a repeated byte, so the JSON compared on both sides
is stable and a failure diff is readable. `Uuid::new_v4()` would work but would make
every failure message different.

**Dependencies** — `uuid::Uuid::from_bytes`.

**Used by** — every message fixture in this file.

**Repeated context** — keeplin-core is the single source of truth for shared wire/format
types and constants; keeplin-srv imports them and pins the crate to an exact git `rev`,
never a branch. A breaking change to a shared surface bumps `PROTOCOL_VERSION` on both
sides in lockstep; an additive optional field does not.

---

## fn fixed_vv

**Identification** — helper; marker `// md:fn fixed_vv`.

**Code** — complete and verbatim:

```rust
// md:fn fixed_vv
fn fixed_vv() -> VersionVector {
    VersionVector::from([("device-a".to_string(), 7u64)])
}
```

**What it does** — A one-entry version vector with a fixed actor and counter. The vv type itself is
already shared (`keeplin_core::storage::note_log::VersionVector`), so this fixture
exists to make the surrounding messages concrete, not to test the vv.

**Dependencies** — `VersionVector::from`; expects the map shape to serialise as a JSON object of actor → counter.

**Used by** — `srv_line_ops`, `srv_snapshot`.

**Repeated context** — keeplin-core is the single source of truth for shared wire/format
types and constants; keeplin-srv imports them and pins the crate to an exact git `rev`,
never a branch. A breaking change to a shared surface bumps `PROTOCOL_VERSION` on both
sides in lockstep; an additive optional field does not.

---

## fn srv_line_ops

**Identification** — helper; marker `// md:fn srv_line_ops`.

**Code** — complete and verbatim:

```rust
// md:fn srv_line_ops
fn srv_line_ops() -> Vec<srv_protocol::LineOp> {
    let now = Utc.with_ymd_and_hms(2026, 7, 20, 12, 0, 0).unwrap();
    vec![
        srv_protocol::LineOp::Insert {
            after_line_id: Some(fixed_uuid(1)),
            line_id: fixed_uuid(2),
            content: "hola".into(),
            vv: fixed_vv(),
            last_writer: "device-a".into(),
            updated_at: now,
        },
        srv_protocol::LineOp::Update {
            line_id: fixed_uuid(2),
            content: "adéu".into(),
            vv: fixed_vv(),
            last_writer: "device-a".into(),
            updated_at: now,
        },
        srv_protocol::LineOp::Delete {
            line_id: fixed_uuid(2),
            deleted_at: now,
            vv: fixed_vv(),
            last_writer: "device-a".into(),
            updated_at: now,
        },
        srv_protocol::LineOp::Move {
            line_ids: vec![fixed_uuid(2), fixed_uuid(3)],
            after_line_id: None,
            vv: fixed_vv(),
            last_writer: "device-a".into(),
            updated_at: now,
        },
    ]
}
```

**What it does** — One instance of **every** `LineOp` variant — `Insert`, `Update`, `Delete`, `Move` —
built with keeplin-srv's types. Coverage is the point: a new variant added on one side
only will not appear here, and the reviewer adding it has to decide what the other side
does with it. `Insert` carries `Some(after_line_id)` while `Move` carries `None`, so the
optional field is exercised both ways; the `Update` content is non-ASCII (`adéu`) so a
UTF-8 handling difference would surface as a JSON mismatch.

**Dependencies** — `srv_protocol::LineOp`, `fixed_uuid`, `fixed_vv`, `chrono::TimeZone::with_ymd_and_hms`; expects the four variants to keep their `op`-tagged serde representation.

**Used by** — `every_client_message_round_trips_against_keeplin_core`, `every_server_message_round_trips_against_keeplin_core`.

**Repeated context** — keeplin-core is the single source of truth for shared wire/format
types and constants; keeplin-srv imports them and pins the crate to an exact git `rev`,
never a branch. A breaking change to a shared surface bumps `PROTOCOL_VERSION` on both
sides in lockstep; an additive optional field does not.

---

## fn srv_snapshot

**Identification** — helper; marker `// md:fn srv_snapshot`.

**Code** — complete and verbatim:

```rust
// md:fn srv_snapshot
fn srv_snapshot() -> srv_protocol::NoteLinesSnapshot {
    let now = Utc.with_ymd_and_hms(2026, 7, 20, 12, 0, 0).unwrap();
    srv_protocol::NoteLinesSnapshot {
        note_id: fixed_uuid(9),
        order: vec![fixed_uuid(2), fixed_uuid(3)],
        updated_at: now,
        vv: fixed_vv(),
        last_writer: "device-a".into(),
        lines: vec![srv_protocol::LineSnapshot {
            id: fixed_uuid(2),
            content: "hola".into(),
            created_at: now,
            updated_at: now,
            deleted_at: None,
            vv: fixed_vv(),
            last_writer: "device-a".into(),
        }],
    }
}
```

**What it does** — A `NoteLinesSnapshot` — the `Welcome` payload, the largest shared structure — with a
populated order vector and one live `LineSnapshot`. `deleted_at: None` exercises the
optional timestamp; the snapshot is what a client rebuilds its whole mirror from, so a
field drift here would corrupt every note on join.

**Dependencies** — `srv_protocol::{NoteLinesSnapshot, LineSnapshot}`, `fixed_uuid`, `fixed_vv`, `chrono`.

**Used by** — `every_server_message_round_trips_against_keeplin_core`.

**Repeated context** — keeplin-core is the single source of truth for shared wire/format
types and constants; keeplin-srv imports them and pins the crate to an exact git `rev`,
never a branch. A breaking change to a shared surface bumps `PROTOCOL_VERSION` on both
sides in lockstep; an additive optional field does not.

---

## fn round_trips_through_core

**Identification** — generic helper; marker `// md:fn round_trips_through_core`.

**Code** — complete and verbatim:

```rust
// md:fn round_trips_through_core
fn round_trips_through_core<S, C>(message: &S) -> Value
where
    S: serde::Serialize + serde::de::DeserializeOwned,
    C: serde::Serialize + serde::de::DeserializeOwned,
{
    let sent = serde_json::to_value(message).expect("keeplin-srv message serialises");
    let received: C = serde_json::from_value(sent.clone())
        .unwrap_or_else(|e| panic!("keeplin-core cannot read {sent}: {e}"));
    let echoed = serde_json::to_value(&received).expect("keeplin-core message serialises");
    assert_eq!(
        sent, echoed,
        "keeplin-core re-serialises this message differently"
    );
    let back: S = serde_json::from_value(echoed.clone())
        .unwrap_or_else(|e| panic!("keeplin-srv cannot read back {echoed}: {e}"));
    let reserialised = serde_json::to_value(&back).expect("keeplin-srv message serialises");
    assert_eq!(sent, reserialised, "the round trip changed the message");
    sent
}
```

**What it does** — The round-trip itself, in both directions. Serialise the keeplin-srv message to JSON
(`sent`); parse it as the keeplin-core type `C` — failing loudly with the offending
JSON if the client cannot read what the server sends; re-serialise the core value
(`echoed`) and require `sent == echoed`, which catches a field the client would **drop**
or rename on its way back out; then parse `echoed` back into the keeplin-srv type and
re-serialise, requiring equality again, which catches the mirror-image loss on the
server side. Returns `sent` so callers can assert on the tag.

Comparing `serde_json::Value`s rather than the values themselves is deliberate: the two
protocol types are distinct Rust types in different crates and neither implements
`PartialEq` against the other, but the JSON *is* the contract — it is what actually
crosses the socket.

**Dependencies** — `serde_json::{to_value, from_value}`; expects both types to be `Serialize + DeserializeOwned` and their serde attributes to be the sole determinant of the wire shape.

**Used by** — all three message round-trip tests below.

**Repeated context** — keeplin-core is the single source of truth for shared wire/format
types and constants; keeplin-srv imports them and pins the crate to an exact git `rev`,
never a branch. A breaking change to a shared surface bumps `PROTOCOL_VERSION` on both
sides in lockstep; an additive optional field does not.

---

## fn the_three_format_limits_are_the_ones_keeplin_core_defines

**Identification** — unit test; marker `// md:fn the_three_format_limits_are_the_ones_keeplin_core_defines`.

**Code** — complete and verbatim:

```rust
// md:fn the_three_format_limits_are_the_ones_keeplin_core_defines
#[test]
fn the_three_format_limits_are_the_ones_keeplin_core_defines() {
    use keeplin_core::format;

    assert_eq!(format::MAX_LINE_BYTES, 1 << 12);
    assert_eq!(format::MAX_LINES_PER_NOTE, 1 << 16);
    assert_eq!(format::MAX_NOTES_PER_NOTEBOOK, 1 << 24);
    assert_eq!(format::CODE_LINE_TOO_LONG, "too_long");
    assert_eq!(format::CODE_TOO_MANY_LINES, "too_many_lines");
    assert_eq!(format::CODE_NOTEBOOK_FULL, "notebook_full");

    let at_limit = "a".repeat(format::MAX_LINE_BYTES);
    assert!(format::check_line(&at_limit).is_ok());
    assert!(format::check_line(&format!("{at_limit}a")).is_err());
    assert!(format::check_line_count(format::MAX_LINES_PER_NOTE).is_ok());
    assert!(format::check_line_count(format::MAX_LINES_PER_NOTE + 1).is_err());
    assert!(format::check_notebook_capacity(format::MAX_NOTES_PER_NOTEBOOK - 1).is_ok());
    assert!(format::check_notebook_capacity(format::MAX_NOTES_PER_NOTEBOOK).is_err());
}
```

**What it does** — Pins the shared **format** contract (issue keeplin#130): the three limits are the exact
powers of two keeplin-core defines (2¹², 2¹⁶, 2²⁴), the three wire codes are the exact
strings, and the predicates behave at the boundary — 4096 bytes accepted and 4097
rejected, 65 536 lines accepted and 65 537 rejected, a notebook with 2²⁴−1 notes taking
one more and one with 2²⁴ refusing.

Asserting this from **keeplin-srv's** test suite is what makes it a cross-repo test:
the server no longer declares its own limits, so this test exercises the values it
actually enforces, and bumping the pinned keeplin-core `rev` to a version with
different limits fails here rather than in production.

**Dependencies** — `keeplin_core::format` — the whole module; expects the constants and the `check_*` predicates to keep their current semantics (`check_line_count` takes the resulting count, `check_notebook_capacity` the count before the new note).

**Used by** — CI only.

**Repeated context** — keeplin-core is the single source of truth for shared wire/format
types and constants; keeplin-srv imports them and pins the crate to an exact git `rev`,
never a branch. A breaking change to a shared surface bumps `PROTOCOL_VERSION` on both
sides in lockstep; an additive optional field does not.

---

## fn protocol_version_matches_keeplin_core

**Identification** — unit test; marker `// md:fn protocol_version_matches_keeplin_core`.

**Code** — complete and verbatim:

```rust
// md:fn protocol_version_matches_keeplin_core
#[test]
fn protocol_version_matches_keeplin_core() {
    assert_eq!(
        keeplin_srv::http::PROTOCOL_VERSION,
        keeplin_core::compat::PROTOCOL_VERSION
    );
    assert!(keeplin_core::compat::compatible_with(
        keeplin_srv::http::PROTOCOL_VERSION
    ));
}
```

**What it does** — The two `PROTOCOL_VERSION` constants — keeplin-srv's (in `http.rs`, advertised by
`GET /version`) and keeplin-core's (in `compat.rs`, checked by the client at connect
time) — must be equal, and the client's own `compatible_with` must accept the server's.
The project rule is that a breaking change to a shared surface bumps both in lockstep;
this test is what makes "in lockstep" mechanical instead of a promise.

**Dependencies** — `keeplin_srv::http::PROTOCOL_VERSION`, `keeplin_core::compat::{PROTOCOL_VERSION, compatible_with}`; expects `compatible_with` to remain an exact-equality rule (additive evolution goes through `capabilities`, not the version).

**Used by** — CI only.

**Repeated context** — keeplin-core is the single source of truth for shared wire/format
types and constants; keeplin-srv imports them and pins the crate to an exact git `rev`,
never a branch. A breaking change to a shared surface bumps `PROTOCOL_VERSION` on both
sides in lockstep; an additive optional field does not.

---

## fn every_client_message_round_trips_against_keeplin_core

**Identification** — unit test; marker `// md:fn every_client_message_round_trips_against_keeplin_core`.

**Code** — complete and verbatim:

```rust
// md:fn every_client_message_round_trips_against_keeplin_core
#[test]
fn every_client_message_round_trips_against_keeplin_core() {
    let note_id = fixed_uuid(9);
    let messages = vec![
        srv_protocol::CollabClientMsg::Join { note_id },
        srv_protocol::CollabClientMsg::Leave { note_id },
        srv_protocol::CollabClientMsg::Op {
            note_id,
            ops: srv_line_ops(),
        },
        srv_protocol::CollabClientMsg::Cursor {
            note_id,
            cursor: srv_protocol::Cursor {
                line_id: fixed_uuid(2),
                column: 3,
            },
        },
        srv_protocol::CollabClientMsg::Ack { server_seq: 42 },
    ];
    let mut seen = Vec::new();
    for message in &messages {
        let json = round_trips_through_core::<_, core_protocol::CollabClientMsg>(message);
        seen.push(json["type"].as_str().expect("tagged message").to_string());
    }
    assert_eq!(seen, ["Join", "Leave", "Op", "Cursor", "Ack"]);
}
```

**What it does** — Every `CollabClientMsg` variant — `Join`, `Leave`, `Op`, `Cursor`, `Ack` — round-trips
through keeplin-core unchanged. The `Op` case carries all four `LineOp` variants, so
this single test covers the client→server half of the whole collab vocabulary. The
final `assert_eq!` on the collected `type` tags is a guard against the fixture list
being silently reordered or truncated: if someone drops a message from the list, the
test fails rather than quietly covering less.

**Dependencies** — `srv_protocol::CollabClientMsg`, `core_protocol::CollabClientMsg`, `round_trips_through_core`, `srv_line_ops`, `fixed_uuid`.

**Used by** — CI only.

**Repeated context** — keeplin-core is the single source of truth for shared wire/format
types and constants; keeplin-srv imports them and pins the crate to an exact git `rev`,
never a branch. A breaking change to a shared surface bumps `PROTOCOL_VERSION` on both
sides in lockstep; an additive optional field does not.

---

## fn every_server_message_round_trips_against_keeplin_core

**Identification** — unit test; marker `// md:fn every_server_message_round_trips_against_keeplin_core`.

**Code** — complete and verbatim:

```rust
// md:fn every_server_message_round_trips_against_keeplin_core
#[test]
fn every_server_message_round_trips_against_keeplin_core() {
    let note_id = fixed_uuid(9);
    let messages = vec![
        srv_protocol::CollabServerMsg::Welcome {
            note_id,
            snapshot: srv_snapshot(),
        },
        srv_protocol::CollabServerMsg::Op {
            server_seq: 5,
            note_id,
            user_id: fixed_uuid(4).to_string(),
            ops: srv_line_ops(),
        },
        srv_protocol::CollabServerMsg::Presence {
            note_id,
            users: vec![srv_protocol::PresenceInfo {
                user_id: fixed_uuid(4).to_string(),
                display_name: "Jordi".into(),
                cursor: Some(srv_protocol::Cursor {
                    line_id: fixed_uuid(2),
                    column: 0,
                }),
            }],
        },
        srv_protocol::CollabServerMsg::Error {
            code: keeplin_core::format::CODE_LINE_TOO_LONG.into(),
            message: "line exceeds the format limit of 4096 bytes".into(),
            note_id: Some(note_id),
        },
        srv_protocol::CollabServerMsg::Error {
            code: "bad_message".into(),
            message: "unparseable message".into(),
            note_id: None,
        },
    ];
    let mut seen = Vec::new();
    for message in &messages {
        let json = round_trips_through_core::<_, core_protocol::CollabServerMsg>(message);
        seen.push(json["type"].as_str().expect("tagged message").to_string());
    }
    assert_eq!(seen, ["Welcome", "Op", "Presence", "Error", "Error"]);
}
```

**What it does** — Every `CollabServerMsg` variant — `Welcome`, `Op`, `Presence`, `Error` — round-trips
through keeplin-core unchanged, with `Error` covered **twice**: once carrying a
`note_id` (a format-limit rejection) and once without (a connection-level error). Both
must survive the trip, because the optional field is the mechanism the client uses to
decide whether a rejection is repairable.

As above, the trailing tag assertion pins the fixture list — here it also documents that
`Error` is deliberately present twice.

**Dependencies** — `srv_protocol::CollabServerMsg`, `core_protocol::CollabServerMsg`, `round_trips_through_core`, `srv_snapshot`, `srv_line_ops`, `keeplin_core::format::CODE_LINE_TOO_LONG`.

**Used by** — CI only.

**Repeated context** — keeplin-core is the single source of truth for shared wire/format
types and constants; keeplin-srv imports them and pins the crate to an exact git `rev`,
never a branch. A breaking change to a shared surface bumps `PROTOCOL_VERSION` on both
sides in lockstep; an additive optional field does not.

---

## fn a_limit_rejection_names_the_note_and_an_old_client_still_parses_it

**Identification** — unit test; marker `// md:fn a_limit_rejection_names_the_note_and_an_old_client_still_parses_it`.

**Code** — complete and verbatim:

```rust
// md:fn a_limit_rejection_names_the_note_and_an_old_client_still_parses_it
#[test]
fn a_limit_rejection_names_the_note_and_an_old_client_still_parses_it() {
    let note_id = fixed_uuid(9);
    let rejection = srv_protocol::CollabServerMsg::Error {
        code: keeplin_core::format::CODE_TOO_MANY_LINES.into(),
        message: "note exceeds the format limit of 65536 lines".into(),
        note_id: Some(note_id),
    };
    let json = serde_json::to_value(&rejection).unwrap();
    assert_eq!(json["note_id"], serde_json::json!(note_id));

    let core: core_protocol::CollabServerMsg = serde_json::from_value(json).unwrap();
    match core {
        core_protocol::CollabServerMsg::Error {
            code,
            note_id: carried,
            ..
        } => {
            assert!(keeplin_core::format::is_limit_code(&code));
            assert_eq!(carried, Some(note_id));
        }
        other => panic!("expected an Error message, got {other:?}"),
    }

    let legacy = serde_json::json!({
        "type": "Error",
        "code": "forbidden",
        "message": "no access to this note",
    });
    let core: core_protocol::CollabServerMsg = serde_json::from_value(legacy.clone()).unwrap();
    match core {
        core_protocol::CollabServerMsg::Error { note_id, .. } => assert_eq!(note_id, None),
        other => panic!("expected an Error message, got {other:?}"),
    }
    let srv: srv_protocol::CollabServerMsg = serde_json::from_value(legacy).unwrap();
    match srv {
        srv_protocol::CollabServerMsg::Error { note_id, .. } => assert_eq!(note_id, None),
        other => panic!("expected an Error message, got {other:?}"),
    }
}
```

**What it does** — The compatibility argument for the one field this issue added to the wire, stated as a
test. Three claims: (1) a format-limit rejection **serialises** `note_id`, and
keeplin-core parses it back as `Some(note_id)` with a code its `is_limit_code`
recognises — the exact pair of facts the client's repair path depends on; (2) a legacy
`Error` frame with **no** `note_id` still deserialises on the client side, yielding
`None`; (3) the same legacy frame still deserialises on the server's own type.

Together these are why `PROTOCOL_VERSION` did not need to move: the field is additive
and optional in both directions, so an old client against a new server and a new client
against an old server both keep working.

**Dependencies** — `srv_protocol::CollabServerMsg`, `core_protocol::CollabServerMsg`, `keeplin_core::format::{CODE_TOO_MANY_LINES, is_limit_code}`, `serde_json`; expects `#[serde(default)]` to stay on the field — removing it would turn every legacy frame into a parse error.

**Used by** — CI only.

**Repeated context** — keeplin-core is the single source of truth for shared wire/format
types and constants; keeplin-srv imports them and pins the crate to an exact git `rev`,
never a branch. A breaking change to a shared surface bumps `PROTOCOL_VERSION` on both
sides in lockstep; an additive optional field does not.

---

## Graph context

Repo-tooling metadata, not a code block (no marker in the source). Kept in every
companion because CI (`scripts/check-docs.sh`) enforces it: this file is LAYER 2 of the
navigation model, the Graphify graph (`graphify-out/graph.json`) is LAYER 1; refresh with
`graphify update .` after refactors.

<!-- Data source: graphify-out/graph.json (AST pass; `graphify update .` refreshes it).
     EXTRACTED = mechanically from the graph; INFERRED = authored judgement. -->

**Nodes/edges this file contributes** (top symbols by cross-file degree)

- the helper fns and every test fn — defined here (EXTRACTED; file-local)

**Direct dependencies** (files this one's symbols reference)

- `crates/keeplin-srv/src/protocol.rs` — the server's mirror of the collab wire types (INFERRED)
- `crates/keeplin-srv/src/http.rs` — `PROTOCOL_VERSION` (INFERRED)
- keeplin-core (pinned git dependency) — `collab::protocol`, `compat`, `format`, `storage::note_log` (INFERRED)

**Direct dependents** (files whose symbols reference this one)

- (none — it is a test) (EXTRACTED)

**Invariants** (the rules this file must keep true — restated here even if stated elsewhere)

- Every collab protocol message and every shared constant round-trips between the two
  repositories' types; adding a variant or a field on one side without the other fails here.
- The three format limits and the three limit wire codes are keeplin-core's, not
  keeplin-srv's own.
- `PROTOCOL_VERSION` is identical on both sides.

---

## Coverage checklist

| # | Block (source order) | Marker in code |
|---|----------------------|----------------|
| 1 | `Overview` | `// md:Overview` |
| 2 | `fn fixed_uuid` | `// md:fn fixed_uuid` |
| 3 | `fn fixed_vv` | `// md:fn fixed_vv` |
| 4 | `fn srv_line_ops` | `// md:fn srv_line_ops` |
| 5 | `fn srv_snapshot` | `// md:fn srv_snapshot` |
| 6 | `fn round_trips_through_core` | `// md:fn round_trips_through_core` |
| 7 | `fn the_three_format_limits_are_the_ones_keeplin_core_defines` | `// md:fn the_three_format_limits_are_the_ones_keeplin_core_defines` |
| 8 | `fn protocol_version_matches_keeplin_core` | `// md:fn protocol_version_matches_keeplin_core` |
| 9 | `fn every_client_message_round_trips_against_keeplin_core` | `// md:fn every_client_message_round_trips_against_keeplin_core` |
| 10 | `fn every_server_message_round_trips_against_keeplin_core` | `// md:fn every_server_message_round_trips_against_keeplin_core` |
| 11 | `fn a_limit_rejection_names_the_note_and_an_old_client_still_parses_it` | `// md:fn a_limit_rejection_names_the_note_and_an_old_client_still_parses_it` |
