//! Browser storage for the WebAssembly build.
//!
//! This keeps the same API as the native SQLite store, but a whole journal is
//! serialized as one versioned JSON document in `localStorage`. Every mutation
//! re-reads that document before applying its change, which mirrors the native
//! `TransactionBehavior::Immediate` writes: a tab holding an old snapshot
//! cannot overwrite entries another tab saved in the meantime. Revisions stay
//! monotonic so the editor can still detect concurrent saves.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::model::{EntryId, JournalEntry, SearchFilter, ValidatedEntry, MAX_ENTRY_ID};

const STORAGE_KEY: &str = "e-journal.web.v1";

/// Restored ids above this limit are remapped so the journal keeps room to
/// create new entries, matching the native storage policy.
const RESTORE_ID_HEADROOM: EntryId = 1_000_000;
const MAX_SAFE_RESTORE_ID: EntryId = MAX_ENTRY_ID - RESTORE_ID_HEADROOM;

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
        self.mutate(|data| {
            check_tags(&input.tags)?;
            let now = Utc::now();
            let max_id = data.entries.iter().map(|entry| entry.id).max().unwrap_or(0);
            let id = data.next_id.max(max_id.saturating_add(1));
            if id <= 0 || id > MAX_ENTRY_ID {
                return Err(StorageError::EntryIdOutOfRange(id));
            }
            let revision = data.next_revision.saturating_add(1);
            let entry = JournalEntry {
                id,
                revision,
                title: input.title.clone(),
                body: input.body.clone(),
                created_at: now,
                updated_at: now,
                tags: input.tags.clone(),
            };

            data.next_id = id.saturating_add(1);
            data.next_revision = revision;
            data.entries.push(entry.clone());
            Ok(entry)
        })
    }

    pub fn update_entry(
        &mut self,
        id: EntryId,
        expected_revision: i64,
        input: &ValidatedEntry,
    ) -> Result<JournalEntry, StorageError> {
        self.mutate(|data| {
            check_tags(&input.tags)?;
            let index = data
                .entries
                .iter()
                .position(|entry| entry.id == id)
                .ok_or(StorageError::NotFound(id))?;
            let current = data.entries[index].clone();
            if current.revision != expected_revision {
                return Err(StorageError::Conflict(id));
            }

            let now = Utc::now().max(current.created_at).max(current.updated_at);
            let revision = data.next_revision.saturating_add(1);
            let updated = JournalEntry {
                id,
                revision,
                title: input.title.clone(),
                body: input.body.clone(),
                created_at: current.created_at,
                updated_at: now,
                tags: input.tags.clone(),
            };

            data.next_revision = revision;
            data.entries[index] = updated.clone();
            Ok(updated)
        })
    }

    pub fn delete_entry(
        &mut self,
        id: EntryId,
        expected_revision: i64,
    ) -> Result<bool, StorageError> {
        self.mutate(|data| {
            let Some(index) = data.entries.iter().position(|entry| entry.id == id) else {
                return Ok(false);
            };
            if data.entries[index].revision != expected_revision {
                return Err(StorageError::Conflict(id));
            }
            data.entries.remove(index);
            Ok(true)
        })
    }

    pub fn revision_snapshot(&self) -> Result<Vec<(EntryId, i64)>, StorageError> {
        revision_snapshot_of(&self.data)
    }

    pub fn replace_all_entries(&mut self, entries: &[JournalEntry]) -> Result<(), StorageError> {
        self.mutate(|data| replace_entries(data, entries))
    }

    pub fn replace_all_entries_checked(
        &mut self,
        entries: &[JournalEntry],
        expected: &[(EntryId, i64)],
    ) -> Result<(), StorageError> {
        self.mutate(|data| {
            if revision_snapshot_of(data)? != expected {
                return Err(StorageError::JournalChanged);
            }
            replace_entries(data, entries)
        })
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        Ok(self.data.settings.get(key).cloned())
    }

    pub fn set_setting(&mut self, key: &str, value: &str) -> Result<(), StorageError> {
        self.mutate(|data| {
            data.settings.insert(key.to_owned(), value.to_owned());
            Ok(())
        })
    }

    /// Apply a change to the freshest stored journal, then persist it.
    ///
    /// Reading first means a tab that has been idle cannot overwrite entries
    /// another tab saved while it was open.
    fn mutate<T>(
        &mut self,
        change: impl FnOnce(&mut WebData) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let mut data = load_data()?;
        let value = change(&mut data)?;
        save_data(&data)?;
        self.data = data;
        Ok(value)
    }
}

fn revision_snapshot_of(data: &WebData) -> Result<Vec<(EntryId, i64)>, StorageError> {
    let mut revisions = data
        .entries
        .iter()
        .map(|entry| (entry.id, entry.revision))
        .collect::<Vec<_>>();
    revisions.sort_unstable_by_key(|(id, _)| *id);
    Ok(revisions)
}

fn replace_entries(data: &mut WebData, entries: &[JournalEntry]) -> Result<(), StorageError> {
    let mut ids = std::collections::HashSet::with_capacity(entries.len());
    for entry in entries {
        if entry.id <= 0 || entry.id > MAX_ENTRY_ID {
            return Err(StorageError::EntryIdOutOfRange(entry.id));
        }
        if !ids.insert(entry.id) {
            return Err(StorageError::DuplicateEntryId(entry.id));
        }
        check_tags(&entry.tags)?;
    }

    let remapped_ids = remap_restored_ids(entries)?;
    let mut replacement = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let id = remapped_ids.as_ref().map_or(entry.id, |ids| ids[index]);
        let revision = data.next_revision.saturating_add(1);
        data.next_revision = revision;
        replacement.push(JournalEntry {
            id,
            revision,
            ..entry.clone()
        });
    }

    let next_id = replacement
        .iter()
        .map(|entry| entry.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);

    // An empty backup keeps the existing counters so a restore cannot make
    // future revisions repeat ones an editor may still be holding.
    data.next_id = data.next_id.max(next_id);
    data.entries = replacement;
    Ok(())
}

/// Mirror the native restore policy: if any restored id would leave too little
/// headroom for new entries, renumber every restored entry in id order.
fn remap_restored_ids(entries: &[JournalEntry]) -> Result<Option<Vec<EntryId>>, StorageError> {
    if !entries.iter().any(|entry| entry.id > MAX_SAFE_RESTORE_ID) {
        return Ok(None);
    }
    let mut order = (0..entries.len()).collect::<Vec<_>>();
    order.sort_by_key(|&index| entries[index].id);
    let mut ids = vec![0; entries.len()];
    for (position, index) in order.into_iter().enumerate() {
        ids[index] =
            i64::try_from(position + 1).map_err(|_| StorageError::EntryIdOutOfRange(i64::MAX))?;
    }
    Ok(Some(ids))
}

fn check_tags(tags: &[String]) -> Result<(), StorageError> {
    if tags.iter().any(|tag| tag.contains('\0')) {
        return Err(StorageError::NullInTag);
    }
    Ok(())
}

fn local_storage() -> Result<web_sys::Storage, StorageError> {
    web_sys::window()
        .ok_or_else(|| StorageError::Browser("window is unavailable".to_owned()))?
        .local_storage()
        .map_err(|error| StorageError::Browser(js_error(&error)))?
        .ok_or_else(|| StorageError::Browser("localStorage is disabled".to_owned()))
}

fn load_data() -> Result<WebData, StorageError> {
    let storage = local_storage()?;
    match storage
        .get_item(STORAGE_KEY)
        .map_err(|error| StorageError::Browser(js_error(&error)))?
    {
        Some(json) => Ok(serde_json::from_str(&json)?),
        None => Ok(WebData::default()),
    }
}

fn save_data(data: &WebData) -> Result<(), StorageError> {
    let json = serde_json::to_string(data)?;
    local_storage()?
        .set_item(STORAGE_KEY, &json)
        .map_err(|error| StorageError::Browser(js_error(&error)))
}

fn js_error(error: &wasm_bindgen::JsValue) -> String {
    error
        .as_string()
        .or_else(|| {
            js_sys::JSON::stringify(error)
                .ok()
                .and_then(|v| v.as_string())
        })
        .unwrap_or_else(|| "unknown browser error".to_owned())
}
