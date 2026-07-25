// md:Overview
use chrono::{TimeZone, Utc};
use keeplin_core::collab::protocol as core_protocol;
use keeplin_core::storage::note_log::VersionVector;
use keeplin_srv::protocol as srv_protocol;
use serde_json::Value;
use uuid::Uuid;

// md:fn fixed_uuid
fn fixed_uuid(byte: u8) -> Uuid {
    Uuid::from_bytes([byte; 16])
}

// md:fn fixed_vv
fn fixed_vv() -> VersionVector {
    VersionVector::from([("device-a".to_string(), 7u64)])
}

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
