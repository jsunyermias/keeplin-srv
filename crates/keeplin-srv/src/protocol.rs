// md:Overview
use chrono::{DateTime, Utc};
use keeplin_core::storage::note_log::VersionVector;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// md:Type aliases > LineId
pub type LineId = Uuid;

// md:Type aliases > UserId
pub type UserId = String;

// md:Cursor
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub line_id: LineId,
    pub column: usize,
}

// md:LineSnapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineSnapshot {
    pub id: LineId,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
    pub vv: VersionVector,
    pub last_writer: UserId,
}

// md:NoteLinesSnapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteLinesSnapshot {
    pub note_id: Uuid,
    pub order: Vec<LineId>,
    pub updated_at: DateTime<Utc>,
    pub vv: VersionVector,
    pub last_writer: UserId,
    pub lines: Vec<LineSnapshot>,
}

// md:LineOp
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "PascalCase")]
pub enum LineOp {
    Insert {
        after_line_id: Option<LineId>,
        line_id: LineId,
        content: String,
        vv: VersionVector,
        last_writer: UserId,
        updated_at: DateTime<Utc>,
    },
    Update {
        line_id: LineId,
        content: String,
        vv: VersionVector,
        last_writer: UserId,
        updated_at: DateTime<Utc>,
    },
    Delete {
        line_id: LineId,
        deleted_at: DateTime<Utc>,
        vv: VersionVector,
        last_writer: UserId,
        updated_at: DateTime<Utc>,
    },
    Move {
        line_ids: Vec<LineId>,
        after_line_id: Option<LineId>,
        vv: VersionVector,
        last_writer: UserId,
        updated_at: DateTime<Utc>,
    },
}

// md:impl LineOp
impl LineOp {
    // md:impl LineOp > fn last_writer
    pub fn last_writer(&self) -> &str {
        match self {
            LineOp::Insert { last_writer, .. }
            | LineOp::Update { last_writer, .. }
            | LineOp::Delete { last_writer, .. }
            | LineOp::Move { last_writer, .. } => last_writer,
        }
    }
}

// md:PresenceInfo
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceInfo {
    pub user_id: UserId,
    pub display_name: String,
    pub cursor: Option<Cursor>,
}

// md:CollabClientMsg
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum CollabClientMsg {
    Join { note_id: Uuid },
    Leave { note_id: Uuid },
    Op { note_id: Uuid, ops: Vec<LineOp> },
    Cursor { note_id: Uuid, cursor: Cursor },
    Ack { server_seq: u64 },
}

// md:CollabServerMsg
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum CollabServerMsg {
    Welcome {
        note_id: Uuid,
        snapshot: NoteLinesSnapshot,
    },
    Op {
        server_seq: u64,
        note_id: Uuid,
        user_id: UserId,
        ops: Vec<LineOp>,
    },
    Presence {
        note_id: Uuid,
        users: Vec<PresenceInfo>,
    },
    Error {
        code: String,
        message: String,
    },
}
