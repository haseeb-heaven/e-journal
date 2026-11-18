//! SQLite persistence: schema versioning and entry CRUD.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use thiserror::Error;

use crate::model::{
    format_timestamp, parse_timestamp, EntryId, JournalEntry, SearchFilter, ValidatedEntry,
};

/// Schema version written to `PRAGMA user_version`.
pub const SCHEMA_VERSION: u32 = 1;

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
}

/// Owns the SQLite connection. All SQL in the application lives in this module.
#[derive(Debug)]
pub struct Storage {
    conn: Connection,
}

impl Storage {
    /// Open (or create) the journal database at `path`.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| StorageError::CreateDataDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let conn = Connection::open(path).map_err(|source| StorageError::Open {
            path: path.to_path_buf(),
            source,
        })?;
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
        let version: u32 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;

        if version > SCHEMA_VERSION {
            return Err(StorageError::UnsupportedSchema {
                found: version,
                supported: SCHEMA_VERSION,
            });
        }

        if version == 0 {
            let tx = self.conn.transaction()?;
            tx.execute_batch(SCHEMA_V1)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        }

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
    pub fn list_entries(&self) -> Result<Vec<JournalEntry>, StorageError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, body, created_at, updated_at
             FROM entries
             ORDER BY updated_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], raw_entry_from_row)?;

        let mut raw_entries = Vec::new();
        for row in rows {
            raw_entries.push(row?);
        }
        drop(stmt);

        let mut entries = Vec::with_capacity(raw_entries.len());
        for raw in raw_entries {
            entries.push(self.entry_from_raw(raw)?);
        }
        Ok(entries)
    }

    /// Find one entry by id.
    pub fn get_entry(&self, id: EntryId) -> Result<Option<JournalEntry>, StorageError> {
        let raw = self
            .conn
            .query_row(
                "SELECT id, title, body, created_at, updated_at FROM entries WHERE id = ?1",
                [id],
                raw_entry_from_row,
            )
            .optional()?;

        raw.map(|raw| self.entry_from_raw(raw)).transpose()
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
        tx.execute(
            "INSERT INTO entries (title, body, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![input.title, input.body, now, now],
        )?;
        let id = tx.last_insert_rowid();
        set_tags(&tx, id, &input.tags)?;
        tx.commit()?;

        self.get_entry(id)?.ok_or(StorageError::NotFound(id))
    }

    /// Update an entry, preserving its creation timestamp.
    pub fn update_entry(
        &mut self,
        id: EntryId,
        input: &ValidatedEntry,
    ) -> Result<JournalEntry, StorageError> {
        let now = format_timestamp(chrono::Utc::now());
        let tx = self.conn.transaction()?;
        let changed = tx.execute(
            "UPDATE entries SET title = ?1, body = ?2, updated_at = ?3 WHERE id = ?4",
            params![input.title, input.body, now, id],
        )?;
        if changed == 0 {
            return Err(StorageError::NotFound(id));
        }
        tx.execute("DELETE FROM entry_tags WHERE entry_id = ?1", [id])?;
        set_tags(&tx, id, &input.tags)?;
        tx.commit()?;

        self.get_entry(id)?.ok_or(StorageError::NotFound(id))
    }

    /// Delete an entry. Returns `false` if it was already gone.
    pub fn delete_entry(&mut self, id: EntryId) -> Result<bool, StorageError> {
        let tx = self.conn.transaction()?;
        let deleted = tx.execute("DELETE FROM entries WHERE id = ?1", [id])? > 0;
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
    /// transaction. Entry ids and timestamps are preserved.
    pub fn replace_all_entries(&mut self, entries: &[JournalEntry]) -> Result<(), StorageError> {
        let mut ids = HashSet::with_capacity(entries.len());
        for entry in entries {
            if !ids.insert(entry.id) {
                return Err(StorageError::DuplicateEntryId(entry.id));
            }
        }

        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM entry_tags", [])?;
        tx.execute("DELETE FROM tags", [])?;
        tx.execute("DELETE FROM entries", [])?;

        for entry in entries {
            tx.execute(
                "INSERT INTO entries (id, title, body, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    entry.id,
                    entry.title,
                    entry.body,
                    format_timestamp(entry.created_at),
                    format_timestamp(entry.updated_at)
                ],
            )?;
            set_tags(&tx, entry.id, &entry.tags)?;
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

    fn entry_from_raw(&self, raw: RawEntry) -> Result<JournalEntry, StorageError> {
        let RawEntry {
            id,
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
        let tags = self.tags_for_entry(id)?;

        Ok(JournalEntry {
            id,
            title,
            body,
            created_at,
            updated_at,
            tags,
        })
    }

    fn tags_for_entry(&self, id: EntryId) -> Result<Vec<String>, StorageError> {
        let mut stmt = self.conn.prepare(
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
}

struct RawEntry {
    id: EntryId,
    title: String,
    body: String,
    created_at: String,
    updated_at: String,
}

fn raw_entry_from_row(row: &Row<'_>) -> rusqlite::Result<RawEntry> {
    Ok(RawEntry {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
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
                &validated("Renamed entry", "Updated body", "journal, work"),
            )
            .unwrap();

        assert_eq!(updated.id, created.id);
        assert_eq!(updated.title, "Renamed entry");
        assert_eq!(updated.created_at, created.created_at);
        assert!(updated.updated_at > created.updated_at);
        assert_eq!(updated.tags, vec!["journal", "work"]);

        assert!(storage.delete_entry(created.id).unwrap());
        assert!(storage.get_entry(created.id).unwrap().is_none());
        assert!(storage.all_tags().unwrap().is_empty());
        assert!(!storage.delete_entry(created.id).unwrap());
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

        assert!(storage.delete_entry(first.id).unwrap());
        assert_eq!(storage.all_tags().unwrap(), vec!["work".to_owned()]);
        assert!(storage.delete_entry(second.id).unwrap());
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
            .update_entry(first.id, &validated("First updated", "Body text", ""))
            .unwrap();
        let entries = storage.list_entries().unwrap();
        assert_eq!(entries[0].id, first.id);
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
            title: "Restored".into(),
            body: "From backup".into(),
            created_at: now,
            updated_at: now,
            tags: vec!["backup".into()],
        }];

        storage.replace_all_entries(&replacement).unwrap();
        assert_eq!(storage.list_entries().unwrap(), replacement);
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
        assert_eq!(storage.list_entries().unwrap(), replacement);
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
