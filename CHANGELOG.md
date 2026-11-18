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
- Backup files use owner-only permissions on Unix.
- Saves now detect when another app instance changed the entry and keep the
  draft open instead of silently overwriting the newer version.
- A draft stays open as a new entry when its entry was deleted by another app
  instance.
- Imports reject entry IDs that would exhaust SQLite's ID sequence and accept
  valid backups whose timestamps predate a system clock change.
- The import confirmation warns before discarding an unsaved draft.
- Save failures in switch and close confirmations are shown in the UI.
- Exports read a consistent snapshot from the database instead of the sidebar
  cache.
- Tag normalization is idempotent, and reordered tags no longer trigger a false
  unsaved-change prompt.
- Theme changes apply widget visuals immediately.
- Search filters the cached entries, and sidebar rendering no longer scans the
  full entry list for every row.
- Delete and refresh failures are reported alongside the delete result.
- Smoke tests create unique working directories and never delete files supplied
  by the caller.

[Unreleased]: https://github.com/haseeb-heaven/e-journal/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/haseeb-heaven/e-journal/releases/tag/v1.0.0
