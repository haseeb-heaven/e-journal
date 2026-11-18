//! Versioned JSON backups, validation, and transactional restore.
//!
//! A backup is a complete snapshot of the journal. Import replaces the stored
//! entries only after the whole file has been validated, and the replacement
//! itself runs in one SQLite transaction.

use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{format_timestamp, normalize_tags, parse_timestamp, EntryDraft, JournalEntry};
use crate::storage::{Storage, StorageError};

pub const BACKUP_FORMAT: &str = "e-journal-backup";
pub const BACKUP_VERSION: u32 = 1;
pub const MAX_BACKUP_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("Could not read the backup file at {path}: {source}")]
    ReadFile {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Could not write the backup file at {path}: {source}")]
    WriteFile {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[cfg(unix)]
    #[error("The backup was written to {path}, but its folder could not be synced: {source}")]
    SyncDirectory {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("The backup file at {path} is larger than the supported limit of {limit} bytes.")]
    TooLarge { path: String, limit: u64 },

    #[error("This journal is too large to export as one backup (limit {limit} bytes).")]
    ExportTooLarge { limit: u64 },

    #[error("This file is not valid Journal backup JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),

    #[error("Could not encode the backup: {0}")]
    Serialize(serde_json::Error),

    #[error("Unsupported backup format {0:?}. Choose a Journal backup file.")]
    UnsupportedFormat(String),

    #[error("Backup version {found} is not supported. This app can restore version {supported}.")]
    UnsupportedVersion { found: u32, supported: u32 },

    #[error("Entry {index} in the backup is invalid: {message}")]
    InvalidEntry { index: usize, message: String },

    #[error("The backup contains duplicate entry id {0}.")]
    DuplicateId(i64),

    #[error(transparent)]
    Storage(#[from] StorageError),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupFile {
    format: String,
    version: u32,
    exported_at: DateTime<Utc>,
    entries: Vec<BackupEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackupEntry {
    id: i64,
    title: String,
    body: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    tags: Vec<String>,
}

impl From<&JournalEntry> for BackupEntry {
    fn from(entry: &JournalEntry) -> Self {
        Self {
            id: entry.id,
            title: entry.title.clone(),
            body: entry.body.clone(),
            created_at: entry.created_at,
            updated_at: entry.updated_at,
            tags: entry.tags.clone(),
        }
    }
}

impl BackupEntry {
    fn validate(self, index: usize) -> Result<JournalEntry, BackupError> {
        if self.id <= 0 {
            return Err(BackupError::InvalidEntry {
                index,
                message: "entry id must be a positive number".into(),
            });
        }

        for (field, timestamp) in [
            ("created_at", self.created_at),
            ("updated_at", self.updated_at),
        ] {
            let formatted = format_timestamp(timestamp);
            if parse_timestamp(&formatted).ok() != Some(timestamp) {
                return Err(BackupError::InvalidEntry {
                    index,
                    message: format!("{field} is outside the supported timestamp range"),
                });
            }
        }

        let draft = EntryDraft {
            title: self.title,
            body: self.body,
            tags_text: self.tags.join(", "),
        };
        let validated = draft
            .validate()
            .map_err(|error| BackupError::InvalidEntry {
                index,
                message: error.to_string(),
            })?;

        Ok(JournalEntry {
            id: self.id,
            revision: 0,
            title: validated.title,
            body: validated.body,
            created_at: self.created_at,
            updated_at: self.updated_at,
            tags: normalize_tags(&validated.tags.join(", ")),
        })
    }
}

/// Encode all entries as a pretty-printed, versioned JSON backup.
pub fn export_to_string(entries: &[JournalEntry]) -> Result<String, BackupError> {
    let backup = BackupFile {
        format: BACKUP_FORMAT.to_owned(),
        version: BACKUP_VERSION,
        exported_at: Utc::now(),
        entries: entries.iter().map(BackupEntry::from).collect(),
    };
    serde_json::to_string_pretty(&backup).map_err(BackupError::Serialize)
}

/// Validate and decode a backup, returning normalized entries ready to store.
pub fn parse(json: &str) -> Result<Vec<JournalEntry>, BackupError> {
    let backup: BackupFile = serde_json::from_str(json)?;

    if backup.format != BACKUP_FORMAT {
        return Err(BackupError::UnsupportedFormat(backup.format));
    }
    if backup.version != BACKUP_VERSION {
        return Err(BackupError::UnsupportedVersion {
            found: backup.version,
            supported: BACKUP_VERSION,
        });
    }

    let mut ids = HashSet::with_capacity(backup.entries.len());
    let mut entries = Vec::with_capacity(backup.entries.len());
    for (index, backup_entry) in backup.entries.into_iter().enumerate() {
        let entry = backup_entry.validate(index)?;
        if !ids.insert(entry.id) {
            return Err(BackupError::DuplicateId(entry.id));
        }
        entries.push(entry);
    }

    Ok(entries)
}

/// Read and validate a backup file. The journal database is not touched.
pub fn read_backup(path: &Path) -> Result<Vec<JournalEntry>, BackupError> {
    read_backup_with_limit(path, MAX_BACKUP_BYTES)
}

fn read_backup_with_limit(path: &Path, limit: u64) -> Result<Vec<JournalEntry>, BackupError> {
    let file = std::fs::File::open(path).map_err(|source| BackupError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| BackupError::ReadFile {
            path: path.display().to_string(),
            source,
        })?;
    if bytes.len() as u64 > limit {
        return Err(BackupError::TooLarge {
            path: path.display().to_string(),
            limit,
        });
    }
    let json = String::from_utf8(bytes).map_err(|_| BackupError::ReadFile {
        path: path.display().to_string(),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the backup file is not valid UTF-8",
        ),
    })?;
    parse(&json)
}

/// Write a backup file through a private temporary file.
///
/// The destination is replaced only after the complete JSON payload has been
/// written and synced, so a failed export cannot truncate an existing backup.
pub fn write_backup(path: &Path, entries: &[JournalEntry]) -> Result<(), BackupError> {
    write_backup_with_limit(path, entries, MAX_BACKUP_BYTES)
}

fn write_backup_with_limit(
    path: &Path,
    entries: &[JournalEntry],
    limit: u64,
) -> Result<(), BackupError> {
    let json = export_to_string(entries)?;
    if json.len() as u64 > limit {
        return Err(BackupError::ExportTooLarge { limit });
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    let mut temporary = tempfile::Builder::new()
        .prefix(".e-journal-backup-")
        .tempfile_in(parent)
        .map_err(|source| BackupError::WriteFile {
            path: path.display().to_string(),
            source,
        })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(|source| BackupError::WriteFile {
                path: path.display().to_string(),
                source,
            })?;
    }

    temporary
        .write_all(json.as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|source| BackupError::WriteFile {
            path: path.display().to_string(),
            source,
        })?;

    temporary
        .persist(path)
        .map_err(|error| BackupError::WriteFile {
            path: path.display().to_string(),
            source: error.error,
        })?;
    sync_parent_directory(parent, path)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path, path: &Path) -> Result<(), BackupError> {
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| BackupError::SyncDirectory {
            path: path.display().to_string(),
            source,
        })
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path, _path: &Path) -> Result<(), BackupError> {
    Ok(())
}

/// Replace the journal contents with an already-validated backup.
pub fn apply_import(storage: &mut Storage, entries: &[JournalEntry]) -> Result<(), BackupError> {
    storage.replace_all_entries(entries)?;
    Ok(())
}

/// Replace the journal only if it still matches the confirmed revision
/// snapshot.
pub fn apply_import_checked(
    storage: &mut Storage,
    entries: &[JournalEntry],
    expected: &[(crate::model::EntryId, i64)],
) -> Result<(), BackupError> {
    storage.replace_all_entries_checked(entries, expected)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    use super::*;
    use crate::model::EntryDraft;

    fn revisions(storage: &Storage) -> Vec<(crate::model::EntryId, i64)> {
        let mut revisions = storage
            .list_entries()
            .unwrap()
            .into_iter()
            .map(|entry| (entry.id, entry.revision))
            .collect::<Vec<_>>();
        revisions.sort_unstable_by_key(|(id, _)| *id);
        revisions
    }

    fn entry(id: i64, title: &str, tags: &[&str]) -> JournalEntry {
        let created = Utc.with_ymd_and_hms(2026, 11, 18, 9, 30, 0).unwrap();
        JournalEntry {
            id,
            revision: 0,
            title: title.to_owned(),
            body: "A quiet morning.".to_owned(),
            created_at: created,
            updated_at: created,
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        }
    }

    #[test]
    fn write_backup_enforces_a_size_limit() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("too-large.json");

        let error = write_backup_with_limit(&path, &[entry(1, "Large", &[])], 10).unwrap_err();

        assert!(matches!(error, BackupError::ExportTooLarge { limit: 10 }));
        assert!(!path.exists());
    }

    #[test]
    fn read_backup_enforces_a_size_limit() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("oversized.json");
        std::fs::write(&path, "12345678901").unwrap();

        let error = read_backup_with_limit(&path, 10).unwrap_err();

        assert!(matches!(error, BackupError::TooLarge { limit: 10, .. }));
    }

    #[test]
    fn export_and_parse_round_trip() {
        let entries = vec![
            entry(3, "First", &["personal"]),
            entry(8, "Second", &["work", "ideas"]),
        ];
        let json = export_to_string(&entries).unwrap();
        assert!(json.contains(BACKUP_FORMAT));
        assert!(json.contains("\"version\": 1"));
        assert_eq!(parse(&json).unwrap(), entries);
    }

    #[test]
    fn empty_backup_is_valid() {
        let json = export_to_string(&[]).unwrap();
        assert!(parse(&json).unwrap().is_empty());
    }

    #[test]
    fn rejects_unrelated_json_and_malformed_json() {
        let other = r#"{"hello":"world"}"#;
        assert!(parse(other).is_err());
        assert!(parse("{not json").is_err());
    }

    #[test]
    fn rejects_unsupported_format_and_version() {
        let json = export_to_string(&[entry(1, "Title", &[])])
            .unwrap()
            .replace(BACKUP_FORMAT, "some-other-app");
        assert!(matches!(
            parse(&json),
            Err(BackupError::UnsupportedFormat(_))
        ));

        let json = export_to_string(&[entry(1, "Title", &[])])
            .unwrap()
            .replace("\"version\": 1", "\"version\": 2");
        assert!(matches!(
            parse(&json),
            Err(BackupError::UnsupportedVersion {
                found: 2,
                supported: BACKUP_VERSION
            })
        ));
    }

    #[test]
    fn rejects_duplicate_ids_and_invalid_entries() {
        let entries = vec![entry(1, "One", &[]), entry(1, "Two", &[])];
        assert!(matches!(
            parse(&export_to_string(&entries).unwrap()),
            Err(BackupError::DuplicateId(1))
        ));

        let entries = vec![entry(2, "   ", &[])];
        assert!(matches!(
            parse(&export_to_string(&entries).unwrap()),
            Err(BackupError::InvalidEntry { index: 0, .. })
        ));

        let mut negative = entry(1, "Negative", &[]);
        negative.id = -3;
        assert!(matches!(
            parse(&export_to_string(&[negative]).unwrap()),
            Err(BackupError::InvalidEntry { index: 0, .. })
        ));

        let mut null_tag = entry(6, "Null tag", &[]);
        null_tag.tags = vec!["bad\0tag".into()];
        assert!(matches!(
            parse(&export_to_string(&[null_tag]).unwrap()),
            Err(BackupError::InvalidEntry { index: 0, .. })
        ));
    }

    #[test]
    fn timestamps_that_cannot_round_trip_are_rejected() {
        let extreme = DateTime::parse_from_rfc3339("9999-12-31T23:59:59-14:00")
            .unwrap()
            .with_timezone(&Utc);
        let mut invalid = entry(4, "Time", &[]);
        invalid.created_at = extreme;
        invalid.updated_at = extreme;
        assert!(matches!(
            parse(&export_to_string(&[invalid]).unwrap()),
            Err(BackupError::InvalidEntry { index: 0, .. })
        ));
    }

    #[test]
    fn historical_timestamps_are_accepted() {
        let mut backdated = entry(4, "Time", &[]);
        backdated.updated_at = backdated.created_at - chrono::Duration::days(1);
        let parsed = parse(&export_to_string(&[backdated.clone()]).unwrap()).unwrap();
        assert_eq!(parsed, vec![backdated]);
    }

    #[test]
    fn checked_import_rejects_a_changed_journal() {
        let mut storage = Storage::open_in_memory().unwrap();
        let existing = EntryDraft {
            title: "Existing".into(),
            body: String::new(),
            tags_text: String::new(),
        }
        .validate()
        .unwrap();
        storage.create_entry(&existing).unwrap();
        let snapshot = revisions(&storage);

        let another = EntryDraft {
            title: "Saved elsewhere".into(),
            body: String::new(),
            tags_text: String::new(),
        }
        .validate()
        .unwrap();
        storage.create_entry(&another).unwrap();

        let imported = vec![entry(30, "Restored", &[])];
        let error = apply_import_checked(&mut storage, &imported, &snapshot).unwrap_err();
        assert!(matches!(
            error,
            BackupError::Storage(StorageError::JournalChanged)
        ));
        assert_eq!(storage.list_entries().unwrap().len(), 2);

        let snapshot = revisions(&storage);
        apply_import_checked(&mut storage, &imported, &snapshot).unwrap();
        assert_eq!(storage.list_entries().unwrap()[0].title, "Restored");
    }

    #[test]
    fn imported_tags_are_normalized() {
        let mut raw = entry(5, "Tagged", &[]);
        raw.tags = vec![
            "#Work".to_owned(),
            "work".to_owned(),
            " Deep   Work ".to_owned(),
        ];
        let parsed = parse(&export_to_string(&[raw]).unwrap()).unwrap();
        assert_eq!(parsed[0].tags, vec!["work", "deep work"]);
    }

    #[test]
    fn read_and_write_backup_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("backup.json");
        let entries = vec![entry(11, "On disk", &["backup"])];

        write_backup(&path, &entries).unwrap();
        assert_eq!(read_backup(&path).unwrap(), entries);

        let error = read_backup(&dir.path().join("missing.json")).unwrap_err();
        assert!(matches!(error, BackupError::ReadFile { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn backup_files_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let path = dir.path().join("backup.json");
        write_backup(&path, &[entry(12, "Private", &["backup"])]).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn failed_export_does_not_replace_a_directory() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("backup.json");
        std::fs::create_dir(&path).unwrap();
        let sentinel = path.join("keep-me.txt");
        std::fs::write(&sentinel, "original").unwrap();

        let result = write_backup(&path, &[entry(14, "Replacement", &[])]);

        assert!(result.is_err());
        assert!(path.is_dir());
        assert_eq!(std::fs::read_to_string(sentinel).unwrap(), "original");
    }

    #[test]
    fn apply_import_replaces_existing_data() {
        let mut storage = Storage::open_in_memory().unwrap();
        let existing = EntryDraft {
            title: "Existing".into(),
            body: "".into(),
            tags_text: "old".into(),
        }
        .validate()
        .unwrap();
        storage.create_entry(&existing).unwrap();

        let imported = vec![entry(21, "Restored", &["backup"])];
        apply_import(&mut storage, &imported).unwrap();

        let loaded = storage.list_entries().unwrap();
        assert_eq!(loaded.len(), imported.len());
        assert_eq!(loaded[0].id, imported[0].id);
        assert!(loaded[0].revision > 0);
        assert_eq!(loaded[0].title, imported[0].title);
        assert_eq!(loaded[0].tags, imported[0].tags);
        assert_eq!(storage.all_tags().unwrap(), vec!["backup".to_owned()]);
    }
}
