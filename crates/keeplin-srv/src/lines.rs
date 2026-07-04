use chrono::{DateTime, Utc};
use keeplin_core::storage::note_log::{increment, resolve, VersionVector, Winner};
use uuid::Uuid;

use crate::{
    error::AppError,
    positions,
    store::{Line, NoteLine, Store},
};

#[derive(Debug, Clone)]
pub enum ApplyOutcome<T> {
    Applied(T),
    Rejected { current: T },
}

fn next_vv(base: &VersionVector, device_id: &str) -> VersionVector {
    let mut vv = base.clone();
    increment(&mut vv, device_id);
    vv
}

pub async fn insert_line(
    store: &Store,
    note_id: Uuid,
    after_line_id: Option<Uuid>,
    content: &str,
    base_vv: &VersionVector,
    device_id: &str,
) -> Result<(Line, NoteLine), AppError> {
    let (prev, next) = store.get_adjacent_positions(note_id, after_line_id).await?;
    let position = positions::between(prev.as_deref(), next.as_deref());

    let vv = next_vv(base_vv, device_id);
    let line_id = Uuid::new_v4();
    let line = store.create_line(line_id, content, &vv, device_id).await?;
    let note_line = store
        .link_line(note_id, line_id, &position, &vv, device_id)
        .await?;

    Ok((line, note_line))
}

pub async fn update_line(
    store: &Store,
    note_id: Uuid,
    line_id: Uuid,
    content: &str,
    incoming_vv: &VersionVector,
    incoming_ts: DateTime<Utc>,
    device_id: &str,
) -> Result<ApplyOutcome<Line>, AppError> {
    let line = store.get_line(line_id).await?;
    let line = line.ok_or(AppError::NotFound)?;

    // Verify the line actually belongs to the note.
    let _note_line = store
        .get_note_line(note_id, line_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let winner = resolve(
        &line.vv.0,
        line.updated_at,
        &line.last_writer,
        incoming_vv,
        incoming_ts,
        device_id,
    );

    match winner {
        Winner::Incoming => {
            let new_vv = next_vv(incoming_vv, device_id);
            let updated = store
                .update_line(line_id, content, &new_vv, device_id)
                .await?
                .ok_or(AppError::NotFound)?;
            Ok(ApplyOutcome::Applied(updated))
        }
        Winner::Local => Ok(ApplyOutcome::Rejected { current: line }),
    }
}

pub async fn delete_line(
    store: &Store,
    note_id: Uuid,
    line_id: Uuid,
    incoming_vv: &VersionVector,
    incoming_ts: DateTime<Utc>,
    device_id: &str,
) -> Result<ApplyOutcome<Line>, AppError> {
    let line = store.get_line(line_id).await?;
    let line = line.ok_or(AppError::NotFound)?;

    let _note_line = store
        .get_note_line(note_id, line_id)
        .await?
        .ok_or(AppError::NotFound)?;

    let winner = resolve(
        &line.vv.0,
        line.updated_at,
        &line.last_writer,
        incoming_vv,
        incoming_ts,
        device_id,
    );

    match winner {
        Winner::Incoming => {
            let new_vv = next_vv(incoming_vv, device_id);
            let updated = store
                .soft_delete_line(line_id, &new_vv, device_id)
                .await?
                .ok_or(AppError::NotFound)?;
            Ok(ApplyOutcome::Applied(updated))
        }
        Winner::Local => Ok(ApplyOutcome::Rejected { current: line }),
    }
}

#[derive(Debug, Clone)]
pub struct LineMove {
    pub line_id: Uuid,
    pub after_line_id: Option<Uuid>,
}

pub async fn move_lines(
    store: &Store,
    note_id: Uuid,
    moves: &[LineMove],
    incoming_vv: &VersionVector,
    incoming_ts: DateTime<Utc>,
    device_id: &str,
) -> Result<Vec<ApplyOutcome<(NoteLine, Option<Uuid>)>>, AppError> {
    let mut outcomes = Vec::with_capacity(moves.len());

    for m in moves {
        let note_line = store.get_note_line(note_id, m.line_id).await?;
        let note_line = match note_line {
            Some(nl) => nl,
            None => continue,
        };

        let winner = resolve(
            &note_line.vv.0,
            note_line.updated_at,
            &note_line.last_writer,
            incoming_vv,
            incoming_ts,
            device_id,
        );

        let outcome = match winner {
            Winner::Incoming => {
                let after = m.after_line_id;
                let (prev, next) = store.get_adjacent_positions(note_id, after).await?;
                // Exclude the line being moved from the next-position calculation.
                let next = if next.as_deref() == Some(note_line.position.as_str()) {
                    // Find the following line after the moved one.
                    let rows = store.list_note_lines_active(note_id).await?;
                    let idx = rows.iter().position(|(nl, _)| nl.line_id == m.line_id);
                    idx.and_then(|i| rows.get(i + 1).map(|(nl, _)| nl.position.clone()))
                } else {
                    next
                };
                let position = positions::between(prev.as_deref(), next.as_deref());
                let new_vv = next_vv(incoming_vv, device_id);
                let updated = store
                    .update_note_line_position(note_id, m.line_id, &position, &new_vv, device_id)
                    .await?
                    .ok_or(AppError::NotFound)?;
                ApplyOutcome::Applied((updated, after))
            }
            Winner::Local => ApplyOutcome::Rejected {
                current: (note_line, m.after_line_id),
            },
        };
        outcomes.push(outcome);
    }

    Ok(outcomes)
}
