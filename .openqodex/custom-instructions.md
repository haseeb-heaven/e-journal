# Instructions for the OpenQodex reviewer

## What this repo is

E-Journal is a local-first, single-user desktop journal written in Rust with
`eframe`/`egui` and SQLite. It has no network, accounts, telemetry, or cloud
sync by design.

## Conventions the code follows

- Storage errors are typed with `thiserror`; multi-step writes use SQLite
  transactions.
- UI code runs database work on the game/UI thread and surfaces failures in the
  notice area instead of panicking.
- Backups are versioned JSON. Imports validate the complete file before replacing
  data, and failed imports leave the existing journal untouched.
- Entry saves are optimistic: the editor passes the revision it loaded and a
  stale revision must not overwrite a newer save from another app instance.
- Timestamps are RFC3339 UTC strings with fixed-width nanoseconds so SQLite's
  text ordering matches chronological ordering.

## Never flag

- Missing accounts, cloud sync, collaboration, telemetry, or remote backup.
  Those are explicit product non-goals.
- The screenshots in `docs/screenshots/` being binary files.

## Always check

- Unsaved drafts are preserved or explicitly discarded with a warning.
- Destructive operations require confirmation and report partial failures.
- Backup writes cannot truncate an existing backup, and backup imports remain
  transactional.
- Local-only privacy claims in the README still match the implementation.
- When flagging a dependency advisory, distinguish a direct dependency from an
  upstream transitive one and state whether a fixed version exists.
