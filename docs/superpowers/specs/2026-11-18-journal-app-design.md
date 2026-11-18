# E-Journal Design

## Goal

Build a clean, single-user native E-Journal application in the isolated `Rust/e-journal` project. The app persists entries locally in SQLite and provides a polished `egui` interface with four themes and a settings tab.

## Project boundaries

- This is a standalone Cargo project under `Rust/e-journal`; it does not modify the parent Rust workspace manifest or its Git configuration.
- Initialize a Git repository inside `Rust/e-journal` only.
- The first release is local-only: no accounts, remote service, or cross-device sync.
- Journal content remains on the user's computer unless explicitly exported.

## Technology choices

- `eframe` provides the native application shell and `egui` UI, following the linked egui project.
- `rusqlite` provides SQLite persistence through Rust Cargo dependencies.
- Additional dependencies should be kept focused on reliable date/time handling, serialization for JSON backups, and native file selection only if needed by the chosen UI flow.

## User experience and features

- A two-pane journal layout: a browsable/searchable entry list and an editor/reader for the selected entry.
- Create, edit, and delete entries. Destructive deletion requires confirmation.
- Entries have a title, body, date/timestamps, and tags.
- Search matches entry titles and body text; tag filtering is available.
- Warm, Dark, Slate, and Forest themes are available, and the chosen theme persists between app launches.
- A settings tab provides appearance (theme and text size), startup behavior, local data controls, and an About section.
- JSON export and import provide a user-controlled backup/restore path. Import validates the file before changing stored data and reports errors without replacing valid existing data.
- Save failures and database errors are surfaced in the UI; the app must not silently discard edits.

## Data and persistence

- Store the SQLite database in the operating system's per-user application data directory, not in the project folder.
- Initialize or migrate the schema at startup and use parameterized SQL statements.
- Persist entry identity, title, body, created/updated timestamps, and tags. Keep the schema simple and versioned so future changes can migrate safely.
- Save explicit user actions reliably. Before switching entries or closing after edits, ensure unsaved content is either saved or the user is warned.
- Treat import/export as backup operations, not as synchronization.

## Architecture

- `model` defines the journal entry and validated editor input types.
- `storage` owns SQLite setup, schema versioning, and entry CRUD/query methods.
- `backup` serializes and validates versioned JSON backups and applies imports transactionally.
- `ui` owns app state, theme selection, entry list/editor rendering, and user feedback. It calls storage and backup interfaces instead of embedding SQL in widgets.
- `main` wires startup, database location, and the native `eframe` application lifecycle.

## Error handling and data safety

- Propagate errors from storage and backup operations and show actionable messages in the UI.
- Database mutations and backup imports use transactions where a partial operation could corrupt user data.
- Validate required fields and backup format at boundaries.
- Do not claim encryption or multi-device backup. Local SQLite content is unencrypted unless separately configured by the operating system; users can export backups to a location they control.

## Validation

- Unit tests cover entry validation, tag normalization, search/filter behavior, theme and text-size settings, and backup serialization/validation.
- Storage tests use temporary SQLite databases and cover schema initialization, CRUD, search, tags, and transactional import behavior.
- Run `cargo fmt --check`, `cargo test`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo build` in `Rust/e-journal`.
- Manually launch the desktop app to verify every theme, the settings tab, entry workflows, persistence after restart, and backup import/export where the environment supports GUI execution.

## Explicit non-goals

- User accounts, cloud storage, synchronization, sharing, encryption/password vault, attachments, rich-text/markdown rendering, and mobile support are outside the first release.
