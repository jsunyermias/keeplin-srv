use chrono::{DateTime, Utc};
use keeplin_core::storage::note_log::VersionVector;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: Uuid,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineSnapshot {
    pub line_id: Uuid,
    pub position: String,
    pub content: String,
    pub vv: VersionVector,
    pub last_writer: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineMoveMsg {
    pub line_id: Uuid,
    pub after_line_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum ClientMessage {
    InsertLine {
        note_id: Uuid,
        line_id: Uuid,
        after_line_id: Option<Uuid>,
        content: String,
        vv: VersionVector,
        device_id: String,
        ts: DateTime<Utc>,
    },
    UpdateLine {
        note_id: Uuid,
        line_id: Uuid,
        content: String,
        vv: VersionVector,
        device_id: String,
        ts: DateTime<Utc>,
    },
    DeleteLine {
        note_id: Uuid,
        line_id: Uuid,
        vv: VersionVector,
        device_id: String,
        ts: DateTime<Utc>,
    },
    MoveLines {
        note_id: Uuid,
        moves: Vec<LineMoveMsg>,
        vv: VersionVector,
        device_id: String,
        ts: DateTime<Utc>,
    },
    Cursor {
        note_id: Uuid,
        line_id: Uuid,
        column: u32,
    },
    Presence {
        note_id: Uuid,
        status: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum ServerMessage {
    Snapshot {
        note_id: Uuid,
        lines: Vec<LineSnapshot>,
    },
    InsertLine {
        note_id: Uuid,
        line_id: Uuid,
        after_line_id: Option<Uuid>,
        content: String,
        vv: VersionVector,
        device_id: String,
        ts: DateTime<Utc>,
    },
    UpdateLine {
        note_id: Uuid,
        line_id: Uuid,
        content: String,
        vv: VersionVector,
        device_id: String,
        ts: DateTime<Utc>,
    },
    DeleteLine {
        note_id: Uuid,
        line_id: Uuid,
        vv: VersionVector,
        device_id: String,
        ts: DateTime<Utc>,
    },
    MoveLines {
        note_id: Uuid,
        moves: Vec<LineMoveMsg>,
        vv: VersionVector,
        device_id: String,
        ts: DateTime<Utc>,
    },
    Cursor {
        note_id: Uuid,
        line_id: Uuid,
        column: u32,
        user: UserInfo,
    },
    Presence {
        note_id: Uuid,
        status: String,
        user: UserInfo,
    },
    Rejected {
        reason: String,
    },
}
