//! Domain types for journal entries and editor input.
//!
//! Validation lives here so both the UI and the storage layer operate on the
//! same normalized values. Storage never receives an unvalidated draft.

use std::collections::HashSet;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Stable identity for a stored journal entry.
pub type EntryId = i64;

/// Largest entry id accepted from a backup.
///
/// This leaves enough room in SQLite's signed 64-bit AUTOINCREMENT sequence for
/// future entries after a restore.
pub const MAX_ENTRY_ID: EntryId = 1_000_000_000_000;

/// A validated, stored journal entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: EntryId,
    /// Local optimistic-concurrency token. It is not portable across backups.
    pub revision: i64,
    pub title: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tags: Vec<String>,
}

/// Raw text from the editor, before validation and normalization.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntryDraft {
    pub title: String,
    pub body: String,
    /// Comma-separated tags as typed by the user.
    pub tags_text: String,
}

/// An editor draft that passed validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedEntry {
    pub title: String,
    pub body: String,
    pub tags: Vec<String>,
}

/// Why an editor draft cannot be saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("Add a title before saving this entry.")]
    EmptyTitle,
}

impl EntryDraft {
    /// Build a draft from a stored entry so it can be edited.
    pub fn from_entry(entry: &JournalEntry) -> Self {
        Self {
            title: entry.title.clone(),
            body: entry.body.clone(),
            tags_text: tags_to_text(&entry.tags),
        }
    }

    /// Validate and normalize the draft for storage.
    pub fn validate(&self) -> Result<ValidatedEntry, ValidationError> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err(ValidationError::EmptyTitle);
        }

        Ok(ValidatedEntry {
            title: title.to_owned(),
            body: self.body.trim_end().to_owned(),
            tags: normalize_tags(&self.tags_text),
        })
    }

    /// Does this draft contain anything worth warning about?
    pub fn has_content(&self) -> bool {
        !self.title.trim().is_empty()
            || !self.body.trim().is_empty()
            || !normalize_tags(&self.tags_text).is_empty()
    }

    /// Compare a draft to its stored entry, using the same normalization that a
    /// save applies. This keeps the "unsaved changes" indicator honest.
    pub fn matches_entry(&self, entry: &JournalEntry) -> bool {
        let mut draft_tags = normalize_tags(&self.tags_text);
        let mut entry_tags = normalize_tags(&tags_to_text(&entry.tags));
        draft_tags.sort_unstable();
        entry_tags.sort_unstable();

        self.title.trim() == entry.title.trim()
            && self.body.trim_end() == entry.body.trim_end()
            && draft_tags == entry_tags
    }
}

/// Normalize a comma-separated tag string.
///
/// * trims surrounding whitespace and leading `#` characters
/// * collapses internal whitespace
/// * drops empty tags
/// * removes duplicates case-insensitively while preserving the first spelling
pub fn normalize_tags(input: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut tags = Vec::new();

    for raw in input.split(',') {
        let tag = raw
            .trim()
            .trim_start_matches(|character: char| character == '#' || character.is_whitespace())
            .trim_end();
        if tag.is_empty() {
            continue;
        }

        let tag = fold_case(&tag.split_whitespace().collect::<Vec<_>>().join(" "));
        if seen.insert(tag.clone()) {
            tags.push(tag);
        }
    }

    tags
}

/// Unicode-aware case folding for search and tags.
///
/// Lowercasing Greek sigma is context-sensitive, so a final sigma (`ς`) and a
/// normal sigma (`σ`) otherwise compare differently. Case folding maps both to
/// the normal form.
fn fold_case(input: &str) -> String {
    input
        .chars()
        .flat_map(char::to_lowercase)
        .map(|character| if character == 'ς' { 'σ' } else { character })
        .collect()
}

/// Render tags as the comma-separated text shown in the editor.
pub fn tags_to_text(tags: &[String]) -> String {
    tags.join(", ")
}

/// Search and tag-filter criteria shared by the UI and storage tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilter {
    pub query: String,
    pub tag: Option<String>,
}

impl SearchFilter {
    pub fn is_active(&self) -> bool {
        !self.query.trim().is_empty() || self.tag.is_some()
    }

    /// Match every query term against the title or body (case-insensitive) and
    /// optionally require a tag (case-insensitive).
    pub fn matches(&self, entry: &JournalEntry) -> bool {
        let title = fold_case(&entry.title);
        let body = fold_case(&entry.body);

        let query_matches = self.query.split_whitespace().all(|term| {
            let term = fold_case(term);
            title.contains(&term) || body.contains(&term)
        });

        if !query_matches {
            return false;
        }

        match &self.tag {
            Some(tag) => {
                let tag = fold_case(tag.trim());
                entry
                    .tags
                    .iter()
                    .any(|entry_tag| fold_case(entry_tag) == tag)
            }
            None => true,
        }
    }
}

/// Convert a timestamp into the canonical RFC 3339 form stored in SQLite.
pub fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

/// Parse a timestamp read from SQLite or a backup file.
pub fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(value).map(|timestamp| timestamp.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn entry(title: &str, body: &str, tags: &[&str]) -> JournalEntry {
        let created = Utc.with_ymd_and_hms(2026, 11, 18, 12, 0, 0).unwrap();
        JournalEntry {
            id: 1,
            revision: 1,
            title: title.to_owned(),
            body: body.to_owned(),
            created_at: created,
            updated_at: created,
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        }
    }

    #[test]
    fn title_is_required_and_trimmed() {
        let draft = EntryDraft {
            title: "   ".to_owned(),
            body: "body".to_owned(),
            tags_text: String::new(),
        };
        assert_eq!(draft.validate(), Err(ValidationError::EmptyTitle));

        let draft = EntryDraft {
            title: "  A day at the river  ".to_owned(),
            body: "body".to_owned(),
            tags_text: String::new(),
        };
        let validated = draft.validate().unwrap();
        assert_eq!(validated.title, "A day at the river");
    }

    #[test]
    fn tags_are_normalized_and_deduplicated() {
        assert_eq!(
            normalize_tags(" Work, #Work,  deep   work , , personal, # personal "),
            vec!["work", "deep work", "personal"]
        );
        assert!(normalize_tags("  , # , ").is_empty());
        assert_eq!(tags_to_text(&["a".into(), "b".into()]), "a, b");
    }

    #[test]
    fn tag_normalization_is_idempotent() {
        let once = normalize_tags("# #Work, ## deep   work, # personal");
        assert_eq!(once, vec!["work", "deep work", "personal"]);
        assert_eq!(normalize_tags(&once.join(", ")), once);
    }

    #[test]
    fn search_matches_title_and_body_terms() {
        let entry = entry(
            "Morning walk",
            "The river was quiet.",
            &["nature", "Personal"],
        );
        assert!(SearchFilter {
            query: "river".into(),
            tag: None
        }
        .matches(&entry));
        assert!(SearchFilter {
            query: "MORNING quiet".into(),
            tag: None
        }
        .matches(&entry));
        assert!(!SearchFilter {
            query: "morning mountain".into(),
            tag: None
        }
        .matches(&entry));
    }

    #[test]
    fn search_filters_by_tag_case_insensitively() {
        let entry = entry("Title", "Body", &["Work", "Ideas"]);
        assert!(SearchFilter {
            query: String::new(),
            tag: Some("work".into())
        }
        .matches(&entry));
        assert!(!SearchFilter {
            query: String::new(),
            tag: Some("home".into())
        }
        .matches(&entry));
    }

    #[test]
    fn drafts_compare_using_save_normalization() {
        let entry = entry("Title", "Body\n", &["Work"]);
        let draft = EntryDraft {
            title: " Title ".into(),
            body: "Body".into(),
            tags_text: "#Work".into(),
        };
        assert!(draft.matches_entry(&entry));
        assert!(draft.has_content());
        assert!(!EntryDraft::default().has_content());
    }

    #[test]
    fn search_folds_greek_sigma_variants() {
        let entry = entry("ΟΣ", "Body text", &["ΟΣ"]);
        assert!(SearchFilter {
            query: "οσ".into(),
            tag: None,
        }
        .matches(&entry));
        assert!(SearchFilter {
            query: String::new(),
            tag: Some("οσ".into()),
        }
        .matches(&entry));
        assert_eq!(normalize_tags("ΟΣ"), vec!["οσ"]);
    }

    #[test]
    fn draft_dirty_check_ignores_tag_order() {
        let entry = entry("Title", "Body", &["personal", "work"]);
        let draft = EntryDraft {
            title: "Title".into(),
            body: "Body".into(),
            tags_text: "#work, #personal".into(),
        };
        assert!(draft.matches_entry(&entry));
    }

    #[test]
    fn timestamps_round_trip_through_storage_format() {
        let timestamp = Utc.with_ymd_and_hms(2026, 11, 18, 12, 34, 56).unwrap();
        let text = format_timestamp(timestamp);
        assert!(text.ends_with('Z'));
        assert_eq!(parse_timestamp(&text).unwrap(), timestamp);
        assert!(parse_timestamp("not a timestamp").is_err());
    }
}
