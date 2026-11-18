# E-Journal

**A quiet, local-first journal for the desktop.**

[![Version](https://img.shields.io/badge/version-1.0.0-blue.svg)](CHANGELOG.md)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)
![Rust](https://img.shields.io/badge/Rust-2021-orange?logo=rust)
![Platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Windows%20%7C%20Linux-blue)

E-Journal is a private, single-user desktop journal built in Rust with
[egui](https://github.com/emilk/egui) and SQLite. It opens like a notebook:
your entries on the left, a calm writing surface on the right. No account, no
cloud, and no background synchronization.

![E-Journal in the Warm theme](docs/screenshots/journal-warm.png)

## Highlights

- **Focused two-pane layout** — browse, search, and write without leaving the window.
- **Four themes** — Warm, Dark, Slate, and Forest, with a persistent choice.
- **Comfortable settings** — text size, startup behavior, backup controls, and an About panel.
- **Local SQLite storage** — versioned schema, parameterized SQL, and transactional writes.
- **Search and tags** — search titles and body text, then narrow the list by tag.
- **Safe editing** — unsaved-change prompts when switching entries or closing the window.
- **Confirmed deletes** — destructive actions require an explicit confirmation.
- **JSON backup and restore** — validated before import and applied in a single transaction.
- **Visible failures** — save, database, and backup errors are reported in the UI instead of being swallowed.
- **Tested end to end** — unit tests plus process-level E2E tests that run the real binary.

## Screenshots

| Warm theme | Dark theme |
| --- | --- |
| ![Warm theme](docs/screenshots/journal-warm.png) | ![Dark theme](docs/screenshots/journal-dark.png) |

| Settings and About | Forest welcome screen |
| --- | --- |
| ![Settings tab](docs/screenshots/journal-settings.png) | ![Forest welcome screen](docs/screenshots/journal-welcome.png) |

| New entry in the Slate theme |
| --- |
| ![New entry in Slate](docs/screenshots/journal-new-entry.png) |

The screenshots were captured from the running desktop application.

## Versioning and changelog

E-Journal follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
The current release is **1.0.0**; the application version is shown in the
window, in `--version`, and in **Settings → About**.

Release history lives in [CHANGELOG.md](CHANGELOG.md).

## Platform support

E-Journal is a native cross-platform application. The same Rust codebase runs on:

- **macOS**
- **Windows**
- **Linux**

**Verification:** this release was built, tested, and manually exercised on
macOS (Apple Silicon) with Rust stable. The screenshots above were captured
from the running macOS build. Windows and Linux are supported targets through
the same `eframe`/`egui` and bundled-SQLite stack.

### Prerequisites

- A current stable Rust toolchain (`rustup` recommended).
- macOS: Xcode Command Line Tools.
- Windows: the MSVC Rust toolchain and Visual Studio C++ build tools.
- Linux: a working X11 or Wayland session, a GPU driver, and the usual
  windowing development packages (for Debian/Ubuntu: `build-essential`,
  `pkg-config`, `libxkbcommon-dev`, `libwayland-dev`, `libx11-dev`, and
  `libxcb1-dev`).

## Build and run

From the repository root:

```sh
cargo run --release
```

For a debug build:

```sh
cargo build
./target/debug/e-journal
```

The application creates its database automatically on first launch.

## Where your journal lives

E-Journal stores its SQLite database in the operating system's per-user
application data directory:

| Platform | Database path |
| --- | --- |
| macOS | `~/Library/Application Support/com.HaseebHeaven.E-Journal/e-journal.sqlite3` |
| Windows | `%APPDATA%\HaseebHeaven\E-Journal\data\e-journal.sqlite3` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/e-journal/e-journal.sqlite3` |

The exact path is shown in **Settings → Your data**.

## Backups

- **Export** writes a complete, versioned JSON snapshot to a file you choose.
- **Import** validates the entire backup before touching the database. A valid
  backup replaces the current journal inside one SQLite transaction, so a
  failure leaves the existing data intact.
- Backups are ordinary files under your control. This app does not synchronize
  them anywhere.

Journal data is stored **unencrypted** in a local SQLite database, and backups
are plain JSON. Use your operating system's disk encryption and store backups
in a location you trust.

## Settings

Open **Settings** from the sidebar or the tab bar to configure:

- **Theme** — Warm, Dark, Slate, or Forest.
- **Text size** — Small, Default, or Large; applies to the whole interface.
- **Startup** — open the most recent entry automatically, or start on the welcome screen.
- **Your data** — view the database path and run backup/restore.
- **About** — version, platform support, and license.
- **Keyboard shortcuts** — a quick reference.

Preferences are stored alongside the journal and survive restarts.

### Keyboard shortcuts

| Action | Shortcut |
| --- | --- |
| New entry | `Cmd/Ctrl+N` |
| Save entry | `Cmd/Ctrl+S` |
| Focus search | `Cmd/Ctrl+F` |

### Command line

```text
Usage: e-journal [OPTIONS]

  -h, --help        Show help
  -V, --version     Show the application version
      --smoke-test  Run the end-to-end smoke test
```

The smoke test requires a disposable directory:

```sh
E_JOURNAL_SMOKE_DIR=/tmp/e-journal-smoke ./target/release/e-journal --smoke-test
```

It exercises database creation, CRUD, settings, backup export, delete,
transactional restore, and reopening the database without opening a window.

## Architecture

```text
src/
├── main.rs      CLI modes, startup, database location, native eframe lifecycle
├── model.rs     Entry types, validation, tag normalization, search matching
├── storage.rs   SQLite schema/migrations, CRUD, queries, settings
├── backup.rs    Versioned JSON backup validation and transactional restore
├── smoke.rs     Headless end-to-end workflow used by the E2E suite
└── ui.rs        App state, themes, entry list/editor, settings, feedback

tests/
└── cli_e2e.rs   Process-level E2E tests driven by assert_cmd
```

Design boundaries:

- Widgets never contain SQL; all database access goes through `storage`.
- Backup import validates every entry before changing stored data.
- Domain validation lives in `model`, and database writes use parameterized SQL.
- Destructive operations use transactions or explicit confirmation.
- The UI reports storage and backup failures instead of discarding edits silently.

## Validation

Run the full local check suite:

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build
cargo build --release
```

The suite currently includes **64 unit tests** and **7 process-level E2E
tests** (`assert_cmd`) covering:

- entry validation, tag normalization, search, filters, and CLI parsing;
- schema initialization and migration, CRUD, revisions, timestamps, settings,
  permission hardening, and transactional restore;
- backup round-trips, malformed files, unsupported versions, duplicate ids, and
  unsafe restored ids;
- UI rendering in every theme, the welcome screen, new entries, modal states,
  stale saves, delete conflicts, and draft preservation;
- running the real binary for `--version`, `--help`, `--smoke-test`, and the
  complete create/update/export/delete/import/reopen workflow.

Code is reviewed with OpenQodex before release pushes.

## Branches

| Branch | Purpose |
| --- | --- |
| `master` | Stable, release-ready code |
| `develop` | Active integration branch |
| `feature` | Feature work |

## License

Released under the [MIT License](LICENSE).

Copyright © 2026 Haseeb Mir.
