//! Moving a journal in and out of the app as a backup file.
//!
//! The desktop build writes to a path chosen in a native file dialog. The
//! browser build downloads the file and reads it back through a file input.
//! Both paths share the same JSON document format, so a backup written on one
//! platform restores on the other.

use crate::backup::{self, BackupError};
use crate::model::JournalEntry;

/// The outcome of an export request.
pub enum ExportOutcome {
    /// The user chose a destination and the backup was written.
    Exported { name: String },
    /// The user cancelled the dialog.
    Cancelled,
}

/// The outcome of an import request.
pub enum ImportOutcome {
    /// The user chose a file and it was validated.
    Imported {
        entries: Vec<JournalEntry>,
        name: String,
    },
    /// The user cancelled the dialog.
    Cancelled,
}

/// Export the journal to a backup file the user picks.
#[cfg(not(target_arch = "wasm32"))]
pub fn export(
    entries: &[JournalEntry],
    default_name: &str,
    database_path: &std::path::Path,
) -> Result<ExportOutcome, BackupError> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Journal backup", &["json"])
        .set_file_name(default_name.to_owned())
        .set_title("Export journal backup")
        .save_file()
    else {
        return Ok(ExportOutcome::Cancelled);
    };

    if crate::ui::export_destination_is_database(&path, database_path) {
        return Err(BackupError::ReadFile {
            path: path.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "choose a different destination",
            ),
        });
    }

    backup::write_backup(&path, entries)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(default_name)
        .to_owned();
    Ok(ExportOutcome::Exported { name })
}

/// Read a backup file the user picks.
#[cfg(not(target_arch = "wasm32"))]
pub fn import() -> Result<ImportOutcome, BackupError> {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Journal backup", &["json"])
        .set_title("Import journal backup")
        .pick_file()
    else {
        return Ok(ImportOutcome::Cancelled);
    };

    let entries = backup::read_backup(&path)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("backup")
        .to_owned();
    Ok(ImportOutcome::Imported { entries, name })
}

/// Export the journal to a backup file the user picks.
#[cfg(target_arch = "wasm32")]
pub fn export(
    entries: &[JournalEntry],
    default_name: &str,
    _database_path: &std::path::Path,
) -> Result<ExportOutcome, BackupError> {
    let json = backup::export_to_json(entries)?;
    crate::web::download_text(default_name, &json)
        .map_err(|error| BackupError::Browser(format!("{error:?}")))?;
    Ok(ExportOutcome::Exported {
        name: default_name.to_owned(),
    })
}

/// Read a backup file the user picks.
#[cfg(target_arch = "wasm32")]
pub fn import() -> impl std::future::Future<Output = Result<ImportOutcome, BackupError>> {
    import_async()
}

/// Read a backup file the user picks.
#[cfg(target_arch = "wasm32")]
pub async fn import_async() -> Result<ImportOutcome, BackupError> {
    let Some((name, json)) = crate::web::pick_text_file(".json")
        .await
        .map_err(|error| BackupError::Browser(format!("{error:?}")))?
    else {
        return Ok(ImportOutcome::Cancelled);
    };
    let entries = backup::parse_json(&json)?;
    Ok(ImportOutcome::Imported { entries, name })
}
