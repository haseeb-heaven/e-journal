#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod model;
mod storage;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;

use directories::ProjectDirs;
use thiserror::Error;

use crate::storage::{Storage, StorageError};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Journal could not start.\n\n{error}");
            rfd::MessageDialog::new()
                .set_title("Journal could not start")
                .set_description(error.to_string())
                .set_level(rfd::MessageLevel::Error)
                .show();
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), StartupError> {
    let db_path = database_path()?;
    let storage = Storage::open(&db_path)?;

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Journal")
            .with_app_id("com.JournalApp.Journal")
            .with_inner_size([1160.0, 760.0])
            .with_min_inner_size([900.0, 580.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Journal",
        native_options,
        Box::new(move |cc| Ok(Box::new(ui::JournalApp::new(cc, storage, db_path)))),
    )?;

    Ok(())
}

#[derive(Debug, Error)]
enum StartupError {
    #[error(
        "Could not find a per-user application data directory for this system. \
         The journal needs a place to store its local database."
    )]
    NoDataDirectory,

    #[error(transparent)]
    Storage(#[from] StorageError),

    #[error("The journal window could not start: {0}")]
    NativeWindow(#[from] eframe::Error),
}

/// `main` owns where the database lives.
fn database_path() -> Result<PathBuf, StartupError> {
    let project_dirs =
        ProjectDirs::from("com", "JournalApp", "Journal").ok_or(StartupError::NoDataDirectory)?;
    Ok(project_dirs.data_dir().join("journal.sqlite3"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn database_path_is_outside_the_project_folder() {
        let path = super::database_path().expect("a data directory should be available");
        assert!(path.ends_with("journal.sqlite3"));
        assert!(!path.starts_with(env!("CARGO_MANIFEST_DIR")));
    }
}
