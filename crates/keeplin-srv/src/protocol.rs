//! The collaborative-editing wire protocol, as specified in the Keeplin
//! server-mode design: the unit of concurrency is the **line**, every
//! operation carries its own version vector / writer / timestamp, and the
//! order of lines is a versioned entity of its own.
//!
//! All messages travel over `GET /api/ws?token=<jwt>` as JSON with an
//! internally tagged `type` field.

use chrono::{DateTime, Utc};
use keeplin_core::storage::note_log::VersionVector;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type LineId = Uuid;
/// The concurrency actor in server mode is the **user** (not the device):
/// `last_writer` and version-vector components are user ids.
pub type UserId = String;

/// A caret position inside a note: which line, and the column within it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub line_id: LineId,
    pub column: usize,
}

/// One line as sent inside snapshots: the full versioned entity, tombstones
/// included.
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

/// The state a client needs to render a note: the versioned order plus every
/// line (live and tombstoned). Sent in `Welcome`; a reconnecting client
/// rebuilds from this instead of replaying an op log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoteLinesSnapshot {
    pub note_id: Uuid,
    /// Current order of ALL LineIds, tombstoned lines included.
    pub order: Vec<LineId>,
    pub updated_at: DateTime<Utc>,
    pub vv: VersionVector,
    pub last_writer: UserId,
    pub lines: Vec<LineSnapshot>,
}

/// One line-level operation. Each op carries its own `vv`, `last_writer` and
/// `updated_at`; the server resolves it against the current entity state with
/// `note_log::resolve` and either applies or ignores it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "PascalCase")]
pub enum LineOp {
    Insert {
        /// `None` = insert at the beginning.
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

impl LineOp {
    pub fn last_writer(&self) -> &str {
        match self {
            LineOp::Insert { last_writer, .. }
            | LineOp::Update { last_writer, .. }
            | LineOp::Delete { last_writer, .. }
            | LineOp::Move { last_writer, .. } => last_writer,
        }
    }
}

/// Who is inside a note's session right now, and where their caret is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresenceInfo {
    pub user_id: UserId,
    pub display_name: String,
    pub cursor: Option<Cursor>,
}

/// Client → server messages. One WebSocket connection can join any number of
/// notes; every message names the note it targets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum CollabClientMsg {
    Join {
        note_id: Uuid,
    },
    Leave {
        note_id: Uuid,
    },
    Op {
        note_id: Uuid,
        ops: Vec<LineOp>,
    },
    Cursor {
        note_id: Uuid,
        cursor: Cursor,
    },
    /// Client-side delivery bookkeeping; the server accepts and ignores it.
    Ack {
        server_seq: u64,
    },
}

/// Server → client messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum CollabServerMsg {
    /// Reply to a successful `Join`: the full current state of the note.
    Welcome {
        note_id: Uuid,
        snapshot: NoteLinesSnapshot,
    },
    /// Operations from another participant, already validated, resolved and
    /// persisted by the server. `note_id` is included (a deliberate addition
    /// to the design sketch) so one connection can multiplex several notes.
    Op {
        server_seq: u64,
        note_id: Uuid,
        user_id: UserId,
        ops: Vec<LineOp>,
    },
    /// Full presence list for a note; sent after every join/leave/cursor move.
    Presence {
        note_id: Uuid,
        users: Vec<PresenceInfo>,
    },
    Error {
        code: String,
        message: String,
    },
}
