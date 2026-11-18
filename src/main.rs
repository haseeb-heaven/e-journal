#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod model;
mod smoke;
mod storage;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;

use directories::ProjectDirs;
use thiserror::Error;

use crate::storage::{Storage, StorageError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Gui,
    Version,
    Help,
    SmokeTest,
}

fn main() -> ExitCode {
    #[cfg(all(windows, not(debug_assertions)))]
    attach_parent_console();

    let mode = std::env::args_os()
        .skip(1)
        .map(|argument| {
            argument
                .into_string()
                .map_err(|argument| format!("Unknown argument: {}", argument.to_string_lossy()))
        })
        .collect::<Result<Vec<_>, _>>()
        .and_then(|args| parse_mode(&args));
    let mode = match mode {
        Ok(mode) => mode,
        Err(message) => {
            eprintln!("{message}\n");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    match mode {
        Mode::Version => {
            println!("E-Journal {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Mode::Help => {
            print_usage();
            ExitCode::SUCCESS
        }
        Mode::SmokeTest => match smoke::run() {
            Ok(entries) => {
                println!(
                    "SMOKE_OK entries={entries} version={}",
                    env!("CARGO_PKG_VERSION")
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("SMOKE_FAILED: {error}");
                ExitCode::FAILURE
            }
        },
        Mode::Gui => match run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("E-Journal could not start.\n\n{error}");
                rfd::MessageDialog::new()
                    .set_title("E-Journal could not start")
                    .set_description(error.to_string())
                    .set_level(rfd::MessageLevel::Error)
                    .show();
                ExitCode::FAILURE
            }
        },
    }
}

/// A release build uses the Windows GUI subsystem so launching E-Journal does
/// not open a console window. CLI modes still need a console for their output,
/// so attach to the parent terminal when one exists. Redirected handles are
/// left untouched so automation can still capture stdout and stderr.
#[cfg(all(windows, not(debug_assertions)))]
fn attach_parent_console() {
    use std::ptr;

    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{
        AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE,
        STD_OUTPUT_HANDLE,
    };

    fn missing(handle: HANDLE) -> bool {
        handle.is_null() || handle == INVALID_HANDLE_VALUE
    }

    let stdout_missing = missing(unsafe { GetStdHandle(STD_OUTPUT_HANDLE) });
    let stderr_missing = missing(unsafe { GetStdHandle(STD_ERROR_HANDLE) });
    if !stdout_missing && !stderr_missing {
        return;
    }

    if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } == 0 {
        return;
    }

    let console = || -> HANDLE {
        let name: Vec<u16> = "CONOUT$\0".encode_utf16().collect();
        unsafe {
            CreateFileW(
                name.as_ptr(),
                FILE_GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        }
    };

    if stdout_missing {
        let handle = console();
        if handle != INVALID_HANDLE_VALUE {
            unsafe { SetStdHandle(STD_OUTPUT_HANDLE, handle) };
        }
    }
    if stderr_missing {
        let handle = console();
        if handle != INVALID_HANDLE_VALUE {
            unsafe { SetStdHandle(STD_ERROR_HANDLE, handle) };
        }
    }
}

fn parse_mode(args: &[String]) -> Result<Mode, String> {
    let mut mode = Mode::Gui;

    for arg in args {
        let requested = match arg.as_str() {
            "-h" | "--help" => Mode::Help,
            "-V" | "--version" => Mode::Version,
            "--smoke-test" => Mode::SmokeTest,
            other => return Err(format!("Unknown argument: {other}")),
        };

        if mode != Mode::Gui && mode != requested {
            return Err("Conflicting command-line options.".to_owned());
        }
        mode = requested;
    }

    Ok(mode)
}

fn print_usage() {
    println!(
        "E-Journal {}\n\
         A local-first desktop journal built with Rust, egui, and SQLite.\n\n\
         Usage: e-journal [OPTIONS]\n\n\
         Options:\n\
           -h, --help        Show this help\n\
           -V, --version     Show the version\n\
               --smoke-test  Run the end-to-end smoke test (requires E_JOURNAL_SMOKE_DIR)\n",
        env!("CARGO_PKG_VERSION")
    );
}

fn run() -> Result<(), StartupError> {
    let db_path = database_path()?;
    let storage = Storage::open(&db_path)?;

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("E-Journal")
            .with_app_id("com.HaseebHeaven.E-Journal")
            .with_inner_size([1160.0, 760.0])
            .with_min_inner_size([900.0, 580.0]),
        ..Default::default()
    };

    eframe::run_native(
        "E-Journal",
        native_options,
        Box::new(move |cc| Ok(Box::new(ui::EJournalApp::new(cc, storage, db_path)))),
    )?;

    Ok(())
}

#[derive(Debug, Error)]
enum StartupError {
    #[error(
        "Could not find a per-user application data directory for this system. \
         E-Journal needs a place to store its local database."
    )]
    NoDataDirectory,

    #[error(transparent)]
    Storage(#[from] StorageError),

    #[error("The E-Journal window could not start: {0}")]
    NativeWindow(#[from] eframe::Error),
}

/// `main` owns where the database lives.
fn database_path() -> Result<PathBuf, StartupError> {
    let project_dirs = ProjectDirs::from("com", "HaseebHeaven", "E-Journal")
        .ok_or(StartupError::NoDataDirectory)?;
    Ok(project_dirs.data_dir().join("e-journal.sqlite3"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn command_line_modes_are_parsed() {
        assert_eq!(parse_mode(&[]), Ok(Mode::Gui));
        assert_eq!(parse_mode(&args(&["--version"])), Ok(Mode::Version));
        assert_eq!(parse_mode(&args(&["-V"])), Ok(Mode::Version));
        assert_eq!(parse_mode(&args(&["--help"])), Ok(Mode::Help));
        assert_eq!(parse_mode(&args(&["--smoke-test"])), Ok(Mode::SmokeTest));
        assert!(parse_mode(&args(&["--unknown"])).is_err());
        assert!(parse_mode(&args(&["--help", "--version"])).is_err());
    }

    #[test]
    fn database_path_is_outside_the_project_folder() {
        let path = super::database_path().expect("a data directory should be available");
        assert!(path.ends_with("e-journal.sqlite3"));
        assert!(!path.starts_with(env!("CARGO_MANIFEST_DIR")));
    }
}
