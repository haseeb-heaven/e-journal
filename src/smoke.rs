//! Process-level smoke test used by the end-to-end test suite.
//!
//! This runs without opening a window. It exercises the same storage and
//! backup paths as the desktop app against a disposable directory.

use std::path::PathBuf;

use thiserror::Error;

use crate::backup::{self, BackupError};
use crate::model::{EntryDraft, ValidationError};
use crate::storage::{Storage, StorageError};

#[derive(Debug, Error)]
pub enum SmokeError {
    #[error("Set E_JOURNAL_SMOKE_DIR to a disposable directory before running --smoke-test.")]
    MissingDirectory,

    #[error(transparent)]
    Storage(#[from] StorageError),

    #[error(transparent)]
    Backup(#[from] BackupError),

    #[error(transparent)]
    Validation(#[from] ValidationError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("Smoke check failed: {0}")]
    Check(String),
}

/// Run the full create/update/export/delete/import/reopen workflow.
///
/// Returns the number of entries restored from the backup.
pub fn run() -> Result<usize, SmokeError> {
    let parent = std::env::var_os("E_JOURNAL_SMOKE_DIR")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or(SmokeError::MissingDirectory)?;

    let work_dir = parent.join("e-journal-smoke");
    if work_dir.exists() {
        std::fs::remove_dir_all(&work_dir)?;
    }
    std::fs::create_dir_all(&work_dir)?;

    let db_path = work_dir.join("smoke.sqlite3");
    let backup_path = work_dir.join("backup.json");
    let mut storage = Storage::open(&db_path)?;

    let created = storage.create_entry(&draft(
        "Smoke entry",
        "Created by the end-to-end smoke test.",
        "smoke, e2e",
    )?)?;
    if created.tags != ["e2e", "smoke"] {
        return Err(SmokeError::Check(format!(
            "unexpected tags after create: {:?}",
            created.tags
        )));
    }

    let updated = storage.update_entry(
        created.id,
        &draft(
            "Smoke entry updated",
            "Updated body for the end-to-end smoke test.",
            "smoke, e2e, verified",
        )?,
    )?;
    if updated.created_at != created.created_at {
        return Err(SmokeError::Check(
            "update changed the creation timestamp".to_owned(),
        ));
    }

    let entries = storage.list_entries()?;
    if entries.len() != 1 || entries[0].title != "Smoke entry updated" {
        return Err(SmokeError::Check(
            "updated entry was not returned by list_entries".to_owned(),
        ));
    }

    storage.set_setting("theme", "slate")?;
    if storage.get_setting("theme")?.as_deref() != Some("slate") {
        return Err(SmokeError::Check("setting was not persisted".to_owned()));
    }

    backup::write_backup(&backup_path, &entries)?;
    let restored = backup::read_backup(&backup_path)?;
    if restored != entries {
        return Err(SmokeError::Check(
            "backup round-trip changed the entry".to_owned(),
        ));
    }

    if !storage.delete_entry(created.id)? || !storage.list_entries()?.is_empty() {
        return Err(SmokeError::Check(
            "entry was not deleted from the database".to_owned(),
        ));
    }

    backup::apply_import(&mut storage, &restored)?;
    if storage.list_entries()? != restored {
        return Err(SmokeError::Check(
            "import did not restore the backup".to_owned(),
        ));
    }

    drop(storage);
    let reopened = Storage::open(&db_path)?;
    if reopened.list_entries()? != restored {
        return Err(SmokeError::Check(
            "entries did not persist after reopening the database".to_owned(),
        ));
    }
    if reopened.get_setting("theme")?.as_deref() != Some("slate") {
        return Err(SmokeError::Check(
            "setting did not persist after reopening the database".to_owned(),
        ));
    }

    Ok(restored.len())
}

fn draft(title: &str, body: &str, tags: &str) -> Result<crate::model::ValidatedEntry, SmokeError> {
    Ok(EntryDraft {
        title: title.to_owned(),
        body: body.to_owned(),
        tags_text: tags.to_owned(),
    }
    .validate()?)
}
