//! Process-level end-to-end tests.
//!
//! These tests build and run the real `e-journal` binary. The smoke-test mode
//! exercises startup, SQLite setup, CRUD, settings, backup export, delete,
//! transactional import, and reopening the database without opening a window.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn smoke_test_runs_the_full_workflow() {
    let temp = tempdir().expect("a temporary smoke directory");

    Command::cargo_bin("e-journal")
        .expect("the e-journal binary")
        .arg("--smoke-test")
        .env("E_JOURNAL_SMOKE_DIR", temp.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("SMOKE_OK entries=1"))
        .stdout(predicate::str::contains(format!(
            "version={}",
            env!("CARGO_PKG_VERSION")
        )));

    let work_dir = temp.path().join("e-journal-smoke");
    assert!(work_dir.join("smoke.sqlite3").is_file());
    assert!(work_dir.join("backup.json").is_file());
}

#[test]
fn version_flag_reports_the_release_version() {
    Command::cargo_bin("e-journal")
        .expect("the e-journal binary")
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "E-Journal {}",
            env!("CARGO_PKG_VERSION")
        )));
}

#[test]
fn help_flag_lists_the_supported_modes() {
    Command::cargo_bin("e-journal")
        .expect("the e-journal binary")
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--smoke-test"))
        .stdout(predicate::str::contains("--version"));
}

#[test]
fn smoke_test_requires_an_explicit_disposable_directory() {
    Command::cargo_bin("e-journal")
        .expect("the e-journal binary")
        .arg("--smoke-test")
        .env_remove("E_JOURNAL_SMOKE_DIR")
        .assert()
        .failure()
        .stderr(predicate::str::contains("E_JOURNAL_SMOKE_DIR"));
}

#[test]
fn unknown_options_are_rejected() {
    Command::cargo_bin("e-journal")
        .expect("the e-journal binary")
        .arg("--not-a-real-option")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown argument"));
}
