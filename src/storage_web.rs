//! Browser storage for the WebAssembly build.
//!
//! This keeps the same API as the native SQLite store, but a whole journal is
//! serialized as one versioned JSON document in `localStorage`. Revisions are
//! still monotonic so the editor can detect concurrent saves from another tab.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{EntryId, JournalEntry, SearchFilter, ValidatedEntry, MAX_ENTRY_ID};

const STORAGE_KEY: &str = "e-journal.web.v1";

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("Browser storage is unavailable: {0}")]
    Browser(String),

    #[error("Could not decode the journal in browser storage: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Entry {0} no longer exists.")]
    NotFound(EntryId),

    #[error("Entry {0} was changed by another tab. Reload it before saving again.")]
    Conflict(EntryId),

    #[error(
        "The journal changed since this action was confirmed. Review the current entries \
         and try again."
    )]
    JournalChanged,

    #[error("A journal replacement cannot contain duplicate entry id {0}.")]
    DuplicateEntryId(EntryId),

    #[error("A journal replacement contains a tag with a null character.")]
    NullInTag,

    #[error("Entry id {0} is outside the range this journal supports.")]
    EntryIdOutOfRange(EntryId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebData {
    next_id: EntryId,
    next_revision: i64,
    entries: Vec<JournalEntry>,
    settings: BTreeMap<String, String>,
}

impl Default for WebData {
    fn default() -> Self {
        Self {
            next_id: 1,
            next_revision: 0,
            entries: Vec::new(),
            settings: BTreeMap::new(),
        }
    }
}

/// Browser-backed journal storage.
#[derive(Debug)]
pub struct Storage {
    data: WebData,
}

impl Storage {
    /// Open the journal stored in this browser.
    pub fn open(_path: &Path) -> Result<Self, StorageError> {
        Ok(Self { data: load_data()? })
    }

    pub fn list_entries(&self) -> Result<Vec<JournalEntry>, StorageError> {
        let mut entries = self.data.entries.clone();
        entries.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| right.id.cmp(&left.id))
        });
        Ok(entries)
    }

    pub fn get_entry(&self, id: EntryId) -> Result<Option<JournalEntry>, StorageError> {
        Ok(self
            .data
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned())
    }

    pub fn search(&self, filter: &SearchFilter) -> Result<Vec<JournalEntry>, StorageError> {
        let prepared = filter.prepare();
        Ok(self
            .list_entries()?
            .into_iter()
            .filter(|entry| prepared.matches(entry))
            .collect())
    }

    pub fn create_entry(&mut self, input: &ValidatedEntry) -> Result<JournalEntry, StorageError> {
        check_tags(&input.tags)?;
        let now = Utc::now();
        let max_id = self
            .data
            .entries
            .iter()
            .map(|entry| entry.id)
            .max()
            .unwrap_or(0);
        let id = self.data.next_id.max(max_id.saturating_add(1));
        if id <= 0 || id > MAX_ENTRY_ID {
            return Err(StorageError::EntryIdOutOfRange(id));
        }
        let revision = self.data.next_revision.saturating_add(1);
        let entry = JournalEntry {
            id,
            revision,
            title: input.title.clone(),
            body: input.body.clone(),
            created_at: now,
            updated_at: now,
            tags: input.tags.clone(),
        };

        let mut data = self.data.clone();
        data.next_id = id.saturating_add(1);
        data.next_revision = revision;
        data.entries.push(entry.clone());
        commit(&mut self.data, data)?;
        Ok(entry)
    }

    pub fn update_entry(
        &mut self,
        id: EntryId,
        expected_revision: i64,
        input: &ValidatedEntry,
    ) -> Result<JournalEntry, StorageError> {
        check_tags(&input.tags)?;
        let mut data = self.data.clone();
        let entry = data
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or(StorageError::NotFound(id))?;
        if entry.revision != expected_revision {
            return Err(StorageError::Conflict(id));
        }

        entry.revision = data.next_revision.saturating_add(1);
        entry.title = input.title.clone();
        entry.body = input.body.clone();
        entry.updated_at = Utc::now();
        entry.tags = input.tags.clone();
        data.next_revision = entry.revision;
        let saved = entry.clone();
        commit(&mut self.data, data)?;
        Ok(saved)
    }

    pub fn delete_entry(
        &mut self,
        id: EntryId,
        expected_revision: i64,
    ) -> Result<bool, StorageError> {
        let Some(index) = self.data.entries.iter().position(|entry| entry.id == id) else {
            return Ok(false);
        };
        if self.data.entries[index].revision != expected_revision {
            return Err(StorageError::Conflict(id));
        }

        let mut data = self.data.clone();
        data.entries.remove(index);
        commit(&mut self.data, data)?;
        Ok(true)
    }

    pub fn revision_snapshot(&self) -> Result<Vec<(EntryId, i64)>, StorageError> {
        let mut revisions = self
            .data
            .entries
            .iter()
            .map(|entry| (entry.id, entry.revision))
            .collect::<Vec<_>>();
        revisions.sort_unstable_by_key(|(id, _)| *id);
        Ok(revisions)
    }

    pub fn replace_all_entries(&mut self, entries: &[JournalEntry]) -> Result<(), StorageError> {
        self.replace_entries(entries)
    }

    pub fn replace_all_entries_checked(
        &mut self,
        entries: &[JournalEntry],
        expected: &[(EntryId, i64)],
    ) -> Result<(), StorageError> {
        if self.revision_snapshot()? != expected {
            return Err(StorageError::JournalChanged);
        }
        self.replace_entries(entries)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        Ok(self.data.settings.get(key).cloned())
    }

    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<(), StorageError> {
        let mut data = self.data.clone();
        data.settings.insert(key.to_owned(), value.to_owned());
        commit(&mut self.data, data)
    }

    fn replace_entries(&mut self, entries: &[JournalEntry]) -> Result<(), StorageError> {
        let mut ids = std::collections::HashSet::with_capacity(entries.len());
        let mut next_id = 1;
        let mut replacement = Vec::with_capacity(entries.len());

        for (index, entry) in entries.iter().enumerate() {
            if entry.id <= 0 || entry.id > MAX_ENTRY_ID {
                return Err(StorageError::EntryIdOutOfRange(entry.id));
            }
            if !ids.insert(entry.id) {
                return Err(StorageError::DuplicateEntryId(entry.id));
            }
            check_tags(&entry.tags)?;
            next_id = next_id.max(entry.id.saturating_add(1));
            let mut entry = entry.clone();
            entry.revision = i64::try_from(index + 1).unwrap_or(i64::MAX);
            replacement.push(entry);
        }

        let revision = i64::try_from(replacement.len()).unwrap_or(i64::MAX);
        let mut data = self.data.clone();
        data.entries = replacement;
        data.next_id = next_id;
        data.next_revision = revision;
        commit(&mut self.data, data)
    }
}

fn check_tags(tags: &[String]) -> Result<(), StorageError> {
    if tags.iter().any(|tag| tag.contains('\0')) {
        return Err(StorageError::NullInTag);
    }
    Ok(())
}

fn commit(current: &mut WebData, data: WebData) -> Result<(), StorageError> {
    save_data(&data)?;
    *current = data;
    Ok(())
}

fn local_storage() -> Result<web_sys::Storage, StorageError> {
    web_sys::window()
        .ok_or_else(|| StorageError::Browser("window is unavailable".to_owned()))?
        .local_storage()
        .map_err(|error| StorageError::Browser(format!("{error:?}")))?
        .ok_or_else(|| StorageError::Browser("localStorage is disabled".to_owned()))
}

fn load_data() -> Result<WebData, StorageError> {
    let storage = local_storage()?;
    match storage
        .get_item(STORAGE_KEY)
        .map_err(|error| StorageError::Browser(format!("{error:?}")))?
    {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => Ok(WebData::default()),
    }
}

fn save_data(data: &WebData) -> Result<(), StorageError> {
    let json = serde_json::to_string(data)?;
    local_storage()?
        .set_item(STORAGE_KEY, &json)
        .map_err(|error| StorageError::Browser(format!("{error:?}")))
}
