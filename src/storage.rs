//! SQLite persistence: schema versioning and entry CRUD.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use thiserror::Error;

use crate::model::{
    format_timestamp, parse_timestamp, EntryId, JournalEntry, SearchFilter, ValidatedEntry,
    MAX_ENTRY_ID,
};

/// Schema version written to `PRAGMA user_version`.
pub const SCHEMA_VERSION: u32 = 2;

/// Leave room for new entries after restoring a journal with very high ids.
const RESTORE_ID_HEADROOM: EntryId = 1_000_000;
const MAX_SAFE_RESTORE_ID: EntryId = MAX_ENTRY_ID - RESTORE_ID_HEADROOM;

const SCHEMA_V1: &str = r#"
CREATE TABLE entries (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    title       TEXT NOT NULL,
    body        TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE tags (
    id    INTEGER PRIMARY KEY AUTOINCREMENT,
    name  TEXT NOT NULL COLLATE NOCASE UNIQUE
);

CREATE TABLE entry_tags (
    entry_id  INTEGER NOT NULL REFERENCES entries(id) ON DELETE CASCADE,
    tag_id    INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (entry_id, tag_id)
);

CREATE INDEX entries_updated_at_idx ON entries(updated_at DESC);
CREATE INDEX entry_tags_tag_idx ON entry_tags(tag_id);

CREATE TABLE settings (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);
"#;

const SCHEMA_V2: &str = r#"
ALTER TABLE entries ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;

CREATE TABLE counters (
    name   TEXT PRIMARY KEY,
    value  INTEGER NOT NULL
);

INSERT INTO counters (name, value) VALUES ('revision', 0);

UPDATE entries
SET revision = (
    SELECT COUNT(*) FROM entries AS older WHERE older.id <= entries.id
);

UPDATE counters
SET value = (SELECT COUNT(*) FROM entries)
WHERE name = 'revision';

CREATE UNIQUE INDEX entries_revision_idx ON entries(revision);
"#;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Could not create the journal data folder at {path}: {source}")]
    CreateDataDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Could not open the journal database at {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error("The journal database reported an error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error(
        "The journal database was created by a newer version of the app \
         (schema {found}; this app supports {supported})."
    )]
    UnsupportedSchema { found: u32, supported: u32 },

    #[error("Entry {0} no longer exists.")]
    NotFound(EntryId),

    #[error("The journal database contains an unreadable timestamp for entry {id}: {value}")]
    CorruptTimestamp { id: EntryId, value: String },

    #[error("A journal replacement cannot contain duplicate entry id {0}.")]
    DuplicateEntryId(EntryId),

    #[error("Entry id {0} is outside the range this journal supports.")]
    EntryIdOutOfRange(EntryId),

    #[error("Entry {0} was changed by another instance. Reload it before saving again.")]
    Conflict(EntryId),

    #[error("Could not restrict access to {path}: {source}")]
    RestrictAccess {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Owns the SQLite connection. All SQL in the application lives in this module.
#[derive(Debug)]
pub struct Storage {
    conn: Connection,
}

impl Storage {
    /// Open (or create) the journal database at `path`.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|source| StorageError::CreateDataDir {
                path: parent.to_path_buf(),
                source,
            })?;
            restrict_directory(parent)?;
        }

        let conn = Connection::open(path).map_err(|source| StorageError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        restrict_file(path)?;
        Self::from_connection(conn)
    }

    /// Open a fresh in-memory database. Used by tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, StorageError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, StorageError> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", true)?;

        let mut storage = Self { conn };
        storage.migrate()?;
        Ok(storage)
    }

    fn migrate(&mut self) -> Result<(), StorageError> {
        // The write lock is taken before reading the schema version so two
        // first launches cannot both create the same tables.
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: u32 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;

        if version > SCHEMA_VERSION {
            return Err(StorageError::UnsupportedSchema {
                found: version,
                supported: SCHEMA_VERSION,
            });
        }

        if version < 1 {
            tx.execute_batch(SCHEMA_V1)?;
            tx.pragma_update(None, "user_version", 1)?;
        }

        if version < 2 {
            tx.execute_batch(SCHEMA_V2)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }

        tx.commit()?;
        Ok(())
    }

    /// The schema version currently stored in the database.
    #[cfg(test)]
    pub fn schema_version(&self) -> Result<u32, StorageError> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?)
    }

    /// All entries, newest update first.
    ///
    /// Entries and their tags are read from one SQLite snapshot so the editor
    /// cannot mix an entry body with tags from another revision.
    pub fn list_entries(&self) -> Result<Vec<JournalEntry>, StorageError> {
        let tx = self.conn.unchecked_transaction()?;
        let raw_entries = {
            let mut stmt = tx.prepare(
                "SELECT id, revision, title, body, created_at, updated_at
                 FROM entries
                 ORDER BY updated_at DESC, id DESC",
            )?;
            let rows = stmt.query_map([], raw_entry_from_row)?;
            let mut raw_entries = Vec::new();
            for row in rows {
                raw_entries.push(row?);
            }
            raw_entries
        };

        let mut entries = Vec::with_capacity(raw_entries.len());
        for raw in raw_entries {
            entries.push(entry_from_raw(&tx, raw)?);
        }
        tx.commit()?;
        Ok(entries)
    }

    /// Find one entry by id.
    pub fn get_entry(&self, id: EntryId) -> Result<Option<JournalEntry>, StorageError> {
        let tx = self.conn.unchecked_transaction()?;
        let entry = entry_by_id(&tx, id)?;
        tx.commit()?;
        Ok(entry)
    }

    /// Entries matching a search query and optional tag filter.
    pub fn search(&self, filter: &SearchFilter) -> Result<Vec<JournalEntry>, StorageError> {
        Ok(self
            .list_entries()?
            .into_iter()
            .filter(|entry| filter.matches(entry))
            .collect())
    }

    /// Insert a validated entry and its tags in one transaction.
    pub fn create_entry(&mut self, input: &ValidatedEntry) -> Result<JournalEntry, StorageError> {
        let now = format_timestamp(chrono::Utc::now());
        let tx = self.conn.transaction()?;
        let revision = next_revision(&tx)?;
        tx.execute(
            "INSERT INTO entries (title, body, created_at, updated_at, revision)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![input.title, input.body, now, now, revision],
        )?;
        let id = tx.last_insert_rowid();
        if id > MAX_ENTRY_ID {
            return Err(StorageError::EntryIdOutOfRange(id));
        }
        set_tags(&tx, id, &input.tags)?;

        let saved = entry_by_id(&tx, id)?.ok_or(StorageError::NotFound(id))?;
        tx.commit()?;
        Ok(saved)
    }

    /// Update an entry only if its stored revision still matches the editor.
    ///
    /// `expected_revision` prevents a stale draft from overwriting a newer
    /// save made by another app instance.
    pub fn update_entry(
        &mut self,
        id: EntryId,
        expected_revision: i64,
        input: &ValidatedEntry,
    ) -> Result<JournalEntry, StorageError> {
        let tx = self.conn.transaction()?;
        let current = tx
            .query_row(
                "SELECT created_at, updated_at, revision FROM entries WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((created_at_raw, current_updated_raw, current_revision)) = current else {
            return Err(StorageError::NotFound(id));
        };
        if current_revision != expected_revision {
            return Err(StorageError::Conflict(id));
        }

        let created_at = match parse_timestamp(&created_at_raw) {
            Ok(timestamp) => timestamp,
            Err(_) => {
                return Err(StorageError::CorruptTimestamp {
                    id,
                    value: created_at_raw,
                });
            }
        };
        let current_updated = match parse_timestamp(&current_updated_raw) {
            Ok(timestamp) => timestamp,
            Err(_) => {
                return Err(StorageError::CorruptTimestamp {
                    id,
                    value: current_updated_raw,
                });
            }
        };
        let now = format_timestamp(Utc::now().max(created_at).max(current_updated));
        let revision = next_revision(&tx)?;

        let changed = tx.execute(
            "UPDATE entries SET title = ?1, body = ?2, updated_at = ?3, revision = ?4
             WHERE id = ?5 AND revision = ?6",
            params![
                input.title,
                input.body,
                now,
                revision,
                id,
                expected_revision
            ],
        )?;
        if changed == 0 {
            return Err(StorageError::Conflict(id));
        }
        tx.execute("DELETE FROM entry_tags WHERE entry_id = ?1", [id])?;
        set_tags(&tx, id, &input.tags)?;

        let saved = entry_by_id(&tx, id)?.ok_or(StorageError::NotFound(id))?;
        tx.commit()?;
        Ok(saved)
    }

    /// Delete an entry only if it still has the revision the user confirmed.
    /// Returns `false` if it was already gone.
    pub fn delete_entry(
        &mut self,
        id: EntryId,
        expected_revision: i64,
    ) -> Result<bool, StorageError> {
        let tx = self.conn.transaction()?;
        let revision = tx
            .query_row("SELECT revision FROM entries WHERE id = ?1", [id], |row| {
                row.get::<_, i64>(0)
            })
            .optional()?;
        let Some(revision) = revision else {
            tx.commit()?;
            return Ok(false);
        };
        if revision != expected_revision {
            return Err(StorageError::Conflict(id));
        }

        let deleted = tx.execute(
            "DELETE FROM entries WHERE id = ?1 AND revision = ?2",
            params![id, expected_revision],
        )? > 0;
        if deleted {
            tx.execute(
                "DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM entry_tags)",
                [],
            )?;
        }
        tx.commit()?;
        Ok(deleted)
    }

    /// All tags that are currently attached to at least one entry.
    pub fn all_tags(&self) -> Result<Vec<String>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT t.name
             FROM tags t
             JOIN entry_tags et ON et.tag_id = t.id
             ORDER BY t.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;

        let mut tags = Vec::new();
        for tag in rows {
            tags.push(tag?);
        }
        Ok(tags)
    }

    /// Replace every stored entry (used by backup restore) in a single
    /// transaction. IDs and timestamps are preserved when they are safely
    /// below the storage limit; otherwise IDs are remapped to leave room for
    /// new entries. Every restored entry gets a fresh revision.
    pub fn replace_all_entries(&mut self, entries: &[JournalEntry]) -> Result<(), StorageError> {
        let mut ids = HashSet::with_capacity(entries.len());
        for entry in entries {
            if entry.id <= 0 {
                return Err(StorageError::EntryIdOutOfRange(entry.id));
            }
            if !ids.insert(entry.id) {
                return Err(StorageError::DuplicateEntryId(entry.id));
            }
        }

        let remap_ids = entries.iter().any(|entry| entry.id > MAX_SAFE_RESTORE_ID);
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM entry_tags", [])?;
        tx.execute("DELETE FROM tags", [])?;
        tx.execute("DELETE FROM entries", [])?;
        tx.execute("DELETE FROM sqlite_sequence WHERE name = 'entries'", [])?;

        for (index, entry) in entries.iter().enumerate() {
            let id = if remap_ids {
                i64::try_from(index + 1).map_err(|_| StorageError::EntryIdOutOfRange(i64::MAX))?
            } else {
                entry.id
            };
            let revision = next_revision(&tx)?;
            tx.execute(
                "INSERT INTO entries (id, title, body, created_at, updated_at, revision)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    entry.title,
                    entry.body,
                    format_timestamp(entry.created_at),
                    format_timestamp(entry.updated_at),
                    revision
                ],
            )?;
            set_tags(&tx, id, &entry.tags)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Read a persisted setting (currently the theme).
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    /// Persist a setting.
    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<(), StorageError> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

struct RawEntry {
    id: EntryId,
    revision: i64,
    title: String,
    body: String,
    created_at: String,
    updated_at: String,
}

fn raw_entry_from_row(row: &Row<'_>) -> rusqlite::Result<RawEntry> {
    Ok(RawEntry {
        id: row.get(0)?,
        revision: row.get(1)?,
        title: row.get(2)?,
        body: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn entry_by_id(conn: &Connection, id: EntryId) -> Result<Option<JournalEntry>, StorageError> {
    let raw = conn
        .query_row(
            "SELECT id, revision, title, body, created_at, updated_at
             FROM entries WHERE id = ?1",
            [id],
            raw_entry_from_row,
        )
        .optional()?;
    raw.map(|raw| entry_from_raw(conn, raw)).transpose()
}

fn entry_from_raw(conn: &Connection, raw: RawEntry) -> Result<JournalEntry, StorageError> {
    let RawEntry {
        id,
        revision,
        title,
        body,
        created_at,
        updated_at,
    } = raw;
    let created_at = match parse_timestamp(&created_at) {
        Ok(timestamp) => timestamp,
        Err(_) => {
            return Err(StorageError::CorruptTimestamp {
                id,
                value: created_at,
            });
        }
    };
    let updated_at = match parse_timestamp(&updated_at) {
        Ok(timestamp) => timestamp,
        Err(_) => {
            return Err(StorageError::CorruptTimestamp {
                id,
                value: updated_at,
            });
        }
    };
    let tags = tags_for_entry(conn, id)?;

    Ok(JournalEntry {
        id,
        revision,
        title,
        body,
        created_at,
        updated_at,
        tags,
    })
}

fn tags_for_entry(conn: &Connection, id: EntryId) -> Result<Vec<String>, StorageError> {
    let mut stmt = conn.prepare(
        "SELECT t.name
         FROM tags t
         JOIN entry_tags et ON et.tag_id = t.id
         WHERE et.entry_id = ?1
         ORDER BY t.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([id], |row| row.get::<_, String>(0))?;

    let mut tags = Vec::new();
    for tag in rows {
        tags.push(tag?);
    }
    Ok(tags)
}

fn next_revision(tx: &Transaction<'_>) -> Result<i64, StorageError> {
    Ok(tx.query_row(
        "UPDATE counters SET value = value + 1 WHERE name = 'revision' RETURNING value",
        [],
        |row| row.get(0),
    )?)
}

#[cfg(unix)]
fn restrict_directory(path: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(|source| {
        StorageError::RestrictAccess {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(not(unix))]
fn restrict_directory(_path: &Path) -> Result<(), StorageError> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file(path: &Path) -> Result<(), StorageError> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|source| {
        StorageError::RestrictAccess {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(not(unix))]
fn restrict_file(_path: &Path) -> Result<(), StorageError> {
    Ok(())
}

fn set_tags(tx: &Transaction<'_>, entry_id: EntryId, tags: &[String]) -> rusqlite::Result<()> {
    for tag in tags {
        tx.execute(
            "INSERT INTO tags (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            [tag],
        )?;
        let tag_id: i64 = tx.query_row("SELECT id FROM tags WHERE name = ?1", [tag], |row| {
            row.get(0)
        })?;
        tx.execute(
            "INSERT OR IGNORE INTO entry_tags (entry_id, tag_id) VALUES (?1, ?2)",
            params![entry_id, tag_id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;

    use chrono::Utc;
    use tempfile::tempdir;

    use super::*;
    use crate::model::EntryDraft;

    fn validated(title: &str, body: &str, tags: &str) -> ValidatedEntry {
        EntryDraft {
            title: title.to_owned(),
            body: body.to_owned(),
            tags_text: tags.to_owned(),
        }
        .validate()
        .unwrap()
    }

    fn seed(storage: &mut Storage, title: &str, tags: &str) -> JournalEntry {
        storage
            .create_entry(&validated(title, "Body text", tags))
            .unwrap()
    }

    fn assert_same_content(actual: &[JournalEntry], expected: &[JournalEntry]) {
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.iter().zip(expected) {
            assert_eq!(actual.id, expected.id);
            assert_eq!(actual.title, expected.title);
            assert_eq!(actual.body, expected.body);
            assert_eq!(actual.created_at, expected.created_at);
            assert_eq!(actual.updated_at, expected.updated_at);
            assert_eq!(actual.tags, expected.tags);
        }
    }

    #[test]
    fn schema_is_created_and_reopened() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("journal.sqlite3");

        {
            let storage = Storage::open(&path).unwrap();
            assert_eq!(storage.schema_version().unwrap(), SCHEMA_VERSION);
        }

        let storage = Storage::open(&path).unwrap();
        assert_eq!(storage.schema_version().unwrap(), SCHEMA_VERSION);
        assert!(storage.list_entries().unwrap().is_empty());
    }

    #[test]
    fn crud_round_trip() {
        let mut storage = Storage::open_in_memory().unwrap();
        let created = storage
            .create_entry(&validated(
                "First entry",
                "Hello there",
                "personal, Journal",
            ))
            .unwrap();

        assert_eq!(created.title, "First entry");
        assert_eq!(created.tags, vec!["journal", "personal"]);
        assert_eq!(storage.list_entries().unwrap().len(), 1);
        assert_eq!(storage.get_entry(created.id).unwrap().unwrap(), created);

        thread::sleep(Duration::from_millis(5));
        let updated = storage
            .update_entry(
                created.id,
                created.revision,
                &validated("Renamed entry", "Updated body", "journal, work"),
            )
            .unwrap();

        assert_eq!(updated.id, created.id);
        assert_eq!(updated.title, "Renamed entry");
        assert_eq!(updated.created_at, created.created_at);
        assert!(updated.updated_at > created.updated_at);
        assert_eq!(updated.tags, vec!["journal", "work"]);

        assert!(storage.delete_entry(created.id, updated.revision).unwrap());
        assert!(storage.get_entry(created.id).unwrap().is_none());
        assert!(storage.all_tags().unwrap().is_empty());
        assert!(!storage.delete_entry(created.id, updated.revision).unwrap());
    }

    #[test]
    fn search_matches_terms_and_tags() {
        let mut storage = Storage::open_in_memory().unwrap();
        seed(&mut storage, "Morning walk", "nature");
        seed(&mut storage, "Project notes", "work, ideas");
        seed(&mut storage, "Grocery list", "personal");

        let by_title = storage
            .search(&SearchFilter {
                query: "MORNING".into(),
                tag: None,
            })
            .unwrap();
        assert_eq!(by_title.len(), 1);
        assert_eq!(by_title[0].title, "Morning walk");

        let by_body = storage
            .search(&SearchFilter {
                query: "body text".into(),
                tag: None,
            })
            .unwrap();
        assert_eq!(by_body.len(), 3);

        let by_tag = storage
            .search(&SearchFilter {
                query: String::new(),
                tag: Some("WORK".into()),
            })
            .unwrap();
        assert_eq!(by_tag.len(), 1);
        assert_eq!(by_tag[0].title, "Project notes");

        let combined = storage
            .search(&SearchFilter {
                query: "notes".into(),
                tag: Some("work".into()),
            })
            .unwrap();
        assert_eq!(combined.len(), 1);

        assert!(storage
            .search(&SearchFilter {
                query: "notes".into(),
                tag: Some("personal".into()),
            })
            .unwrap()
            .is_empty());
    }

    #[test]
    fn tags_are_unique_and_cleanup_after_delete() {
        let mut storage = Storage::open_in_memory().unwrap();
        let first = seed(&mut storage, "One", "Work, work, Deep Work");
        let second = seed(&mut storage, "Two", "work");
        assert_eq!(
            storage.all_tags().unwrap(),
            vec!["deep work".to_owned(), "work".to_owned()]
        );

        assert!(storage.delete_entry(first.id, first.revision).unwrap());
        assert_eq!(storage.all_tags().unwrap(), vec!["work".to_owned()]);
        assert!(storage.delete_entry(second.id, second.revision).unwrap());
        assert!(storage.all_tags().unwrap().is_empty());
    }

    #[test]
    fn entries_are_ordered_by_latest_update() {
        let mut storage = Storage::open_in_memory().unwrap();
        let first = seed(&mut storage, "First", "");
        thread::sleep(Duration::from_millis(5));
        let second = seed(&mut storage, "Second", "");

        let entries = storage.list_entries().unwrap();
        assert_eq!(entries[0].id, second.id);
        assert_eq!(entries[1].id, first.id);

        thread::sleep(Duration::from_millis(5));
        storage
            .update_entry(
                first.id,
                first.revision,
                &validated("First updated", "Body text", ""),
            )
            .unwrap();
        let entries = storage.list_entries().unwrap();
        assert_eq!(entries[0].id, first.id);
    }

    #[test]
    fn revisions_increase_on_every_save() {
        let mut storage = Storage::open_in_memory().unwrap();
        let created = seed(&mut storage, "Original", "");
        let first = storage
            .update_entry(
                created.id,
                created.revision,
                &validated("First update", "Body text", ""),
            )
            .unwrap();
        let second = storage
            .update_entry(
                created.id,
                first.revision,
                &validated("Second update", "Body text", ""),
            )
            .unwrap();

        assert!(first.revision > created.revision);
        assert!(second.revision > first.revision);
    }

    #[test]
    fn delete_requires_the_confirmed_revision() {
        let mut storage = Storage::open_in_memory().unwrap();
        let created = seed(&mut storage, "Original", "");
        let updated = storage
            .update_entry(
                created.id,
                created.revision,
                &validated("Updated", "Body text", ""),
            )
            .unwrap();

        let error = storage
            .delete_entry(created.id, created.revision)
            .unwrap_err();
        assert!(matches!(error, StorageError::Conflict(id) if id == created.id));
        assert_eq!(storage.get_entry(created.id).unwrap().unwrap(), updated);

        assert!(storage.delete_entry(created.id, updated.revision).unwrap());
    }

    #[test]
    fn restore_assigns_fresh_revisions() {
        let mut storage = Storage::open_in_memory().unwrap();
        let created = seed(&mut storage, "Original", "old");
        let now = Utc::now();
        let replacement = vec![JournalEntry {
            id: created.id,
            revision: 0,
            title: "Restored".into(),
            body: "From backup".into(),
            created_at: now,
            updated_at: now,
            tags: vec!["backup".into()],
        }];

        storage.replace_all_entries(&replacement).unwrap();
        let loaded = storage.list_entries().unwrap();
        assert!(loaded[0].revision > created.revision);

        let error = storage
            .update_entry(
                created.id,
                created.revision,
                &validated("Stale", "Stale body", ""),
            )
            .unwrap_err();
        assert!(matches!(error, StorageError::Conflict(id) if id == created.id));
        assert_eq!(storage.list_entries().unwrap()[0].title, "Restored");
    }

    #[test]
    fn very_large_restored_ids_are_remapped_with_headroom() {
        let mut storage = Storage::open_in_memory().unwrap();
        let now = Utc::now();
        let high = vec![JournalEntry {
            id: MAX_ENTRY_ID,
            revision: 0,
            title: "High id".into(),
            body: "From a crafted backup".into(),
            created_at: now,
            updated_at: now,
            tags: vec!["restore".into()],
        }];

        storage.replace_all_entries(&high).unwrap();
        let loaded = storage.list_entries().unwrap();
        assert_eq!(loaded[0].id, 1);

        let new = seed(&mut storage, "After restore", "");
        assert_eq!(new.id, 2);
        assert!(new.id <= MAX_ENTRY_ID);
        assert!(crate::backup::parse(
            &crate::backup::export_to_string(&storage.list_entries().unwrap()).unwrap()
        )
        .is_ok());
    }

    #[test]
    fn restore_resets_the_entry_id_sequence() {
        let mut storage = Storage::open_in_memory().unwrap();
        let now = Utc::now();
        let high = vec![JournalEntry {
            id: 500,
            revision: 0,
            title: "High sequence".into(),
            body: String::new(),
            created_at: now,
            updated_at: now,
            tags: Vec::new(),
        }];
        storage.replace_all_entries(&high).unwrap();

        let low = vec![JournalEntry {
            id: 42,
            revision: 0,
            ..high[0].clone()
        }];
        storage.replace_all_entries(&low).unwrap();

        let new = seed(&mut storage, "After restore", "");
        assert_eq!(new.id, 43);
    }

    #[test]
    fn stale_updates_are_rejected() {
        let mut storage = Storage::open_in_memory().unwrap();
        let created = seed(&mut storage, "Original", "work");
        thread::sleep(Duration::from_millis(5));

        let updated = storage
            .update_entry(
                created.id,
                created.revision,
                &validated("Updated", "New body", "work"),
            )
            .unwrap();

        let error = storage
            .update_entry(
                created.id,
                created.revision,
                &validated("Stale draft", "Old body", "work"),
            )
            .unwrap_err();
        assert!(matches!(error, StorageError::Conflict(id) if id == created.id));
        assert_eq!(
            storage.get_entry(created.id).unwrap().unwrap(),
            updated,
            "a conflicting save must not modify the entry"
        );
    }

    #[test]
    fn sql_parameters_do_not_execute_entry_text() {
        let mut storage = Storage::open_in_memory().unwrap();
        let malicious = "Robert'); DROP TABLE entries;--";
        let entry = storage
            .create_entry(&validated(malicious, "still here", ""))
            .unwrap();
        assert_eq!(entry.title, malicious);
        assert_eq!(storage.list_entries().unwrap().len(), 1);
    }

    #[test]
    fn replace_all_is_transactional_and_preserves_ids() {
        let mut storage = Storage::open_in_memory().unwrap();
        let original = seed(&mut storage, "Original", "old");

        let now = Utc::now();
        let replacement = vec![JournalEntry {
            id: 42,
            revision: 0,
            title: "Restored".into(),
            body: "From backup".into(),
            created_at: now,
            updated_at: now,
            tags: vec!["backup".into()],
        }];

        storage.replace_all_entries(&replacement).unwrap();
        let loaded = storage.list_entries().unwrap();
        assert_same_content(&loaded, &replacement);
        assert!(loaded[0].revision > original.revision);
        assert_eq!(storage.all_tags().unwrap(), vec!["backup".to_owned()]);
        assert!(storage.get_entry(original.id).unwrap().is_none());

        let duplicate = vec![
            replacement[0].clone(),
            JournalEntry {
                id: 42,
                title: "Duplicate".into(),
                ..replacement[0].clone()
            },
        ];
        let error = storage.replace_all_entries(&duplicate).unwrap_err();
        assert!(matches!(error, StorageError::DuplicateEntryId(42)));
        assert_same_content(&storage.list_entries().unwrap(), &replacement);

        let out_of_range = vec![JournalEntry {
            id: 0,
            ..replacement[0].clone()
        }];
        let error = storage.replace_all_entries(&out_of_range).unwrap_err();
        assert!(matches!(error, StorageError::EntryIdOutOfRange(0)));
        assert_same_content(&storage.list_entries().unwrap(), &replacement);
    }

    #[test]
    fn settings_are_persisted() {
        let mut storage = Storage::open_in_memory().unwrap();
        assert_eq!(storage.get_setting("theme").unwrap(), None);
        storage.set_setting("theme", "dark").unwrap();
        assert_eq!(
            storage.get_setting("theme").unwrap(),
            Some("dark".to_owned())
        );
        storage.set_setting("theme", "warm").unwrap();
        assert_eq!(
            storage.get_setting("theme").unwrap(),
            Some("warm".to_owned())
        );
    }

    #[test]
    fn version_one_database_is_migrated() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("journal.sqlite3");
        let now = format_timestamp(Utc::now());
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA_V1).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute(
                "INSERT INTO entries (title, body, created_at, updated_at)
                 VALUES ('Legacy', 'Old body', ?1, ?1)",
                [&now],
            )
            .unwrap();
        }

        let mut storage = Storage::open(&path).unwrap();
        assert_eq!(storage.schema_version().unwrap(), SCHEMA_VERSION);
        let entries = storage.list_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].revision > 0);

        let updated = storage
            .update_entry(
                entries[0].id,
                entries[0].revision,
                &validated("Migrated", "New body", ""),
            )
            .unwrap();
        assert!(updated.revision > entries[0].revision);
    }

    #[cfg(unix)]
    #[test]
    fn journal_files_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = dir.path().join("journal.sqlite3");
        let _storage = Storage::open(&path).unwrap();

        let dir_mode = std::fs::metadata(dir.path()).unwrap().permissions().mode();
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(dir_mode & 0o777, 0o700);
        assert_eq!(file_mode & 0o777, 0o600);
    }

    #[test]
    fn newer_schema_is_rejected() {
        let mut storage = Storage::open_in_memory().unwrap();
        storage
            .conn
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        let error = storage.migrate().unwrap_err();
        assert!(matches!(
            error,
            StorageError::UnsupportedSchema {
                found,
                supported: SCHEMA_VERSION
            } if found == SCHEMA_VERSION + 1
        ));
    }

    #[test]
    fn corrupt_timestamps_are_reported() {
        let storage = Storage::open_in_memory().unwrap();
        storage
            .conn
            .execute(
                "INSERT INTO entries (title, body, created_at, updated_at)
                 VALUES ('Broken', '', 'not-a-time', 'not-a-time')",
                [],
            )
            .unwrap();
        let error = storage.list_entries().unwrap_err();
        assert!(matches!(error, StorageError::CorruptTimestamp { .. }));
    }
}
