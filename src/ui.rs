//! egui application shell: journal list, editor, themes, and user feedback.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Local, Utc};
use eframe::CreationContext;
use egui::{
    Align, Align2, Button, Color32, ComboBox, CornerRadius, FontId, Frame, Id, Key,
    KeyboardShortcut, Label, Layout, Margin, Modifiers, Response, RichText, ScrollArea, Sense,
    Stroke, TextEdit, TextStyle, Ui, ViewportCommand,
};

use crate::backup;
use crate::model::{normalize_tags, EntryDraft, EntryId, JournalEntry, SearchFilter};
use crate::storage::{Storage, StorageError};

const THEME_SETTING: &str = "theme";
const TEXT_SIZE_SETTING: &str = "text_size";
const OPEN_RECENT_SETTING: &str = "open_recent";
const SIDEBAR_WIDTH: f32 = 344.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Theme {
    Warm,
    Dark,
    Slate,
    Forest,
}

impl Theme {
    const ALL: [Self; 4] = [Self::Warm, Self::Dark, Self::Slate, Self::Forest];

    fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("dark") => Self::Dark,
            Some("slate") => Self::Slate,
            Some("forest") => Self::Forest,
            _ => Self::Warm,
        }
    }

    fn setting_value(self) -> &'static str {
        match self {
            Self::Warm => "warm",
            Self::Dark => "dark",
            Self::Slate => "slate",
            Self::Forest => "forest",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Warm => "Warm",
            Self::Dark => "Dark",
            Self::Slate => "Slate",
            Self::Forest => "Forest",
        }
    }

    fn is_dark(self) -> bool {
        matches!(self, Self::Dark | Self::Slate)
    }

    fn palette(self) -> Palette {
        match self {
            Self::Warm => Palette {
                background: Color32::from_rgb(246, 241, 231),
                sidebar: Color32::from_rgb(239, 231, 217),
                field: Color32::from_rgb(252, 249, 243),
                text: Color32::from_rgb(59, 50, 39),
                weak_text: Color32::from_rgb(122, 107, 89),
                accent: Color32::from_rgb(150, 78, 38),
                accent_text: Color32::from_rgb(255, 249, 241),
                accent_soft: Color32::from_rgb(228, 211, 184),
                hover: Color32::from_rgb(234, 224, 206),
                stroke: Color32::from_rgb(217, 203, 182),
                danger: Color32::from_rgb(166, 58, 46),
                on_danger: Color32::from_rgb(255, 247, 242),
                success: Color32::from_rgb(63, 107, 63),
                warning: Color32::from_rgb(138, 90, 0),
            },
            Self::Dark => Palette {
                background: Color32::from_rgb(21, 20, 25),
                sidebar: Color32::from_rgb(29, 27, 33),
                field: Color32::from_rgb(36, 33, 40),
                text: Color32::from_rgb(234, 226, 212),
                weak_text: Color32::from_rgb(164, 154, 139),
                accent: Color32::from_rgb(212, 162, 76),
                accent_text: Color32::from_rgb(27, 23, 16),
                accent_soft: Color32::from_rgb(69, 58, 37),
                hover: Color32::from_rgb(43, 40, 48),
                stroke: Color32::from_rgb(55, 50, 60),
                danger: Color32::from_rgb(229, 133, 122),
                on_danger: Color32::from_rgb(35, 18, 16),
                success: Color32::from_rgb(143, 191, 135),
                warning: Color32::from_rgb(224, 177, 94),
            },
            Self::Slate => Palette {
                background: Color32::from_rgb(18, 22, 28),
                sidebar: Color32::from_rgb(24, 30, 38),
                field: Color32::from_rgb(32, 40, 50),
                text: Color32::from_rgb(227, 234, 242),
                weak_text: Color32::from_rgb(147, 161, 179),
                accent: Color32::from_rgb(95, 168, 211),
                accent_text: Color32::from_rgb(13, 20, 27),
                accent_soft: Color32::from_rgb(42, 63, 82),
                hover: Color32::from_rgb(35, 45, 57),
                stroke: Color32::from_rgb(51, 64, 78),
                danger: Color32::from_rgb(224, 122, 122),
                on_danger: Color32::from_rgb(27, 15, 16),
                success: Color32::from_rgb(127, 200, 169),
                warning: Color32::from_rgb(227, 179, 65),
            },
            Self::Forest => Palette {
                background: Color32::from_rgb(241, 244, 236),
                sidebar: Color32::from_rgb(228, 234, 219),
                field: Color32::from_rgb(249, 251, 245),
                text: Color32::from_rgb(45, 58, 46),
                weak_text: Color32::from_rgb(109, 124, 106),
                accent: Color32::from_rgb(63, 107, 79),
                accent_text: Color32::from_rgb(247, 251, 247),
                accent_soft: Color32::from_rgb(201, 220, 201),
                hover: Color32::from_rgb(221, 230, 213),
                stroke: Color32::from_rgb(200, 211, 192),
                danger: Color32::from_rgb(166, 75, 60),
                on_danger: Color32::from_rgb(255, 247, 242),
                success: Color32::from_rgb(63, 107, 79),
                warning: Color32::from_rgb(138, 90, 0),
            },
        }
    }

    fn theme_preference(self) -> egui::ThemePreference {
        if self.is_dark() {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        }
    }

    fn egui_theme(self) -> egui::Theme {
        if self.is_dark() {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        }
    }

    fn visuals(self) -> egui::Visuals {
        let palette = self.palette();
        let mut visuals = if self.is_dark() {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };

        visuals.dark_mode = self.is_dark();
        visuals.panel_fill = palette.background;
        visuals.window_fill = palette.sidebar;
        visuals.extreme_bg_color = palette.field;
        visuals.text_edit_bg_color = Some(palette.field);
        visuals.faint_bg_color = palette.hover;
        visuals.selection.bg_fill = palette.accent_soft;
        visuals.selection.stroke = Stroke::new(1.0, palette.accent);
        visuals.hyperlink_color = palette.accent;
        visuals.warn_fg_color = palette.warning;
        visuals.error_fg_color = palette.danger;
        visuals.window_stroke = Stroke::new(1.0, palette.stroke);
        visuals.window_corner_radius = CornerRadius::same(12);
        visuals.menu_corner_radius = CornerRadius::same(10);

        visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.stroke);
        visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
        visuals.widgets.inactive.bg_fill = palette.field;
        visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
        visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.stroke);
        visuals.widgets.hovered.weak_bg_fill = palette.hover;
        visuals.widgets.hovered.bg_fill = palette.hover;
        visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, palette.text);
        visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette.stroke);
        visuals.widgets.active.weak_bg_fill = palette.accent_soft;
        visuals.widgets.active.bg_fill = palette.accent_soft;
        visuals.widgets.active.fg_stroke = Stroke::new(1.0, palette.text);
        visuals.widgets.active.bg_stroke = Stroke::new(1.0, palette.accent);
        visuals.widgets.open.weak_bg_fill = palette.hover;
        visuals.widgets.open.bg_fill = palette.hover;
        visuals.widgets.open.fg_stroke = Stroke::new(1.0, palette.text);
        visuals.widgets.open.bg_stroke = Stroke::new(1.0, palette.accent);

        visuals
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextSize {
    Small,
    Default,
    Large,
}

impl TextSize {
    const ALL: [Self; 3] = [Self::Small, Self::Default, Self::Large];

    fn from_setting(value: Option<&str>) -> Self {
        match value {
            Some("small") => Self::Small,
            Some("large") => Self::Large,
            _ => Self::Default,
        }
    }

    fn setting_value(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Default => "default",
            Self::Large => "large",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Small => "Small",
            Self::Default => "Default",
            Self::Large => "Large",
        }
    }

    fn zoom_factor(self) -> f32 {
        match self {
            Self::Small => 0.9,
            Self::Default => 1.0,
            Self::Large => 1.15,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MainTab {
    Entry,
    Settings,
}

#[derive(Debug, Clone, Copy)]
struct Palette {
    background: Color32,
    sidebar: Color32,
    field: Color32,
    text: Color32,
    weak_text: Color32,
    accent: Color32,
    accent_text: Color32,
    accent_soft: Color32,
    hover: Color32,
    stroke: Color32,
    danger: Color32,
    on_danger: Color32,
    success: Color32,
    warning: Color32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorTarget {
    New,
    Entry { id: EntryId, revision: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NoticeKind {
    Info,
    Success,
    Error,
}

#[derive(Debug)]
struct Notice {
    kind: NoticeKind,
    message: String,
}

impl Notice {
    fn info(message: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Info,
            message: message.into(),
        }
    }

    fn success(message: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Success,
            message: message.into(),
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            kind: NoticeKind::Error,
            message: message.into(),
        }
    }
}

enum Modal {
    Delete {
        id: EntryId,
        title: String,
        revision: i64,
    },
    UnsavedSwitch {
        target: EditorTarget,
    },
    UnsavedClose,
    ConfirmRevert,
    ReloadBeforeDelete,
    Import {
        entries: Vec<JournalEntry>,
        file_name: String,
        expected_revisions: Vec<(EntryId, i64)>,
    },
}

enum Action {
    NewEntry,
    Select(EntryId, i64),
    OpenTab(MainTab),
    SetTheme(Theme),
    SetTextSize(TextSize),
    SetOpenRecent(bool),
    Save,
    Revert,
    RequestDelete,
    Export,
    Import,
    Refresh,
    DismissNotice,
    ConfirmDelete(EntryId, i64),
    ConfirmRevert,
    SaveBeforeSwitch(EditorTarget),
    DiscardAndSwitch(EditorTarget),
    SaveBeforeClose,
    DiscardAndClose,
    ApplyImport(Vec<JournalEntry>, Vec<(EntryId, i64)>),
}

/// The native application state.
pub struct EJournalApp {
    storage: Storage,
    db_path: PathBuf,
    entries: Vec<JournalEntry>,
    visible_indices: Vec<usize>,
    tags: Vec<String>,
    selected_id: Option<EntryId>,
    base_entry: Option<JournalEntry>,
    is_new: bool,
    draft: EntryDraft,
    editor_generation: u64,
    search: String,
    tag_filter: Option<String>,
    tab: MainTab,
    theme: Theme,
    text_size: TextSize,
    open_recent: bool,
    notice: Option<Notice>,
    modal: Option<Modal>,
    focus_title: bool,
    focus_search: bool,
    allow_close: bool,
}

impl EJournalApp {
    pub fn new(cc: &CreationContext<'_>, storage: Storage, db_path: PathBuf) -> Self {
        let mut theme = Theme::Warm;
        let mut text_size = TextSize::Default;
        let mut open_recent = true;
        let mut notice = None;

        match storage.get_setting(THEME_SETTING) {
            Ok(Some(value)) => theme = Theme::from_setting(Some(&value)),
            Ok(None) => {}
            Err(error) => {
                notice = Some(Notice::error(format!(
                    "Could not load the saved theme: {error}"
                )));
            }
        }
        match storage.get_setting(TEXT_SIZE_SETTING) {
            Ok(Some(value)) => text_size = TextSize::from_setting(Some(&value)),
            Ok(None) => {}
            Err(error) => {
                notice.get_or_insert_with(|| {
                    Notice::error(format!("Could not load the saved text size: {error}"))
                });
            }
        }
        match storage.get_setting(OPEN_RECENT_SETTING) {
            Ok(Some(value)) => open_recent = value != "false",
            Ok(None) => {}
            Err(error) => {
                notice.get_or_insert_with(|| {
                    Notice::error(format!("Could not load the saved startup setting: {error}"))
                });
            }
        }

        configure_context(&cc.egui_ctx, theme, text_size);

        let mut app = Self {
            storage,
            db_path,
            entries: Vec::new(),
            visible_indices: Vec::new(),
            tags: Vec::new(),
            selected_id: None,
            base_entry: None,
            is_new: false,
            draft: EntryDraft::default(),
            editor_generation: 0,
            search: String::new(),
            tag_filter: None,
            tab: MainTab::Entry,
            theme,
            text_size,
            open_recent,
            notice,
            modal: None,
            focus_title: false,
            focus_search: false,
            allow_close: false,
        };

        let loaded = app.refresh_data();
        match loaded {
            Ok(()) => {
                if app.open_recent {
                    if let Some(entry) = app.entries.first() {
                        app.open_entry(entry.id, entry.revision);
                    }
                }
            }
            Err(error) => {
                app.notice = Some(Notice::error(format!(
                    "Could not load your journal: {error}"
                )));
            }
        }

        app
    }

    fn refresh_data(&mut self) -> Result<(), StorageError> {
        let entries = self.storage.list_entries()?;
        let tags = self.storage.all_tags()?;
        self.entries = entries;
        self.tags = tags;

        if let Some(tag) = &self.tag_filter {
            if !self
                .tags
                .iter()
                .any(|available| available.eq_ignore_ascii_case(tag))
            {
                self.tag_filter = None;
            }
        }

        if let Some(id) = self.selected_id {
            if !self.entries.iter().any(|entry| entry.id == id) {
                let had_unsaved_changes = self.is_dirty();
                self.selected_id = None;
                self.base_entry = None;
                if had_unsaved_changes {
                    self.is_new = true;
                    self.notice = Some(Notice::info(
                        "That entry was removed outside E-Journal. Your draft is still open; \
                         save it to keep it as a new entry.",
                    ));
                } else {
                    self.is_new = false;
                    self.draft = EntryDraft::default();
                    self.editor_generation = self.editor_generation.wrapping_add(1);
                }
            }
        }

        let filter = self.current_filter();
        self.refresh_visible(&filter);

        Ok(())
    }

    fn current_filter(&self) -> SearchFilter {
        SearchFilter {
            query: self.search.clone(),
            tag: self.tag_filter.clone(),
        }
    }

    fn refresh_visible(&mut self, filter: &SearchFilter) {
        let prepared = filter.prepare();
        self.visible_indices = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| prepared.matches(entry))
            .map(|(index, _)| index)
            .collect();
    }

    fn reload_data(&mut self) {
        if let Err(error) = self.refresh_data() {
            self.notice = Some(Notice::error(format!(
                "Could not refresh your journal: {error}"
            )));
        }
    }

    fn open_entry(&mut self, id: EntryId, expected_revision: i64) {
        match self.storage.get_entry(id) {
            Ok(Some(entry)) if entry.revision != expected_revision => {
                self.notice = Some(Notice::info(
                    "That entry changed in another window. The journal was refreshed; \
                     select the entry again.",
                ));
                self.reload_data();
            }
            Ok(Some(entry)) => {
                self.selected_id = Some(id);
                self.base_entry = Some(entry.clone());
                self.is_new = false;
                self.draft = EntryDraft::from_entry(&entry);
                self.editor_generation = self.editor_generation.wrapping_add(1);
                if let Err(error) = self.refresh_data() {
                    self.notice = Some(Notice::error(format!(
                        "Loaded the entry, but the journal list could not refresh: {error}"
                    )));
                }
            }
            Ok(None) => {
                self.notice = Some(Notice::error("That entry is no longer in the journal."));
                self.reload_data();
            }
            Err(error) => {
                self.notice = Some(Notice::error(format!(
                    "Could not load that entry. Your current draft is unchanged. {error}"
                )));
            }
        }
    }

    #[cfg(test)]
    fn open_entry_cached(&mut self, id: EntryId) {
        let revision = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .expect("entry in cache")
            .revision;
        self.open_entry(id, revision);
    }

    fn current_target(&self) -> Option<EditorTarget> {
        if self.is_new {
            Some(EditorTarget::New)
        } else {
            self.selected_id.map(|id| EditorTarget::Entry {
                id,
                revision: self.base_entry.as_ref().map_or(0, |entry| entry.revision),
            })
        }
    }

    fn is_dirty(&self) -> bool {
        match (self.selected_id, self.base_entry.as_ref()) {
            (Some(_), Some(base)) => !self.draft.matches_entry(base),
            (Some(_), None) => self.draft.has_content(),
            (None, _) => self.is_new && self.draft.has_content(),
        }
    }

    fn has_stale_revision(&self) -> bool {
        match (self.selected_id, self.base_entry.as_ref()) {
            (Some(id), Some(base)) => self
                .entries
                .iter()
                .find(|entry| entry.id == id)
                .is_some_and(|entry| entry.revision != base.revision),
            _ => false,
        }
    }

    fn request_switch(&mut self, target: EditorTarget) {
        if self.current_target() == Some(target) {
            if let EditorTarget::Entry { id, .. } = target {
                match self.storage.get_entry(id) {
                    Ok(Some(entry))
                        if self.base_entry.as_ref().map(|base| base.revision)
                            != Some(entry.revision) =>
                    {
                        self.modal = Some(Modal::ConfirmRevert);
                    }
                    Ok(None) => {
                        self.notice =
                            Some(Notice::error("That entry is no longer in the journal."));
                        self.reload_data();
                    }
                    Ok(Some(_)) => {}
                    Err(error) => {
                        self.notice = Some(Notice::error(format!(
                            "Could not check the entry for changes: {error}"
                        )));
                    }
                }
            }
            return;
        }

        if self.is_dirty() {
            self.modal = Some(Modal::UnsavedSwitch { target });
        } else {
            self.perform_switch(target);
        }
    }

    fn perform_switch(&mut self, target: EditorTarget) {
        match target {
            EditorTarget::New => {
                self.selected_id = None;
                self.base_entry = None;
                self.is_new = true;
                self.draft = EntryDraft::default();
                self.editor_generation = self.editor_generation.wrapping_add(1);
                self.focus_title = true;
            }
            EditorTarget::Entry { id, revision } => self.open_entry(id, revision),
        }
    }

    fn save_current(&mut self) -> Result<(), String> {
        let validated = match self.draft.validate() {
            Ok(validated) => validated,
            Err(error) => {
                self.focus_title = true;
                return Err(error.to_string());
            }
        };

        let was_new = self.selected_id.is_none();
        let save_result = match self.selected_id {
            Some(id) => {
                let Some(expected_revision) = self.base_entry.as_ref().map(|entry| entry.revision)
                else {
                    return Err(
                        "This entry is no longer available. Reopen it before saving.".to_owned(),
                    );
                };
                self.storage.update_entry(id, expected_revision, &validated)
            }
            None => self.storage.create_entry(&validated),
        };

        let saved = match save_result {
            Ok(saved) => saved,
            Err(StorageError::Conflict(_)) => {
                let refresh_message = match self.refresh_data() {
                    Ok(()) => String::new(),
                    Err(error) => format!(" The journal list also failed to refresh: {error}"),
                };
                return Err(format!(
                    "This entry changed in another window. Your draft is still open; choose \
                     Revert to load the latest version before saving again.{refresh_message}"
                ));
            }
            Err(StorageError::NotFound(_)) => {
                self.selected_id = None;
                self.base_entry = None;
                self.is_new = true;
                if let Err(error) = self.refresh_data() {
                    return Err(format!(
                        "That entry was deleted outside E-Journal, and the journal list could \
                         not refresh: {error}. Your draft is still open."
                    ));
                }
                return Err(
                    "That entry was deleted outside E-Journal. Your draft is still open; \
                     choose Save again to keep it as a new entry."
                        .to_owned(),
                );
            }
            Err(error) => return Err(format!("Could not save this entry: {error}")),
        };

        self.selected_id = Some(saved.id);
        self.base_entry = Some(saved.clone());
        self.is_new = false;
        self.draft = EntryDraft::from_entry(&saved);

        match self.refresh_data() {
            Ok(()) => {
                self.notice = Some(Notice::success(if was_new {
                    "Entry created."
                } else {
                    "Changes saved."
                }));
            }
            Err(error) => {
                self.notice = Some(Notice::error(format!(
                    "The entry was saved, but the list could not be refreshed: {error}"
                )));
            }
        }

        Ok(())
    }

    fn delete_entry(&mut self, id: EntryId, revision: i64) {
        match self.storage.delete_entry(id, revision) {
            Ok(true) => {
                if self.selected_id == Some(id) {
                    self.selected_id = None;
                    self.base_entry = None;
                    self.is_new = false;
                    self.draft = EntryDraft::default();
                    self.editor_generation = self.editor_generation.wrapping_add(1);
                }
                match self.refresh_data() {
                    Ok(()) => {
                        self.notice = Some(Notice::success("Entry deleted."));
                    }
                    Err(error) => {
                        self.notice = Some(Notice::error(format!(
                            "Entry deleted, but the journal list could not be refreshed: {error}"
                        )));
                    }
                }
            }
            Ok(false) => {
                self.notice = Some(Notice::error("That entry was already deleted."));
                self.reload_data();
            }
            Err(StorageError::Conflict(_)) => {
                let refresh = self.refresh_data();
                let message = match refresh {
                    Ok(()) => {
                        "That entry changed before it was deleted. Review it and confirm again."
                            .to_owned()
                    }
                    Err(error) => format!(
                        "That entry changed before it was deleted. The journal list could \
                         not refresh: {error}. Review the entry before confirming again."
                    ),
                };
                self.notice = Some(Notice::error(message));
            }
            Err(error) => {
                self.notice = Some(Notice::error(format!(
                    "Could not delete this entry: {error}"
                )));
            }
        }
    }

    fn apply_import(
        &mut self,
        entries: Vec<JournalEntry>,
        expected_revisions: Vec<(EntryId, i64)>,
    ) {
        let imported_count = entries.len();
        match backup::apply_import_checked(&mut self.storage, &entries, &expected_revisions) {
            Ok(()) => {
                self.selected_id = None;
                self.base_entry = None;
                self.is_new = false;
                self.draft = EntryDraft::default();
                self.editor_generation = self.editor_generation.wrapping_add(1);
                match self.refresh_data() {
                    Ok(()) => {
                        self.notice = Some(Notice::success(format!(
                            "Imported {imported_count} entries from the backup."
                        )));
                    }
                    Err(error) => {
                        self.notice = Some(Notice::error(format!(
                            "The backup was imported, but the list could not be refreshed: {error}"
                        )));
                    }
                }
            }
            Err(backup::BackupError::Storage(StorageError::JournalChanged)) => {
                let refresh = self.refresh_data();
                let message = match refresh {
                    Ok(()) => "The journal changed after you opened this import. Review the \
                              current entries and import again. Nothing was changed."
                        .to_owned(),
                    Err(error) => format!(
                        "The journal changed after you opened this import. Nothing was \
                         changed. The journal list could not refresh: {error}"
                    ),
                };
                self.notice = Some(Notice::error(message));
            }
            Err(error) => {
                self.notice = Some(Notice::error(format!(
                    "Import failed. Your existing journal was not changed. {error}"
                )));
            }
        }
    }

    fn export_backup(&mut self) {
        let default_name = format!("journal-backup-{}.json", Local::now().format("%Y-%m-%d"));
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Journal backup", &["json"])
            .set_file_name(default_name)
            .set_title("Export journal backup")
            .save_file()
        else {
            return;
        };

        if export_destination_is_database(&path, &self.db_path) {
            self.notice = Some(Notice::error(
                "Choose a different file. That path is the active journal database or one \
                 of its SQLite sidecar files.",
            ));
            return;
        }

        let entries = match self.storage.list_entries() {
            Ok(entries) => entries,
            Err(error) => {
                self.notice = Some(Notice::error(format!(
                    "Could not read the journal for export: {error}"
                )));
                return;
            }
        };

        match backup::write_backup(&path, &entries) {
            Ok(()) => {
                self.notice = Some(Notice::success(format!(
                    "Backup exported to {}.",
                    path.display()
                )));
            }
            Err(error) => {
                self.notice = Some(Notice::error(format!(
                    "Could not export the backup: {error}"
                )));
            }
        }
    }

    fn choose_import(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Journal backup", &["json"])
            .set_title("Import journal backup")
            .pick_file()
        else {
            return;
        };

        match backup::read_backup(&path) {
            Ok(entries) => {
                let expected_revisions = match self.storage.revision_snapshot() {
                    Ok(revisions) => revisions,
                    Err(error) => {
                        self.notice = Some(Notice::error(format!(
                            "Could not read the current journal revisions, so the import \
                             was not opened. Your journal was not changed. {error}"
                        )));
                        return;
                    }
                };
                let refresh_error = self.refresh_data().err();
                let file_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("backup")
                    .to_owned();
                self.modal = Some(Modal::Import {
                    entries,
                    file_name,
                    expected_revisions,
                });
                if let Some(error) = refresh_error {
                    self.notice = Some(Notice::error(format!(
                        "Some journal entries could not be displayed, but the import \
                         can still replace the journal: {error}"
                    )));
                }
            }
            Err(error) => {
                self.notice = Some(Notice::error(format!(
                    "Could not import that backup. Your journal was not changed. {error}"
                )));
            }
        }
    }

    fn set_theme(&mut self, ctx: &egui::Context, theme: Theme) {
        if self.theme == theme {
            return;
        }
        self.theme = theme;
        apply_theme(ctx, theme);

        if let Err(error) = self
            .storage
            .set_setting(THEME_SETTING, theme.setting_value())
        {
            self.notice = Some(Notice::error(format!(
                "The theme changed for this session, but it could not be saved: {error}"
            )));
        }
    }

    fn set_text_size(&mut self, ctx: &egui::Context, text_size: TextSize) {
        if self.text_size == text_size {
            return;
        }
        self.text_size = text_size;
        ctx.set_zoom_factor(text_size.zoom_factor());

        if let Err(error) = self
            .storage
            .set_setting(TEXT_SIZE_SETTING, text_size.setting_value())
        {
            self.notice = Some(Notice::error(format!(
                "The text size changed for this session, but it could not be saved: {error}"
            )));
        }
    }

    fn set_open_recent(&mut self, open_recent: bool) {
        if self.open_recent == open_recent {
            return;
        }
        self.open_recent = open_recent;

        if let Err(error) = self.storage.set_setting(
            OPEN_RECENT_SETTING,
            if open_recent { "true" } else { "false" },
        ) {
            self.notice = Some(Notice::error(format!(
                "The startup setting changed for this session, but it could not be saved: {error}"
            )));
        }
    }

    fn apply_action(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::NewEntry => {
                self.tab = MainTab::Entry;
                self.request_switch(EditorTarget::New);
            }
            Action::Select(id, revision) => {
                self.tab = MainTab::Entry;
                self.request_switch(EditorTarget::Entry { id, revision });
            }
            Action::OpenTab(tab) => self.tab = tab,
            Action::SetTheme(theme) => self.set_theme(ctx, theme),
            Action::SetTextSize(text_size) => self.set_text_size(ctx, text_size),
            Action::SetOpenRecent(open_recent) => self.set_open_recent(open_recent),
            Action::Save => {
                if self.is_dirty() {
                    if let Err(message) = self.save_current() {
                        self.notice = Some(Notice::error(message));
                    }
                }
            }
            Action::Revert => {
                if self.is_dirty() || self.has_stale_revision() {
                    self.modal = Some(Modal::ConfirmRevert);
                } else {
                    self.notice = Some(Notice::info("There are no changes to revert."));
                }
            }
            Action::ConfirmRevert => {
                self.modal = None;
                if let Some(id) = self.selected_id {
                    match self.storage.get_entry(id) {
                        Ok(Some(entry)) => {
                            self.draft = EntryDraft::from_entry(&entry);
                            self.editor_generation = self.editor_generation.wrapping_add(1);
                            self.base_entry = Some(entry);
                            if let Err(error) = self.refresh_data() {
                                self.notice = Some(Notice::info(format!(
                                    "Reloaded the entry, but the journal list could not \
                                     refresh: {error}"
                                )));
                            } else {
                                self.notice = Some(Notice::info("Reloaded the latest entry."));
                            }
                        }
                        Ok(None) => {
                            let had_unsaved_changes = self.is_dirty();
                            self.selected_id = None;
                            self.base_entry = None;
                            if had_unsaved_changes {
                                self.is_new = true;
                                self.notice = Some(Notice::error(
                                    "That entry was deleted outside E-Journal. Your draft is \
                                     still open; save it to keep it as a new entry.",
                                ));
                            } else {
                                self.is_new = false;
                                self.draft = EntryDraft::default();
                                self.editor_generation = self.editor_generation.wrapping_add(1);
                                self.notice = Some(Notice::error("That entry no longer exists."));
                            }
                            if let Err(error) = self.refresh_data() {
                                self.notice = Some(Notice::error(format!(
                                    "The entry is gone, and the journal list could not \
                                     refresh: {error}"
                                )));
                            }
                        }
                        Err(error) => {
                            self.notice = Some(Notice::error(format!(
                                "Could not reload the entry. Your draft is unchanged. {error}"
                            )));
                        }
                    }
                } else if self.is_new {
                    self.draft = EntryDraft::default();
                    self.editor_generation = self.editor_generation.wrapping_add(1);
                    self.base_entry = None;
                    self.notice = Some(Notice::info("Discarded the new entry draft."));
                }
            }
            Action::RequestDelete => {
                if self.has_stale_revision() {
                    self.modal = Some(Modal::ReloadBeforeDelete);
                    return;
                }
                if let Some(id) = self.selected_id {
                    if let Some(entry) = self.entries.iter().find(|entry| entry.id == id) {
                        self.modal = Some(Modal::Delete {
                            id,
                            title: entry.title.clone(),
                            revision: entry.revision,
                        });
                    }
                }
            }
            Action::Export => self.export_backup(),
            Action::Import => self.choose_import(),
            Action::Refresh => match self.refresh_data() {
                Ok(()) => {
                    if self.notice.is_none() {
                        self.notice = Some(Notice::success("Journal refreshed."));
                    }
                }
                Err(error) => {
                    self.notice = Some(Notice::error(format!(
                        "Could not refresh the journal: {error}"
                    )));
                }
            },
            Action::DismissNotice => self.notice = None,
            Action::ConfirmDelete(id, revision) => {
                self.modal = None;
                self.delete_entry(id, revision);
            }
            Action::SaveBeforeSwitch(target) => match self.save_current() {
                Ok(()) => self.perform_switch(target),
                Err(message) => self.notice = Some(Notice::error(message)),
            },
            Action::DiscardAndSwitch(target) => self.perform_switch(target),
            Action::SaveBeforeClose => match self.save_current() {
                Ok(()) => {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
                Err(message) => self.notice = Some(Notice::error(message)),
            },
            Action::DiscardAndClose => {
                self.allow_close = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
            Action::ApplyImport(entries, expected_revisions) => {
                self.apply_import(entries, expected_revisions)
            }
        }

        ctx.request_repaint();
    }

    fn handle_close_request(&mut self, ctx: &egui::Context) -> bool {
        if self.allow_close || !ctx.input(|input| input.viewport().close_requested()) {
            return false;
        }

        if self.is_dirty() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.modal = Some(Modal::UnsavedClose);
            true
        } else {
            false
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if self.modal.is_some() {
            return;
        }

        let save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
        let new_entry = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
        let find = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);

        if ctx.input_mut(|input| input.consume_shortcut(&save)) {
            actions.push(Action::Save);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&new_entry)) {
            actions.push(Action::NewEntry);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&find)) {
            self.focus_search = true;
        }
    }

    fn draw_sidebar(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let palette = self.theme.palette();

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("E-Journal")
                    .size(25.0)
                    .strong()
                    .color(palette.text),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (label, tab) = match self.tab {
                    MainTab::Entry => ("Settings", MainTab::Settings),
                    MainTab::Settings => ("Entries", MainTab::Entry),
                };
                if ui
                    .add(
                        Button::new(RichText::new(label).small().color(palette.weak_text))
                            .frame(false),
                    )
                    .on_hover_text("Open appearance, data, and about settings")
                    .clicked()
                {
                    actions.push(Action::OpenTab(tab));
                }
            });
        });
        ui.label(
            RichText::new("A local place for your thoughts")
                .small()
                .color(palette.weak_text),
        );
        ui.add_space(6.0);

        if primary_button(ui, "+  New entry", &palette, ui.available_width()).clicked() {
            actions.push(Action::NewEntry);
        }
        ui.add_space(3.0);

        let previous_filter = self.current_filter();
        let search = ui.add(
            TextEdit::singleline(&mut self.search)
                .hint_text("Search title or body…")
                .desired_width(f32::INFINITY)
                .margin(Margin::symmetric(10, 7)),
        );
        if self.focus_search {
            search.request_focus();
            self.focus_search = false;
        }

        let tags = self.tags.clone();
        let mut selected_tag = self.tag_filter.clone();
        ComboBox::from_id_salt("tag_filter")
            .selected_text(match &selected_tag {
                Some(tag) => format!("#{tag}"),
                None => "All tags".to_owned(),
            })
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(selected_tag.is_none(), "All tags")
                    .clicked()
                {
                    selected_tag = None;
                }
                for tag in &tags {
                    let selected = selected_tag.as_deref() == Some(tag.as_str());
                    if ui.selectable_label(selected, format!("#{tag}")).clicked() {
                        selected_tag = Some(tag.clone());
                    }
                }
            });
        self.tag_filter = selected_tag;

        if previous_filter.is_active()
            && ui
                .add(
                    Button::new(
                        RichText::new("Clear search and filters")
                            .small()
                            .color(palette.weak_text),
                    )
                    .frame(false),
                )
                .clicked()
        {
            self.search.clear();
            self.tag_filter = None;
        }

        let filter = self.current_filter();
        if filter != previous_filter {
            self.refresh_visible(&filter);
        }

        ui.add_space(2.0);
        ui.separator();

        let visible_count = self.visible_indices.len();
        let list_height = (ui.available_height() - 88.0).max(120.0);
        ScrollArea::vertical()
            .id_salt("entry_list")
            .max_height(list_height)
            .auto_shrink([false, false])
            .show_rows(ui, 62.0, visible_count, |ui, row_range| {
                if visible_count == 0 {
                    let message = if self.entries.is_empty() {
                        "No entries yet. Start with a blank page."
                    } else {
                        "No entries match this search."
                    };
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.add(
                            Label::new(RichText::new(message).small().color(palette.weak_text))
                                .wrap(),
                        );
                    });
                }

                for row in row_range {
                    let entry = &self.entries[self.visible_indices[row]];
                    let selected = !self.is_new && self.selected_id == Some(entry.id);
                    if entry_row(ui, entry, selected, &palette).clicked() {
                        actions.push(Action::Select(entry.id, entry.revision));
                    }
                }
            });

        ui.separator();
        ui.horizontal(|ui| {
            if ui
                .add(
                    Button::new(RichText::new("Export").small().color(palette.weak_text))
                        .frame(false),
                )
                .on_hover_text("Write a complete JSON backup to a file you choose")
                .clicked()
            {
                actions.push(Action::Export);
            }
            if ui
                .add(
                    Button::new(RichText::new("Import").small().color(palette.weak_text))
                        .frame(false),
                )
                .on_hover_text("Restore the journal from a JSON backup")
                .clicked()
            {
                actions.push(Action::Import);
            }
            if ui
                .add(
                    Button::new(RichText::new("Refresh").small().color(palette.weak_text))
                        .frame(false),
                )
                .on_hover_text("Reload entries and tags from the local database")
                .clicked()
            {
                actions.push(Action::Refresh);
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let count = if filter.is_active() {
                    format!("{visible_count} of {} entries", self.entries.len())
                } else {
                    format!("{} entries", self.entries.len())
                };
                ui.label(RichText::new(count).small().color(palette.weak_text));
            });
        });
        ui.label(
            RichText::new("Local only · stored on this computer")
                .small()
                .color(palette.weak_text),
        )
        .on_hover_text(format!("SQLite database: {}", self.db_path.display()));
    }

    fn draw_central(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        self.draw_notice(ui, actions);
        self.draw_tabs(ui, actions);

        match self.tab {
            MainTab::Entry => self.draw_entry_pane(ui, actions),
            MainTab::Settings => self.draw_settings(ui, actions),
        }
    }

    fn draw_tabs(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let palette = self.theme.palette();
        ui.horizontal(|ui| {
            for (tab, label) in [(MainTab::Entry, "Entry"), (MainTab::Settings, "Settings")] {
                let selected = self.tab == tab;
                let response = ui.add(
                    Button::new(RichText::new(label).strong().color(if selected {
                        palette.accent
                    } else {
                        palette.weak_text
                    }))
                    .frame(false),
                );
                if selected {
                    let rect = response.rect;
                    ui.painter().line_segment(
                        [
                            egui::pos2(rect.left(), rect.bottom() + 2.0),
                            egui::pos2(rect.right(), rect.bottom() + 2.0),
                        ],
                        Stroke::new(2.0, palette.accent),
                    );
                }
                if response.clicked() && !selected {
                    actions.push(Action::OpenTab(tab));
                }
            }
        });
        ui.add_space(2.0);
        ui.separator();
        ui.add_space(6.0);
    }

    fn draw_settings(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let palette = self.theme.palette();

        ScrollArea::vertical()
            .id_salt("settings")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Settings")
                        .size(26.0)
                        .strong()
                        .color(palette.text),
                );
                ui.label(
                    RichText::new("Preferences are saved with your journal on this computer.")
                        .small()
                        .color(palette.weak_text),
                );

                settings_section(ui, "Appearance", &palette);
                ui.label(RichText::new("Theme").color(palette.text));
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    for theme in Theme::ALL {
                        let selected = self.theme == theme;
                        let theme_palette = theme.palette();
                        let response = ui.add_sized(
                            [116.0, 44.0],
                            Button::new(
                                RichText::new(theme.label())
                                    .strong()
                                    .color(theme_palette.text),
                            )
                            .fill(theme_palette.sidebar)
                            .stroke(Stroke::new(
                                if selected { 2.0 } else { 1.0 },
                                if selected {
                                    palette.accent
                                } else {
                                    theme_palette.stroke
                                },
                            ))
                            .corner_radius(8),
                        );
                        if response.clicked() {
                            actions.push(Action::SetTheme(theme));
                        }
                    }
                });

                ui.add_space(12.0);
                ui.label(RichText::new("Text size").color(palette.text));
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    for size in TextSize::ALL {
                        if ui
                            .selectable_label(self.text_size == size, size.label())
                            .clicked()
                        {
                            actions.push(Action::SetTextSize(size));
                        }
                    }
                });
                ui.label(
                    RichText::new("Changes the size of the whole interface.")
                        .small()
                        .color(palette.weak_text),
                );

                settings_section(ui, "Startup", &palette);
                let mut open_recent = self.open_recent;
                if ui
                    .checkbox(
                        &mut open_recent,
                        "Open the most recent entry when the app starts",
                    )
                    .changed()
                {
                    actions.push(Action::SetOpenRecent(open_recent));
                }

                settings_section(ui, "Your data", &palette);
                ui.label(RichText::new("E-Journal database").color(palette.text));
                Frame::new()
                    .fill(palette.field)
                    .stroke(Stroke::new(1.0, palette.stroke))
                    .corner_radius(6)
                    .inner_margin(Margin::symmetric(10, 7))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add(
                            Label::new(
                                RichText::new(self.db_path.display().to_string())
                                    .monospace()
                                    .small()
                                    .color(palette.weak_text),
                            )
                            .wrap(),
                        );
                    });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            Button::new(
                                RichText::new("Export backup").color(palette.weak_text),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        actions.push(Action::Export);
                    }
                    if ui
                        .add(
                            Button::new(RichText::new("Import backup").color(palette.weak_text))
                                .frame(false),
                        )
                        .clicked()
                    {
                        actions.push(Action::Import);
                    }
                });
                ui.add(
                    Label::new(
                        RichText::new(
                            "Entries are stored unencrypted in a local SQLite database. Backups are plain JSON files that you control. This app does not sync your journal to any service.",
                        )
                        .small()
                        .color(palette.text),
                    )
                    .wrap(),
                );

                settings_section(ui, "About", &palette);
                Frame::new()
                    .fill(palette.field)
                    .stroke(Stroke::new(1.0, palette.stroke))
                    .corner_radius(8)
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.label(
                            RichText::new(format!("E-Journal {}", env!("CARGO_PKG_VERSION")))
                                .strong()
                                .color(palette.text),
                        );
                        ui.label(
                            RichText::new(
                                "A local-first journal built with Rust, egui, and SQLite.",
                            )
                            .color(palette.weak_text),
                        );
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "Tested and built on macOS. Runs on macOS, Windows, and Linux.",
                            )
                            .small()
                            .color(palette.weak_text),
                        );
                        ui.label(
                            RichText::new("MIT License")
                                .small()
                                .color(palette.weak_text),
                        );
                    });

                settings_section(ui, "Keyboard shortcuts", &palette);
                let new_shortcut =
                    ui.ctx()
                        .format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::N));
                let save_shortcut =
                    ui.ctx()
                        .format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::S));
                let find_shortcut =
                    ui.ctx()
                        .format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::F));
                ui.label(
                    RichText::new(format!(
                        "{new_shortcut} new entry · {save_shortcut} save · {find_shortcut} search"
                    ))
                    .small()
                    .color(palette.weak_text),
                );
                ui.add_space(12.0);
            });
    }

    fn draw_entry_pane(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        if !self.is_new && self.selected_id.is_none() {
            self.draw_welcome(ui, actions);
            return;
        }

        let palette = self.theme.palette();
        let selected_created_at = self.selected_id.and_then(|id| {
            self.entries
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.created_at)
        });
        let has_selected_entry = selected_created_at.is_some();

        ui.horizontal(|ui| {
            let date = selected_created_at
                .map(format_full_date)
                .unwrap_or_else(|| Local::now().format("%A, %B %d, %Y").to_string());
            ui.label(RichText::new(date).small().color(palette.weak_text));

            if self.is_dirty() {
                ui.label(
                    RichText::new("· Unsaved changes")
                        .small()
                        .color(palette.accent),
                );
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if has_selected_entry
                    && ui
                        .add(
                            Button::new(RichText::new("Delete").small().color(palette.danger))
                                .frame(false),
                        )
                        .clicked()
                {
                    actions.push(Action::RequestDelete);
                }
            });
        });

        ui.add_space(2.0);
        let title = ui.add(
            TextEdit::singleline(&mut self.draft.title)
                .id_salt(("title_editor", self.editor_generation))
                .hint_text("Title")
                .font(FontId::proportional(30.0))
                .desired_width(f32::INFINITY)
                .frame(
                    Frame::new()
                        .fill(palette.field)
                        .corner_radius(8)
                        .inner_margin(Margin::symmetric(12, 8)),
                ),
        );
        if self.focus_title {
            title.request_focus();
            self.focus_title = false;
        }

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("#").color(palette.accent).size(14.0));
            ui.add(
                TextEdit::singleline(&mut self.draft.tags_text)
                    .id_salt(("tags_editor", self.editor_generation))
                    .hint_text("tags, separated by commas")
                    .font(FontId::proportional(13.5))
                    .desired_width(f32::INFINITY)
                    .frame(
                        Frame::new()
                            .fill(palette.field)
                            .corner_radius(6)
                            .inner_margin(Margin::symmetric(8, 5)),
                    ),
            );
        });

        let normalized_tags = normalize_tags(&self.draft.tags_text);
        if !normalized_tags.is_empty() {
            ScrollArea::vertical()
                .id_salt("entry_tag_preview")
                .max_height(54.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for tag in normalized_tags {
                            ui.label(
                                RichText::new(format!("#{tag}"))
                                    .small()
                                    .color(palette.accent),
                            );
                        }
                    });
                });
        }

        ui.add_space(4.0);
        ui.separator();

        let body_height = (ui.available_height() - 64.0).max(150.0);
        ScrollArea::vertical()
            .id_salt("entry_body")
            .max_height(body_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add(
                    TextEdit::multiline(&mut self.draft.body)
                        .id_salt(("body_editor", self.editor_generation))
                        .hint_text("Start writing…")
                        .font(FontId::proportional(16.0))
                        .desired_width(f32::INFINITY)
                        .desired_rows(14)
                        .frame(
                            Frame::new()
                                .fill(palette.field)
                                .corner_radius(10)
                                .inner_margin(Margin::symmetric(14, 12)),
                        ),
                );
            });

        ui.add_space(3.0);
        ui.separator();
        ui.horizontal(|ui| {
            let words = self.draft.body.split_whitespace().count();
            ui.label(
                RichText::new(format!("{words} words"))
                    .small()
                    .color(palette.weak_text),
            );

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let dirty = self.is_dirty();
                let can_revert = dirty || self.has_stale_revision();
                let save = ui.add_enabled(
                    dirty,
                    Button::new(RichText::new("Save").strong().color(if dirty {
                        palette.accent_text
                    } else {
                        palette.weak_text
                    }))
                    .fill(if dirty { palette.accent } else { palette.field })
                    .corner_radius(8)
                    .min_size(egui::vec2(92.0, 32.0)),
                );
                if save.clicked() {
                    actions.push(Action::Save);
                }
                if can_revert
                    && ui
                        .add(
                            Button::new(RichText::new("Revert").small().color(palette.weak_text))
                                .frame(false),
                        )
                        .clicked()
                {
                    actions.push(Action::Revert);
                }
            });
        });
    }

    fn draw_welcome(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let palette = self.theme.palette();
        let new_shortcut = ui
            .ctx()
            .format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::N));

        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.16).min(130.0));
            ui.label(
                RichText::new("A quieter place to think.")
                    .size(30.0)
                    .strong()
                    .color(palette.text),
            );
            ui.add_space(6.0);
            ui.add(
                Label::new(
                    RichText::new(
                        "Write, reflect, and keep every entry on this computer. \
                         No account, no cloud.",
                    )
                    .size(15.0)
                    .color(palette.weak_text),
                )
                .wrap(),
            );
            ui.add_space(22.0);
            if primary_button(ui, "Start writing", &palette, 180.0).clicked() {
                actions.push(Action::NewEntry);
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!("{new_shortcut} starts a new entry"))
                    .small()
                    .color(palette.weak_text),
            );
        });
    }

    fn draw_notice(&mut self, ui: &mut Ui, actions: &mut Vec<Action>) {
        let Some(notice) = &self.notice else {
            return;
        };

        let palette = self.theme.palette();
        let (fill, foreground, heading) = match notice.kind {
            NoticeKind::Info => (palette.hover, palette.text, "Note"),
            NoticeKind::Success => (palette.hover, palette.success, "Saved"),
            NoticeKind::Error => (palette.hover, palette.danger, "Error"),
        };

        let mut dismiss = false;
        Frame::new()
            .fill(fill)
            .stroke(Stroke::new(1.0, palette.stroke))
            .corner_radius(8)
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(heading).strong().color(foreground));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                Button::new(
                                    RichText::new("Dismiss").small().color(palette.weak_text),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            dismiss = true;
                        }
                    });
                });
                ui.add(Label::new(RichText::new(&notice.message).small().color(foreground)).wrap());
            });

        if dismiss {
            actions.push(Action::DismissNotice);
        }
        ui.add_space(8.0);
    }

    fn draw_modals(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(modal) = self.modal.take() else {
            return;
        };

        let palette = self.theme.palette();
        let mut cancel = false;
        let mut modal_action = None;

        let response = egui::Modal::new(Id::unique("journal_modal")).show(ctx, |ui| {
            ui.set_max_width(470.0);

            match &modal {
                Modal::Delete {
                    id,
                    title,
                    revision,
                } => {
                    let deletes_dirty_draft = self.selected_id == Some(*id) && self.is_dirty();
                    ui.heading("Delete this entry?");
                    ui.add_space(4.0);
                    ui.add(
                        Label::new(format!(
                            "“{title}” will be permanently removed from this computer. \
                             This cannot be undone."
                        ))
                        .wrap(),
                    );
                    if deletes_dirty_draft {
                        ui.add_space(8.0);
                        ui.add(
                            Label::new(
                                RichText::new(
                                    "This entry also has unsaved changes in the editor. \
                                     Deleting it will discard that draft.",
                                )
                                .small()
                                .color(palette.danger),
                            )
                            .wrap(),
                        );
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let label = if deletes_dirty_draft {
                                "Discard draft and delete"
                            } else {
                                "Delete entry"
                            };
                            if ui
                                .add(
                                    Button::new(RichText::new(label).color(palette.on_danger))
                                        .fill(palette.danger)
                                        .corner_radius(8),
                                )
                                .clicked()
                            {
                                modal_action = Some(Action::ConfirmDelete(*id, *revision));
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    });
                }
                Modal::UnsavedSwitch { target } => {
                    ui.heading("Save your changes?");
                    ui.add_space(4.0);
                    ui.add(
                        Label::new(
                            "This entry has changes that have not been saved. \
                             Save them before switching to another entry?",
                        )
                        .wrap(),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if primary_button(ui, "Save and continue", &palette, 150.0).clicked() {
                                modal_action = Some(Action::SaveBeforeSwitch(*target));
                            }
                            if ui.button("Discard changes").clicked() {
                                modal_action = Some(Action::DiscardAndSwitch(*target));
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    });
                }
                Modal::UnsavedClose => {
                    ui.heading("Save before closing?");
                    ui.add_space(4.0);
                    ui.add(
                        Label::new(
                            "This entry has changes that have not been saved. \
                             If you close now, those changes will be lost.",
                        )
                        .wrap(),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if primary_button(ui, "Save and close", &palette, 130.0).clicked() {
                                modal_action = Some(Action::SaveBeforeClose);
                            }
                            if ui.button("Discard and close").clicked() {
                                modal_action = Some(Action::DiscardAndClose);
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    });
                }
                Modal::ReloadBeforeDelete => {
                    let draft_dirty = self.is_dirty();
                    ui.heading("Reload before deleting");
                    ui.add_space(4.0);
                    ui.add(
                        Label::new(
                            "This entry changed in another window. Reload the latest version \
                             and review it before deleting.",
                        )
                        .wrap(),
                    );
                    if draft_dirty {
                        ui.add_space(8.0);
                        ui.add(
                            Label::new(
                                RichText::new(
                                    "Your unsaved draft will be discarded when the entry                                      reloads.",
                                )
                                .small()
                                .color(palette.danger),
                            )
                            .wrap(),
                        );
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let label = if draft_dirty {
                                "Discard draft and reload"
                            } else {
                                "Reload entry"
                            };
                            if primary_button(ui, label, &palette, 190.0).clicked() {
                                modal_action = Some(Action::ConfirmRevert);
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    });
                }
                Modal::ConfirmRevert => {
                    let draft_dirty = self.is_dirty();
                    ui.heading(if draft_dirty {
                        "Discard unsaved changes?"
                    } else {
                        "Reload the latest version?"
                    });
                    ui.add_space(4.0);
                    ui.add(
                        Label::new(if draft_dirty {
                            "Reverting will replace the current draft with the last saved \
                             version. This cannot be undone."
                        } else {
                            "This entry changed in another window. Reload the latest saved \
                             version to continue editing it."
                        })
                        .wrap(),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let label = if draft_dirty {
                                "Discard changes"
                            } else {
                                "Reload entry"
                            };
                            if ui
                                .add(
                                    Button::new(RichText::new(label).color(palette.on_danger))
                                        .fill(palette.danger)
                                        .corner_radius(8),
                                )
                                .clicked()
                            {
                                modal_action = Some(Action::ConfirmRevert);
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    });
                }
                Modal::Import {
                    entries,
                    file_name,
                    expected_revisions,
                } => {
                    let current_count = expected_revisions.len();
                    let draft_dirty = self.is_dirty();
                    ui.heading("Replace your journal?");
                    ui.add_space(4.0);
                    ui.add(
                        Label::new(format!(
                            "Importing “{file_name}” will replace your {current_count} current \
                             entries with {} entries from the backup.",
                            entries.len()
                        ))
                        .wrap(),
                    );
                    ui.add_space(4.0);
                    ui.add(
                        Label::new(
                            RichText::new(
                                "Export a backup first if you need to keep the current journal. \
                                 If the import fails, your existing entries stay in place.",
                            )
                            .small()
                            .color(palette.weak_text),
                        )
                        .wrap(),
                    );
                    if draft_dirty {
                        ui.add_space(8.0);
                        ui.add(
                            Label::new(
                                RichText::new(
                                    "You have unsaved changes in the editor. They will be \
                                     discarded if you replace the journal.",
                                )
                                .small()
                                .color(palette.danger),
                            )
                            .wrap(),
                        );
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let label = if draft_dirty {
                                "Discard draft and replace"
                            } else {
                                "Replace journal"
                            };
                            if ui
                                .add(
                                    Button::new(RichText::new(label).color(palette.on_danger))
                                        .fill(palette.danger)
                                        .corner_radius(8),
                                )
                                .clicked()
                            {
                                modal_action = Some(Action::ApplyImport(
                                    entries.clone(),
                                    expected_revisions.clone(),
                                ));
                            }
                            if ui.button("Cancel").clicked() {
                                cancel = true;
                            }
                        });
                    });
                }
            }
        });

        if let Some(action) = modal_action {
            self.modal = None;
            actions.push(action);
        } else if cancel || response.should_close() {
            self.modal = None;
        } else {
            self.modal = Some(modal);
        }
    }
}

impl EJournalApp {
    fn show_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut actions = Vec::new();
        self.handle_shortcuts(&ctx, &mut actions);

        let palette = self.theme.palette();
        egui::Panel::left("journal_sidebar")
            .resizable(true)
            .default_size(SIDEBAR_WIDTH)
            .min_size(280.0)
            .max_size(460.0)
            .frame(
                Frame::new()
                    .fill(palette.sidebar)
                    .inner_margin(Margin::symmetric(16, 16)),
            )
            .show(ui, |ui| {
                self.draw_sidebar(ui, &mut actions);
            });

        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(palette.background)
                    .inner_margin(Margin::symmetric(28, 22)),
            )
            .show(ui, |ui| {
                self.draw_central(ui, &mut actions);
            });

        // Check after the editor has processed this frame's text input so a
        // first edit cannot be lost with the close request. A pending close
        // prompt takes priority over navigation actions queued this frame.
        let close_pending = self.handle_close_request(&ctx);
        if close_pending {
            actions.clear();
        }
        self.draw_modals(&ctx, &mut actions);

        for action in actions {
            self.apply_action(&ctx, action);
        }
    }
}

impl eframe::App for EJournalApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.show_ui(ui);
    }
}

fn configure_context(ctx: &egui::Context, theme: Theme, text_size: TextSize) {
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.window_margin = Margin::same(16);
        style.text_styles = BTreeMap::from([
            (TextStyle::Heading, FontId::proportional(24.0)),
            (TextStyle::Body, FontId::proportional(15.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Small, FontId::proportional(12.0)),
        ]);
        style.visuals.window_corner_radius = CornerRadius::same(12);
        style.visuals.menu_corner_radius = CornerRadius::same(10);
    });

    ctx.set_visuals_of(egui::Theme::Light, Theme::Warm.visuals());
    ctx.set_visuals_of(egui::Theme::Dark, Theme::Dark.visuals());
    apply_theme(ctx, theme);
    ctx.set_zoom_factor(text_size.zoom_factor());
}

fn apply_theme(ctx: &egui::Context, theme: Theme) {
    ctx.set_theme(theme.theme_preference());
    ctx.set_visuals_of(theme.egui_theme(), theme.visuals());
}

fn settings_section(ui: &mut Ui, title: &str, palette: &Palette) {
    ui.add_space(14.0);
    ui.label(RichText::new(title).size(17.0).strong().color(palette.text));
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(2.0);
}

fn sqlite_related_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn normalized_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    match (parent.canonicalize(), path.file_name()) {
        (Ok(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    }
}

fn normalized_paths_match(left: &Path, right: &Path) -> bool {
    let left = normalized_path(left);
    let right = normalized_path(right);

    #[cfg(any(windows, target_os = "macos"))]
    {
        left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        left == right
    }
}

fn export_destination_is_database(path: &Path, db_path: &Path) -> bool {
    ["", "-wal", "-shm", "-journal"].iter().any(|suffix| {
        let candidate = sqlite_related_path(db_path, suffix);
        same_file::is_same_file(path, &candidate)
            .unwrap_or_else(|_| normalized_paths_match(path, &candidate))
    })
}

fn primary_button(ui: &mut Ui, label: &str, palette: &Palette, width: f32) -> Response {
    ui.add_sized(
        [width, 34.0],
        Button::new(RichText::new(label).strong().color(palette.accent_text))
            .fill(palette.accent)
            .corner_radius(8),
    )
}

fn entry_row(ui: &mut Ui, entry: &JournalEntry, selected: bool, palette: &Palette) -> Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 62.0), Sense::click());

    if ui.is_rect_visible(rect) {
        let fill = if selected {
            palette.accent_soft
        } else if response.hovered() {
            palette.hover
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter()
                .rect_filled(rect.shrink2(egui::vec2(2.0, 2.0)), 8, fill);
        }

        let inner = rect.shrink2(egui::vec2(12.0, 8.0));
        let title_font = FontId::proportional(15.0);
        let title = elide_to_width(ui, &entry.title, inner.width(), &title_font);
        ui.painter().text(
            inner.left_top(),
            Align2::LEFT_TOP,
            title,
            title_font,
            palette.text,
        );

        let small_font = FontId::proportional(11.5);
        let date_text = format_list_date(entry.updated_at);
        let date_width_limit = if entry.tags.is_empty() {
            inner.width()
        } else {
            inner.width() * 0.55
        };
        let date = elide_to_width(ui, &date_text, date_width_limit, &small_font);
        let date_width = ui
            .painter()
            .layout_no_wrap(date.clone(), small_font.clone(), Color32::PLACEHOLDER)
            .size()
            .x;
        ui.painter().text(
            inner.left_bottom(),
            Align2::LEFT_BOTTOM,
            date,
            small_font.clone(),
            palette.weak_text,
        );

        if !entry.tags.is_empty() {
            let tags = entry
                .tags
                .iter()
                .take(2)
                .map(|tag| format!("#{tag}"))
                .collect::<Vec<_>>()
                .join("  ");
            let available = (inner.width() - date_width - 8.0).max(0.0);
            if available > 0.0 {
                let tags = elide_to_width(ui, &tags, available, &small_font);
                ui.painter().text(
                    inner.right_bottom(),
                    Align2::RIGHT_BOTTOM,
                    tags,
                    small_font,
                    palette.accent,
                );
            }
        }
    }

    response
}

fn elide_to_width(ui: &Ui, text: &str, max_width: f32, font_id: &FontId) -> String {
    if max_width <= 0.0 {
        return String::new();
    }
    let measure = |candidate: &str| {
        ui.painter()
            .layout_no_wrap(candidate.to_owned(), font_id.clone(), Color32::PLACEHOLDER)
            .size()
            .x
    };
    if measure(text) <= max_width {
        return text.to_owned();
    }

    let characters = text.chars().collect::<Vec<_>>();
    let mut low = 0;
    let mut high = characters.len();
    while low < high {
        let middle = (low + high).div_ceil(2);
        let candidate = format!("{}…", characters[..middle].iter().collect::<String>());
        if measure(&candidate) <= max_width {
            low = middle;
        } else {
            high = middle - 1;
        }
    }

    if low == 0 {
        "…".to_owned()
    } else {
        format!("{}…", characters[..low].iter().collect::<String>())
    }
}

fn format_full_date(timestamp: DateTime<Utc>) -> String {
    timestamp
        .with_timezone(&Local)
        .format("%A, %B %d, %Y at %H:%M")
        .to_string()
}

fn format_list_date(timestamp: DateTime<Utc>) -> String {
    let local = timestamp.with_timezone(&Local);
    let now = Local::now();
    if local.date_naive() == now.date_naive() {
        local.format("Today · %H:%M").to_string()
    } else if local.year() == now.year() {
        local.format("%b %d").to_string()
    } else {
        local.format("%b %d, %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn theme_and_text_size_settings_round_trip() {
        for theme in Theme::ALL {
            assert_eq!(Theme::from_setting(Some(theme.setting_value())), theme);
        }
        assert_eq!(Theme::from_setting(None), Theme::Warm);
        assert_eq!(Theme::from_setting(Some("unknown")), Theme::Warm);
        assert!(Theme::Dark.is_dark());
        assert!(Theme::Slate.is_dark());
        assert!(!Theme::Warm.is_dark());
        assert!(!Theme::Forest.is_dark());

        for text_size in TextSize::ALL {
            assert_eq!(
                TextSize::from_setting(Some(text_size.setting_value())),
                text_size
            );
        }
        assert_eq!(TextSize::from_setting(None), TextSize::Default);
        assert_eq!(TextSize::from_setting(Some("unknown")), TextSize::Default);
    }

    #[test]
    fn stale_editor_revision_is_not_saved() {
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        let base = app.base_entry.as_ref().unwrap().revision;

        std::thread::sleep(std::time::Duration::from_millis(2));
        let external = EntryDraft {
            title: "External edit".into(),
            body: "Changed in another window.".into(),
            tags_text: String::new(),
        }
        .validate()
        .unwrap();
        app.storage.update_entry(id, base, &external).unwrap();

        app.draft.title = "Stale edit".into();
        let error = app.save_current().unwrap_err();

        assert!(error.contains("changed in another window"));
        assert_eq!(
            app.storage.get_entry(id).unwrap().unwrap().title,
            "External edit"
        );
        assert_eq!(app.draft.title, "Stale edit");
        assert_eq!(app.base_entry.as_ref().unwrap().revision, base);
        assert_eq!(app.entries[0].title, "External edit");
    }

    #[test]
    fn saving_after_external_delete_keeps_the_draft_as_new() {
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        app.storage
            .delete_entry(id, app.base_entry.as_ref().unwrap().revision)
            .unwrap();
        app.draft.title = "Keep this draft".into();

        let error = app.save_current().unwrap_err();
        assert!(error.contains("deleted outside E-Journal"));
        assert_eq!(app.selected_id, None);
        assert!(app.is_new);
        assert_eq!(app.draft.title, "Keep this draft");

        app.save_current().unwrap();
        assert_eq!(app.entries.len(), 1);
        assert_eq!(app.entries[0].title, "Keep this draft");
    }

    #[test]
    fn delete_confirmation_rejects_a_changed_entry() {
        let ctx = egui::Context::default();
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        let stale_revision = app.base_entry.as_ref().unwrap().revision;

        let external = EntryDraft {
            title: "External title".into(),
            body: "Changed after confirmation opened.".into(),
            tags_text: String::new(),
        }
        .validate()
        .unwrap();
        app.storage
            .update_entry(id, stale_revision, &external)
            .unwrap();

        app.apply_action(&ctx, Action::RequestDelete);
        let Some(Modal::Delete { revision, .. }) = app.modal.take() else {
            panic!("delete confirmation should be open");
        };
        app.apply_action(&ctx, Action::ConfirmDelete(id, revision));

        assert_eq!(
            app.storage.get_entry(id).unwrap().unwrap().title,
            "External title"
        );
        assert_eq!(app.entries[0].title, "External title");
        assert!(matches!(
            app.notice.as_ref().map(|notice| &notice.kind),
            Some(NoticeKind::Error)
        ));
    }

    #[test]
    fn externally_deleted_entry_keeps_the_draft() {
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        app.draft.body.push_str(" Still editing.");

        app.storage
            .delete_entry(id, app.base_entry.as_ref().unwrap().revision)
            .unwrap();
        app.refresh_data().unwrap();

        assert_eq!(app.selected_id, None);
        assert!(app.is_new);
        assert_eq!(
            app.draft.body,
            "Some body text for the renderer. Still editing."
        );
        assert!(matches!(
            app.notice.as_ref().map(|notice| &notice.kind),
            Some(NoticeKind::Info)
        ));
    }

    #[test]
    fn export_rejects_database_and_sidecar_paths() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("journal.sqlite3");
        std::fs::write(&db_path, "database").unwrap();

        for suffix in ["", "-wal", "-shm", "-journal"] {
            let candidate = sqlite_related_path(&db_path, suffix);
            if !suffix.is_empty() {
                std::fs::write(&candidate, "sidecar").unwrap();
            }
            assert!(export_destination_is_database(&candidate, &db_path));
        }

        assert!(!export_destination_is_database(
            &dir.path().join("backup.json"),
            &db_path
        ));

        #[cfg(any(windows, target_os = "macos"))]
        assert!(export_destination_is_database(
            &sqlite_related_path(&db_path, "-WAL"),
            &db_path
        ));

        #[cfg(unix)]
        {
            let link = dir.path().join("linked-backup.json");
            std::os::unix::fs::symlink(&db_path, &link).unwrap();
            assert!(export_destination_is_database(&link, &db_path));
        }
    }

    #[test]
    fn stale_but_identical_entry_can_be_reloaded() {
        let ctx = egui::Context::default();
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        let original = app.entries[0].clone();

        let same_content = EntryDraft {
            title: original.title.clone(),
            body: original.body.clone(),
            tags_text: original.tags.join(", "),
        }
        .validate()
        .unwrap();
        app.storage
            .update_entry(id, original.revision, &same_content)
            .unwrap();
        app.refresh_data().unwrap();

        assert!(!app.is_dirty());
        assert!(app.has_stale_revision());

        app.apply_action(&ctx, Action::Revert);
        assert!(matches!(app.modal, Some(Modal::ConfirmRevert)));
        app.apply_action(&ctx, Action::ConfirmRevert);
        assert!(!app.has_stale_revision());
    }

    #[test]
    fn opening_an_entry_reads_the_latest_revision() {
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        let base = app.base_entry.as_ref().unwrap().revision;

        let external = EntryDraft {
            title: "Saved elsewhere".into(),
            body: "Newest body.".into(),
            tags_text: String::new(),
        }
        .validate()
        .unwrap();
        let external_entry = app.storage.update_entry(id, base, &external).unwrap();

        app.open_entry(id, external_entry.revision);

        assert_eq!(app.draft.title, "Saved elsewhere");
        assert_eq!(
            app.base_entry.as_ref().unwrap().revision,
            external_entry.revision
        );
        assert_eq!(app.entries[0].title, "Saved elsewhere");
        app.draft.title = "Edited after reload".into();
        app.save_current().unwrap();
        assert_eq!(app.entries[0].title, "Edited after reload");
    }

    #[test]
    fn reselecting_the_current_entry_prompts_for_a_newer_revision() {
        let ctx = egui::Context::default();
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        let base = app.base_entry.as_ref().unwrap().revision;

        let external = EntryDraft {
            title: "External title".into(),
            body: "Changed elsewhere.".into(),
            tags_text: String::new(),
        }
        .validate()
        .unwrap();
        app.storage.update_entry(id, base, &external).unwrap();

        let target = app.current_target().unwrap();
        app.request_switch(target);
        assert!(matches!(app.modal, Some(Modal::ConfirmRevert)));
        assert_eq!(app.draft.title, "A test entry");

        app.apply_action(&ctx, Action::ConfirmRevert);
        assert_eq!(app.draft.title, "External title");
    }

    #[test]
    fn stale_sidebar_selection_does_not_open_a_changed_entry() {
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        let base = app.base_entry.as_ref().unwrap().revision;

        let external = EntryDraft {
            title: "External title".into(),
            body: "Changed elsewhere.".into(),
            tags_text: String::new(),
        }
        .validate()
        .unwrap();
        app.storage.update_entry(id, base, &external).unwrap();
        let stale_revision = app.entries[0].revision;

        app.open_entry(id, stale_revision);

        assert_eq!(app.draft.title, "A test entry");
        assert_eq!(app.entries[0].title, "External title");
        assert!(matches!(
            app.notice.as_ref().map(|notice| &notice.kind),
            Some(NoticeKind::Info)
        ));
    }

    #[test]
    fn external_save_does_not_dirty_an_untouched_draft() {
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        let base = app.base_entry.as_ref().unwrap().revision;

        let external = EntryDraft {
            title: "External title".into(),
            body: "Changed elsewhere.".into(),
            tags_text: String::new(),
        }
        .validate()
        .unwrap();
        app.storage.update_entry(id, base, &external).unwrap();
        app.refresh_data().unwrap();

        assert!(!app.is_dirty());
        assert!(app.has_stale_revision());
    }

    #[test]
    fn revert_reads_the_latest_entry_from_storage() {
        let ctx = egui::Context::default();
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        let base = app.base_entry.as_ref().unwrap().revision;

        let external = EntryDraft {
            title: "External title".into(),
            body: "Changed elsewhere.".into(),
            tags_text: String::new(),
        }
        .validate()
        .unwrap();
        let external_entry = app.storage.update_entry(id, base, &external).unwrap();
        app.draft.title = "My unsaved title".into();

        app.apply_action(&ctx, Action::Revert);
        app.apply_action(&ctx, Action::ConfirmRevert);

        assert_eq!(app.draft.title, "External title");
        assert_eq!(
            app.base_entry.as_ref().unwrap().revision,
            external_entry.revision
        );
        assert_eq!(app.entries[0].title, "External title");
    }

    #[test]
    fn editor_generation_changes_when_the_draft_is_replaced() {
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        let opened = app.editor_generation;
        assert!(opened > 0);

        app.perform_switch(EditorTarget::New);
        assert!(app.editor_generation > opened);
    }

    #[test]
    fn deletion_requires_reloading_a_stale_editor() {
        let ctx = egui::Context::default();
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        let base = app.base_entry.as_ref().unwrap().revision;

        let external = EntryDraft {
            title: "External title".into(),
            body: "Changed elsewhere.".into(),
            tags_text: String::new(),
        }
        .validate()
        .unwrap();
        app.storage.update_entry(id, base, &external).unwrap();
        app.refresh_data().unwrap();

        app.apply_action(&ctx, Action::RequestDelete);
        assert!(matches!(app.modal, Some(Modal::ReloadBeforeDelete)));
        assert!(app.storage.get_entry(id).unwrap().is_some());

        app.apply_action(&ctx, Action::ConfirmRevert);
        assert_eq!(app.draft.title, "External title");
        app.apply_action(&ctx, Action::RequestDelete);
        assert!(matches!(app.modal, Some(Modal::Delete { .. })));
    }

    #[test]
    fn refresh_action_updates_the_sidebar_and_keeps_the_draft() {
        let ctx = egui::Context::default();
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        app.draft.body.push_str(" Draft text.");
        let external = EntryDraft {
            title: "Saved elsewhere".into(),
            body: String::new(),
            tags_text: String::new(),
        }
        .validate()
        .unwrap();
        app.storage.create_entry(&external).unwrap();

        app.apply_action(&ctx, Action::Refresh);

        assert_eq!(app.entries.len(), 2);
        assert!(app.draft.body.contains("Draft text."));
    }

    #[test]
    fn import_conflict_refreshes_the_visible_journal() {
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let mut expected_revisions = app
            .entries
            .iter()
            .map(|entry| (entry.id, entry.revision))
            .collect::<Vec<_>>();
        expected_revisions.sort_unstable_by_key(|(id, _)| *id);

        let external = EntryDraft {
            title: "Saved elsewhere".into(),
            body: String::new(),
            tags_text: String::new(),
        }
        .validate()
        .unwrap();
        app.storage.create_entry(&external).unwrap();

        let now = Utc::now();
        let imported = vec![JournalEntry {
            id: 99,
            revision: 0,
            title: "Imported".into(),
            body: String::new(),
            created_at: now,
            updated_at: now,
            tags: Vec::new(),
        }];
        app.apply_import(imported, expected_revisions);

        assert_eq!(app.entries.len(), 2);
        assert_eq!(app.storage.list_entries().unwrap().len(), 2);
        assert!(matches!(
            app.notice.as_ref().map(|notice| &notice.kind),
            Some(NoticeKind::Error)
        ));
    }

    #[test]
    fn revert_requires_confirmation() {
        let ctx = egui::Context::default();
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        let id = app.entries[0].id;
        app.open_entry_cached(id);
        app.draft.title = "Unsaved title".into();

        app.apply_action(&ctx, Action::Revert);
        assert!(matches!(app.modal, Some(Modal::ConfirmRevert)));
        assert_eq!(app.draft.title, "Unsaved title");

        app.apply_action(&ctx, Action::ConfirmRevert);
        assert!(app.modal.is_none());
        assert_eq!(app.draft.title, "A test entry");
    }

    #[test]
    fn changing_theme_applies_widget_visuals_immediately() {
        let ctx = egui::Context::default();
        let mut app = test_app(Theme::Warm);
        apply_theme(&ctx, Theme::Warm);

        app.set_theme(&ctx, Theme::Slate);

        assert_eq!(ctx.theme(), egui::Theme::Dark);
        assert_eq!(
            ctx.style_of(egui::Theme::Dark).visuals.panel_fill,
            Theme::Slate.visuals().panel_fill
        );
    }

    #[test]
    fn renders_long_sidebar_text_without_panicking() {
        let mut app = test_app(Theme::Warm);
        app.refresh_data().unwrap();
        app.entries[0].title = "A very long title ".repeat(12);
        app.entries[0].tags = vec!["a-very-long-tag-name".repeat(4); 3];
        render(&mut app);
    }

    fn test_app(theme: Theme) -> EJournalApp {
        let mut storage = Storage::open_in_memory().unwrap();
        let validated = EntryDraft {
            title: "A test entry".into(),
            body: "Some body text for the renderer.".into(),
            tags_text: "testing, journal".into(),
        }
        .validate()
        .unwrap();
        storage.create_entry(&validated).unwrap();

        let mut app = EJournalApp {
            storage,
            db_path: PathBuf::from("/tmp/journal-test.sqlite3"),
            entries: Vec::new(),
            visible_indices: Vec::new(),
            tags: Vec::new(),
            selected_id: None,
            base_entry: None,
            is_new: false,
            draft: EntryDraft::default(),
            editor_generation: 0,
            search: String::new(),
            tag_filter: None,
            tab: MainTab::Entry,
            theme,
            text_size: TextSize::Default,
            open_recent: true,
            notice: None,
            modal: None,
            focus_title: false,
            focus_search: false,
            allow_close: false,
        };
        app.refresh_data().unwrap();
        if let Some(id) = app.entries.first().map(|entry| entry.id) {
            app.open_entry_cached(id);
        }
        app
    }

    fn render(app: &mut EJournalApp) {
        let ctx = egui::Context::default();
        configure_context(&ctx, app.theme, app.text_size);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1160.0, 760.0),
            )),
            ..Default::default()
        };

        let mut painted = false;
        for _ in 0..2 {
            let output = ctx.run_ui(input.clone(), |ui| app.show_ui(ui));
            painted |= !output.shapes.is_empty();
            output.drop_without_applying_deltas();
        }
        assert!(painted, "the journal should paint something");
    }

    #[test]
    fn renders_each_theme_with_entries() {
        for theme in [Theme::Warm, Theme::Dark] {
            let mut app = test_app(theme);
            render(&mut app);
        }
    }

    #[test]
    fn renders_welcome_new_entry_and_modal_states() {
        let mut app = test_app(Theme::Warm);

        app.selected_id = None;
        app.is_new = false;
        render(&mut app);

        app.perform_switch(EditorTarget::New);
        render(&mut app);

        app.modal = Some(Modal::Delete {
            id: 1,
            title: "A test entry".into(),
            revision: 1,
        });
        render(&mut app);

        app.modal = Some(Modal::UnsavedClose);
        render(&mut app);

        app.modal = Some(Modal::Import {
            entries: app.entries.clone(),
            file_name: "backup.json".into(),
            expected_revisions: Vec::new(),
        });
        render(&mut app);
    }

    #[test]
    fn renders_settings_with_every_theme() {
        for theme in Theme::ALL {
            let mut app = test_app(theme);
            app.tab = MainTab::Settings;
            render(&mut app);
        }
    }

    #[test]
    fn settings_changes_are_persisted() {
        let mut app = test_app(Theme::Warm);
        let ctx = egui::Context::default();
        configure_context(&ctx, app.theme, app.text_size);

        app.set_theme(&ctx, Theme::Slate);
        app.set_text_size(&ctx, TextSize::Large);
        app.set_open_recent(false);

        assert_eq!(
            app.storage.get_setting(THEME_SETTING).unwrap().as_deref(),
            Some("slate")
        );
        assert_eq!(
            app.storage
                .get_setting(TEXT_SIZE_SETTING)
                .unwrap()
                .as_deref(),
            Some("large")
        );
        assert_eq!(
            app.storage
                .get_setting(OPEN_RECENT_SETTING)
                .unwrap()
                .as_deref(),
            Some("false")
        );
    }
}
