# Changelog

All notable changes to E-Journal are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0] - 2026-11-19

### Added

- Native two-pane journal with a searchable entry list, tag filter, and editor.
- Create, edit, and delete entries with delete confirmation.
- Unsaved-change prompts when switching entries or closing the application.
- SQLite persistence with a versioned schema, parameterized queries, and
  transactional writes.
- Validated JSON export and transactional import for user-controlled backups.
- Four themes: Warm, Dark, Slate, and Forest.
- Settings tab with theme selection, interface text size, startup behavior,
  database location, backup controls, About section, and shortcut reference.
- Persistent preferences for theme, text size, and startup behavior.
- Keyboard shortcuts: new entry, save, and search.
- Unit tests for model validation, storage, backup handling, settings, and UI
  rendering.
- Process-level end-to-end tests driven by `assert_cmd`.
- `--smoke-test` execution mode for CI and end-to-end workflow checks.
- `--version` and `--help` command-line options.
- README, MIT license, and application screenshots.

### Changed

- Application and package renamed from Journal to E-Journal.
- Versioned the first release as `1.0.0`.

### Fixed

- Backups are written through a private temporary file and atomically replace
  the destination, so a failed export cannot truncate an existing backup.
- Export refuses to write over the active database or its SQLite sidecar files.
- Backup files use owner-only permissions on Unix.
- Saves use database-local revision tokens instead of timestamps, so a stale
  editor cannot overwrite a newer save or a restored entry when the clock moves
  backwards.
- Delete confirmations check the revision the user saw, so a replaced entry is
  not removed without a second confirmation, and a stale editor must reload the
  entry before it can be deleted. The dialog warns when deletion or reloading
  will discard an unsaved draft.
- A draft stays open as a new entry when its entry was deleted by another app
  instance.
- Imports remap unsafe high entry IDs to leave SQLite sequence headroom, reset
  the sequence after restore, preserve timestamp tie ordering, reject timestamps
  that cannot round-trip through storage, and accept valid backups whose
  timestamps predate a system clock change.
- Import confirmation records the revisions of the entries currently displayed
  and refuses to restore if the journal changed before confirmation. A conflict
  refreshes the visible journal before another confirmation can be opened.
- The import confirmation warns before discarding an unsaved draft.
- Save failures in switch and close confirmations are shown in the UI.
- Exports and editor loads read entries and tags from consistent database
  snapshots instead of mixing revisions.
- Save results are read inside the write transaction, before it commits.
- Tag normalization is idempotent, and reordered tags no longer trigger a false
  unsaved-change prompt.
- Theme changes apply widget visuals immediately.
- Search filters cached entries, caches the matching row indices, and the
  sidebar virtualizes fixed-height rows instead of scanning and drawing the
  whole journal every frame.
- Delete and refresh failures are reported alongside the delete result.
- Editing an entry removes tag names that are no longer used by any entry, and
  search uses full Unicode case folding.
- The schema migration assigns revisions in one ordered pass.
- Reverting now requires confirmation before discarding a dirty draft, reloads
  the latest saved entry from SQLite, and remains available when only the stored
  revision changed.
- External saves no longer make an untouched editor look dirty, and opening an
  entry reads its latest content and revision from SQLite.
- Editor widgets get a fresh undo history whenever a different draft is loaded.
- First-launch schema creation takes an immediate write transaction, so
  concurrent launches cannot race.
- The journal directory and database use owner-only permissions on Unix.
- The save shortcut only writes when the editor is dirty, and close requests are
  evaluated after the editor processes that frame's text input.
- Non-Unicode command-line arguments produce a usage error instead of a panic.
- Release builds on Windows attach CLI modes to the parent console while
  preserving redirected output.
- Smoke tests create unique working directories and never delete files supplied
  by the caller.

[Unreleased]: https://github.com/haseeb-heaven/e-journal/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/haseeb-heaven/e-journal/releases/tag/v1.0.0
