use chrono::{DateTime, Utc};
use keeplin_core::storage::note_log::VersionVector;
use sqlx::{types::Json, Pool, Postgres, Row};
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct UserDevice {
    pub id: Uuid,
    pub user_id: Uuid,
    pub device_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct Note {
    pub id: Uuid,
    pub title: String,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct Line {
    pub id: Uuid,
    pub content: String,
    pub vv: Json<VersionVector>,
    pub last_writer: String,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct NoteLine {
    pub note_id: Uuid,
    pub line_id: Uuid,
    pub position: String,
    pub vv: Json<VersionVector>,
    pub last_writer: String,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct NoteShare {
    pub note_id: Uuid,
    pub user_id: Uuid,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Store {
    pool: Pool<Postgres>,
}

impl Store {
    pub fn new(pool: Pool<Postgres>) -> Self {
        Self { pool }
    }

    // Users

    pub async fn create_user(
        &self,
        email: &str,
        password_hash: &str,
    ) -> Result<User, AppError> {
        let id = Uuid::new_v4();
        let user = sqlx::query_as::<_, User>(
            r#"
            INSERT INTO users (id, email, password_hash)
            VALUES ($1, $2, $3)
            RETURNING id, email, password_hash, created_at
            "#,
        )
        .bind(id)
        .bind(email)
        .bind(password_hash)
        .fetch_one(&self.pool)
        .await?;

        Ok(user)
    }

    pub async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        let user = sqlx::query_as::<_, User>(
            "SELECT id, email, password_hash, created_at FROM users WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;

        Ok(user)
    }

    pub async fn get_user_by_id(&self, id: Uuid) -> Result<Option<User>, AppError> {
        let user = sqlx::query_as::<_, User>(
            "SELECT id, email, password_hash, created_at FROM users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(user)
    }

    // Devices

    pub async fn create_device(
        &self,
        user_id: Uuid,
        device_name: &str,
    ) -> Result<UserDevice, AppError> {
        let id = Uuid::new_v4();
        let device = sqlx::query_as::<_, UserDevice>(
            r#"
            INSERT INTO user_devices (id, user_id, device_name)
            VALUES ($1, $2, $3)
            RETURNING id, user_id, device_name, created_at
            "#,
        )
        .bind(id)
        .bind(user_id)
        .bind(device_name)
        .fetch_one(&self.pool)
        .await?;

        Ok(device)
    }

    pub async fn get_device(&self, id: Uuid) -> Result<Option<UserDevice>, AppError> {
        let device = sqlx::query_as::<_, UserDevice>(
            "SELECT id, user_id, device_name, created_at FROM user_devices WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(device)
    }

    pub async fn list_devices_by_user(&self, user_id: Uuid) -> Result<Vec<UserDevice>, AppError> {
        let devices = sqlx::query_as::<_, UserDevice>(
            "SELECT id, user_id, device_name, created_at FROM user_devices WHERE user_id = $1 ORDER BY created_at",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(devices)
    }

    // Notes

    pub async fn create_note(&self, title: &str, owner_id: Uuid) -> Result<Note, AppError> {
        let id = Uuid::new_v4();
        let now = Utc::now();
        let note = sqlx::query_as::<_, Note>(
            r#"
            INSERT INTO notes (id, title, owner_id, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $4)
            RETURNING id, title, owner_id, created_at, updated_at, deleted_at
            "#,
        )
        .bind(id)
        .bind(title)
        .bind(owner_id)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;

        Ok(note)
    }

    pub async fn get_note(&self, id: Uuid) -> Result<Option<Note>, AppError> {
        let note = sqlx::query_as::<_, Note>(
            "SELECT id, title, owner_id, created_at, updated_at, deleted_at FROM notes WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(note)
    }

    pub async fn update_note_title(
        &self,
        id: Uuid,
        title: &str,
    ) -> Result<Option<Note>, AppError> {
        let now = Utc::now();
        let note = sqlx::query_as::<_, Note>(
            r#"
            UPDATE notes
            SET title = $2, updated_at = $3
            WHERE id = $1
            RETURNING id, title, owner_id, created_at, updated_at, deleted_at
            "#,
        )
        .bind(id)
        .bind(title)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        Ok(note)
    }

    pub async fn soft_delete_note(&self, id: Uuid) -> Result<Option<Note>, AppError> {
        let now = Utc::now();
        let note = sqlx::query_as::<_, Note>(
            r#"
            UPDATE notes
            SET deleted_at = $2, updated_at = $2
            WHERE id = $1 AND deleted_at IS NULL
            RETURNING id, title, owner_id, created_at, updated_at, deleted_at
            "#,
        )
        .bind(id)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        Ok(note)
    }

    pub async fn list_notes_for_user(&self, user_id: Uuid) -> Result<Vec<Note>, AppError> {
        let notes = sqlx::query_as::<_, Note>(
            r#"
            SELECT n.id, n.title, n.owner_id, n.created_at, n.updated_at, n.deleted_at
            FROM notes n
            LEFT JOIN note_shares ns ON ns.note_id = n.id AND ns.user_id = $1
            WHERE n.deleted_at IS NULL
              AND (n.owner_id = $1 OR ns.user_id IS NOT NULL)
            ORDER BY n.updated_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(notes)
    }

    // Shares

    pub async fn create_or_update_share(
        &self,
        note_id: Uuid,
        user_id: Uuid,
        role: &str,
    ) -> Result<NoteShare, AppError> {
        let share = sqlx::query_as::<_, NoteShare>(
            r#"
            INSERT INTO note_shares (note_id, user_id, role)
            VALUES ($1, $2, $3)
            ON CONFLICT (note_id, user_id) DO UPDATE SET role = EXCLUDED.role
            RETURNING note_id, user_id, role, created_at
            "#,
        )
        .bind(note_id)
        .bind(user_id)
        .bind(role)
        .fetch_one(&self.pool)
        .await?;

        Ok(share)
    }

    pub async fn get_share(
        &self,
        note_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<NoteShare>, AppError> {
        let share = sqlx::query_as::<_, NoteShare>(
            "SELECT note_id, user_id, role, created_at FROM note_shares WHERE note_id = $1 AND user_id = $2",
        )
        .bind(note_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(share)
    }

    pub async fn delete_share(&self, note_id: Uuid, user_id: Uuid) -> Result<(), AppError> {
        sqlx::query("DELETE FROM note_shares WHERE note_id = $1 AND user_id = $2")
            .bind(note_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    // Lines

    pub async fn create_line(
        &self,
        id: Uuid,
        content: &str,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<Line, AppError> {
        let now = Utc::now();
        let line = sqlx::query_as::<_, Line>(
            r#"
            INSERT INTO lines (id, content, vv, last_writer, updated_at)
            VALUES ($1, $2, $3, $4, $5)
            RETURNING id, content, vv, last_writer, updated_at, deleted_at
            "#,
        )
        .bind(id)
        .bind(content)
        .bind(Json(vv.clone()))
        .bind(last_writer)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;

        Ok(line)
    }

    pub async fn get_line(&self, id: Uuid) -> Result<Option<Line>, AppError> {
        let line = sqlx::query_as::<_, Line>(
            "SELECT id, content, vv, last_writer, updated_at, deleted_at FROM lines WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(line)
    }

    pub async fn update_line(
        &self,
        id: Uuid,
        content: &str,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<Option<Line>, AppError> {
        let now = Utc::now();
        let line = sqlx::query_as::<_, Line>(
            r#"
            UPDATE lines
            SET content = $2, vv = $3, last_writer = $4, updated_at = $5
            WHERE id = $1
            RETURNING id, content, vv, last_writer, updated_at, deleted_at
            "#,
        )
        .bind(id)
        .bind(content)
        .bind(Json(vv.clone()))
        .bind(last_writer)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        Ok(line)
    }

    pub async fn soft_delete_line(
        &self,
        id: Uuid,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<Option<Line>, AppError> {
        let now = Utc::now();
        let line = sqlx::query_as::<_, Line>(
            r#"
            UPDATE lines
            SET deleted_at = $2, vv = $3, last_writer = $4, updated_at = $2
            WHERE id = $1
            RETURNING id, content, vv, last_writer, updated_at, deleted_at
            "#,
        )
        .bind(id)
        .bind(now)
        .bind(Json(vv.clone()))
        .bind(last_writer)
        .fetch_optional(&self.pool)
        .await?;

        Ok(line)
    }

    // NoteLines

    pub async fn link_line(
        &self,
        note_id: Uuid,
        line_id: Uuid,
        position: &str,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<NoteLine, AppError> {
        let now = Utc::now();
        let nl = sqlx::query_as::<_, NoteLine>(
            r#"
            INSERT INTO note_lines (note_id, line_id, position, vv, last_writer, updated_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING note_id, line_id, position, vv, last_writer, updated_at, deleted_at
            "#,
        )
        .bind(note_id)
        .bind(line_id)
        .bind(position)
        .bind(Json(vv.clone()))
        .bind(last_writer)
        .bind(now)
        .fetch_one(&self.pool)
        .await?;

        Ok(nl)
    }

    pub async fn get_note_line(
        &self,
        note_id: Uuid,
        line_id: Uuid,
    ) -> Result<Option<NoteLine>, AppError> {
        let nl = sqlx::query_as::<_, NoteLine>(
            "SELECT note_id, line_id, position, vv, last_writer, updated_at, deleted_at FROM note_lines WHERE note_id = $1 AND line_id = $2",
        )
        .bind(note_id)
        .bind(line_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(nl)
    }

    pub async fn update_note_line_position(
        &self,
        note_id: Uuid,
        line_id: Uuid,
        position: &str,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<Option<NoteLine>, AppError> {
        let now = Utc::now();
        let nl = sqlx::query_as::<_, NoteLine>(
            r#"
            UPDATE note_lines
            SET position = $3, vv = $4, last_writer = $5, updated_at = $6
            WHERE note_id = $1 AND line_id = $2
            RETURNING note_id, line_id, position, vv, last_writer, updated_at, deleted_at
            "#,
        )
        .bind(note_id)
        .bind(line_id)
        .bind(position)
        .bind(Json(vv.clone()))
        .bind(last_writer)
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;

        Ok(nl)
    }

    pub async fn soft_delete_note_line(
        &self,
        note_id: Uuid,
        line_id: Uuid,
        vv: &VersionVector,
        last_writer: &str,
    ) -> Result<Option<NoteLine>, AppError> {
        let now = Utc::now();
        let nl = sqlx::query_as::<_, NoteLine>(
            r#"
            UPDATE note_lines
            SET deleted_at = $3, vv = $4, last_writer = $5, updated_at = $3
            WHERE note_id = $1 AND line_id = $2
            RETURNING note_id, line_id, position, vv, last_writer, updated_at, deleted_at
            "#,
        )
        .bind(note_id)
        .bind(line_id)
        .bind(now)
        .bind(Json(vv.clone()))
        .bind(last_writer)
        .fetch_optional(&self.pool)
        .await?;

        Ok(nl)
    }

    pub async fn list_note_lines(
        &self,
        note_id: Uuid,
    ) -> Result<Vec<(NoteLine, Line)>, AppError> {
        let rows = sqlx::query(
            r#"
            SELECT nl.note_id, nl.line_id, nl.position, nl.vv as nl_vv, nl.last_writer as nl_last_writer, nl.updated_at as nl_updated_at, nl.deleted_at as nl_deleted_at,
                   l.id, l.content, l.vv as l_vv, l.last_writer as l_last_writer, l.updated_at as l_updated_at, l.deleted_at as l_deleted_at
            FROM note_lines nl
            JOIN lines l ON l.id = nl.line_id
            WHERE nl.note_id = $1
            ORDER BY nl.position, nl.line_id
            "#,
        )
        .bind(note_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|r| Ok((map_note_line(&r)?, map_line(&r)?)))
            .collect()
    }

    pub async fn list_note_lines_active(
        &self,
        note_id: Uuid,
    ) -> Result<Vec<(NoteLine, Line)>, AppError> {
        let rows = sqlx::query(
            r#"
            SELECT nl.note_id, nl.line_id, nl.position, nl.vv as nl_vv, nl.last_writer as nl_last_writer, nl.updated_at as nl_updated_at, nl.deleted_at as nl_deleted_at,
                   l.id, l.content, l.vv as l_vv, l.last_writer as l_last_writer, l.updated_at as l_updated_at, l.deleted_at as l_deleted_at
            FROM note_lines nl
            JOIN lines l ON l.id = nl.line_id
            WHERE nl.note_id = $1 AND nl.deleted_at IS NULL AND l.deleted_at IS NULL
            ORDER BY nl.position, nl.line_id
            "#,
        )
        .bind(note_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|r| Ok((map_note_line(&r)?, map_line(&r)?)))
            .collect()
    }

    pub async fn get_adjacent_positions(
        &self,
        note_id: Uuid,
        after_line_id: Option<Uuid>,
    ) -> Result<(Option<String>, Option<String>), AppError> {
        if let Some(after) = after_line_id {
            let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
                r#"
                SELECT
                    (SELECT position FROM note_lines WHERE note_id = $1 AND line_id = $2 AND deleted_at IS NULL) as prev,
                    (SELECT position FROM note_lines WHERE note_id = $1 AND deleted_at IS NULL AND position > (SELECT position FROM note_lines WHERE note_id = $1 AND line_id = $2) ORDER BY position, line_id LIMIT 1) as next
                "#,
            )
            .bind(note_id)
            .bind(after)
            .fetch_one(&self.pool)
            .await?;
            Ok(row)
        } else {
            let next = sqlx::query_scalar::<_, Option<String>>(
                "SELECT position FROM note_lines WHERE note_id = $1 AND deleted_at IS NULL ORDER BY position, line_id LIMIT 1",
            )
            .bind(note_id)
            .fetch_optional(&self.pool)
            .await?;
            Ok((None, next.flatten()))
        }
    }
}

fn map_note_line(row: &sqlx::postgres::PgRow) -> Result<NoteLine, sqlx::Error> {
    Ok(NoteLine {
        note_id: row.try_get("note_id")?,
        line_id: row.try_get("line_id")?,
        position: row.try_get("position")?,
        vv: row.try_get::<Json<VersionVector>, _>("nl_vv")?,
        last_writer: row.try_get("nl_last_writer")?,
        updated_at: row.try_get("nl_updated_at")?,
        deleted_at: row.try_get("nl_deleted_at")?,
    })
}

fn map_line(row: &sqlx::postgres::PgRow) -> Result<Line, sqlx::Error> {
    Ok(Line {
        id: row.try_get("id")?,
        content: row.try_get("content")?,
        vv: row.try_get::<Json<VersionVector>, _>("l_vv")?,
        last_writer: row.try_get("l_last_writer")?,
        updated_at: row.try_get("l_updated_at")?,
        deleted_at: row.try_get("l_deleted_at")?,
    })
}
