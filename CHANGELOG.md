# Changelog

All notable changes to E-Journal are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-18

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

- Application and package renamed from Journal to E-Journal. On first launch,
  an existing Journal database is opened in place when the E-Journal database
  does not exist yet; when both exist, the E-Journal database wins.
- Versioned the first release as `0.1.0`.

### Fixed

- Backups are written through a private temporary file and atomically replace
  the destination, so a failed export cannot truncate an existing backup. On
  Unix the destination folder is synced after the rename so a reported success
  survives a power loss. If only that sync fails, the UI says the file was
  written but may not survive a power loss instead of claiming the export
  failed.
- Export reads its snapshot after the destination is chosen, refuses to write
  over the active database or its SQLite sidecar files (including symlinked
  databases and alternate case spellings on case-insensitive volumes), and
  empty journals can still be exported as empty backups.
- Backup files use owner-only permissions on Unix. Imports are parsed as a
  stream, and exports borrow entries directly from the database snapshot and
  serialize straight to the temporary file instead of cloning every body, so
  large valid journals can still be exported and restored without a second full
  copy in memory.
- Saves use database-local revision tokens instead of timestamps, so a stale
  editor cannot overwrite a newer save or a restored entry when the clock moves
  backwards. Read-then-write transactions start with an immediate write lock so
  concurrent windows wait for the configured busy timeout instead of failing
  with an immediate `SQLITE_BUSY`. Database triggers advance revisions for
  writes from an older Journal build that still shares the migrated database.
- Delete confirmations check the revision the user saw, so a replaced entry is
  not removed without a second confirmation, and a stale editor must reload the
  entry before it can be deleted. The dialog warns when deletion or reloading
  will discard an unsaved draft, and describes deletion as removal from the
  active journal rather than from every backup.
- A draft stays open as a new entry when its entry was deleted by another app
  instance.
- Tags are escaped losslessly between the editor and storage, so importing or
  editing an entry with a literal comma in a tag no longer splits it into
  multiple tags.
- Imports remap unsafe high entry IDs to leave SQLite sequence headroom, reset
  the sequence after restore, preserve timestamp tie ordering, reject timestamps
  that cannot round-trip through storage, and accept valid backups whose
  timestamps predate a system clock change.
- Import confirmation records the current entry revisions without parsing entry
  timestamps, refuses to restore if the journal changed before confirmation, and
  still allows recovery from a malformed entry. A conflict refreshes the visible
  journal before another confirmation can be opened.
- The import confirmation warns before discarding an unsaved draft and moves the
  validated entries into the restore action instead of cloning every body.
- Save failures in switch and close confirmations are shown in the UI. A draft
  that conflicts after an external save or import can be kept with **Save as
  new** instead of being discarded by Revert.
- Exports and editor loads read entries and tags from consistent database
  snapshots instead of mixing revisions.
- Save results are read inside the write transaction, before it commits.
- Tag normalization is idempotent, and reordered tags no longer trigger a false
  unsaved-change prompt.
- Theme changes apply widget visuals immediately, and failed settings writes are
  reported and can be retried by selecting the same option again; a successful
  retry clears the stale failure notice. A failed startup-preference write shows
  a retry control instead of requiring two toggles.
- Sidebar rows expose the full entry title and selected state to accessibility
  APIs instead of painting an unlabeled clickable rectangle.
- Search prepares query terms once, skips text folding for empty queries, filters
  cached entries, caches the matching row indices, and the sidebar virtualizes
  fixed-height rows instead of scanning and drawing the whole journal every
  frame.
- Journal refresh derives the tag list from the same entry snapshot instead of
  a second query, and a successful retry replaces a stale refresh error while
  preserving notices from the refresh. The editor avoids cloning the full body
  every frame. Tag previews are height-bounded so many tags cannot push the
  editor controls off screen, and sidebar labels are truncated to their
  measured pixel widths.
- Delete and refresh failures are reported alongside the delete result.
- Editing an entry removes tag names that are no longer used by any entry,
  search uses full Unicode case folding, and tags containing null characters are
  rejected before storage or restore.
- The schema migration assigns revisions in one ordered pass.
- Reverting now requires confirmation before discarding a dirty draft, reloads
  the latest saved entry from SQLite, and remains available when only the stored
  revision changed.
- External saves no longer make an untouched editor look dirty, opening an
  entry reads its latest content and revision from SQLite, and the editor date
  comes from the loaded draft revision instead of a possibly newer sidebar
  cache.
- Sidebar selections carry the revision the user saw, so a replaced entry is
  refreshed instead of opened as a different entry. Reselecting the current row
  also checks SQLite for a newer revision, and a Refresh action reloads the
  sidebar while preserving the draft.
- Editor widgets get a fresh undo history whenever a different draft is loaded.
- First-launch schema creation takes an immediate write transaction, so
  concurrent launches cannot race.
- The journal directory and database use owner-only permissions on Unix.
- The save shortcut only writes when the editor is dirty, and both save and close
  requests are evaluated after the editor processes that frame's text input.
  A pending close prompt takes priority over navigation queued in the same frame.
- Non-Unicode command-line arguments produce a usage error instead of a panic.
- Release builds on Windows attach CLI modes to the parent console while
  preserving redirected output.
- Smoke tests create unique working directories and never delete files supplied
  by the caller.

[Unreleased]: https://github.com/haseeb-heaven/e-journal/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/haseeb-heaven/e-journal/releases/tag/v0.1.0
