//! Versioned JSON backups, validation, and transactional restore.
//!
//! A backup is a complete snapshot of the journal. Import replaces the stored
//! entries only after the whole file has been validated, and the replacement
//! itself runs in one SQLite transaction.

use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{normalize_tags, EntryDraft, JournalEntry};
use crate::storage::{Storage, StorageError};

pub const BACKUP_FORMAT: &str = "e-journal-backup";
pub const BACKUP_VERSION: u32 = 1;

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

        if self.updated_at < self.created_at {
            return Err(BackupError::InvalidEntry {
                index,
                message: "updated_at is earlier than created_at".into(),
            });
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
    let json = std::fs::read_to_string(path).map_err(|source| BackupError::ReadFile {
        path: path.display().to_string(),
        source,
    })?;
    parse(&json)
}

/// Write a backup file.
pub fn write_backup(path: &Path, entries: &[JournalEntry]) -> Result<(), BackupError> {
    let json = export_to_string(entries)?;
    std::fs::write(path, json).map_err(|source| BackupError::WriteFile {
        path: path.display().to_string(),
        source,
    })
}

/// Replace the journal contents with an already-validated backup.
pub fn apply_import(storage: &mut Storage, entries: &[JournalEntry]) -> Result<(), BackupError> {
    storage.replace_all_entries(entries)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    use super::*;
    use crate::model::EntryDraft;

    fn entry(id: i64, title: &str, tags: &[&str]) -> JournalEntry {
        let created = Utc.with_ymd_and_hms(2026, 11, 18, 9, 30, 0).unwrap();
        JournalEntry {
            id,
            title: title.to_owned(),
            body: "A quiet morning.".to_owned(),
            created_at: created,
            updated_at: created,
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        }
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

        let mut invalid_time = entry(4, "Time", &[]);
        invalid_time.updated_at = invalid_time.created_at - chrono::Duration::seconds(1);
        assert!(matches!(
            parse(&export_to_string(&[invalid_time]).unwrap()),
            Err(BackupError::InvalidEntry { index: 0, .. })
        ));
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

        assert_eq!(storage.list_entries().unwrap(), imported);
        assert_eq!(storage.all_tags().unwrap(), vec!["backup".to_owned()]);
    }
}
