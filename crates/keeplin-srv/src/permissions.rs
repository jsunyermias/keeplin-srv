use uuid::Uuid;

use crate::{error::AppError, store::Note};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Editor,
    Viewer,
}

impl Role {
    /// Owners and editors may send `Op`s; viewers may only join and watch.
    pub fn can_write(self) -> bool {
        matches!(self, Role::Owner | Role::Editor)
    }

    /// Only the owner may share, revoke and delete the note.
    pub fn can_share(self) -> bool {
        matches!(self, Role::Owner)
    }
}

/// Resolve `user_id`'s role on `note`, or `Forbidden` if they have none.
pub async fn resolve_role(
    store: &crate::store::Store,
    note: &Note,
    user_id: Uuid,
) -> Result<Role, AppError> {
    if note.owner_id == user_id {
        return Ok(Role::Owner);
    }

    match store.get_share(note.id, user_id).await? {
        Some(share) => match share.role.as_str() {
            "editor" => Ok(Role::Editor),
            "viewer" => Ok(Role::Viewer),
            _ => Err(AppError::Internal("unknown share role".into())),
        },
        None => Err(AppError::Forbidden),
    }
}
