use uuid::Uuid;

use crate::{error::AppError, store::Note};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Editor,
    Viewer,
}

impl Role {
    pub fn can_write(self) -> bool {
        matches!(self, Role::Owner | Role::Editor)
    }

    pub fn can_share(self) -> bool {
        matches!(self, Role::Owner)
    }
}

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
