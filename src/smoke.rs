//! Process-level smoke test used by the end-to-end test suite.
//!
//! This runs without opening a window. It exercises the same storage and
//! backup paths as the desktop app against a disposable directory.

use std::path::PathBuf;

use thiserror::Error;

use crate::backup::{self, BackupError};
use crate::model::{EntryDraft, SearchFilter, ValidationError};
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

    std::fs::create_dir_all(&parent)?;
    let work_dir = create_work_dir(&parent)?;

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
        created.updated_at,
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

    let matches = storage.search(&SearchFilter {
        query: "updated".to_owned(),
        tag: Some("verified".to_owned()),
    })?;
    if matches.len() != 1 || matches[0].id != updated.id {
        return Err(SmokeError::Check(
            "search did not return the updated entry".to_owned(),
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

fn create_work_dir(parent: &std::path::Path) -> Result<PathBuf, SmokeError> {
    for attempt in 0..100 {
        let name = if attempt == 0 {
            format!("e-journal-smoke-{}", std::process::id())
        } else {
            format!("e-journal-smoke-{}-{attempt}", std::process::id())
        };
        let candidate = parent.join(name);
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(SmokeError::Check(
        "could not create a unique smoke-test directory".to_owned(),
    ))
}

fn draft(title: &str, body: &str, tags: &str) -> Result<crate::model::ValidatedEntry, SmokeError> {
    Ok(EntryDraft {
        title: title.to_owned(),
        body: body.to_owned(),
        tags_text: tags.to_owned(),
    }
    .validate()?)
}
