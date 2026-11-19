//! Reusable E-Journal modules shared by the native and browser builds.

pub mod backup;
pub mod model;
pub mod transfer;
pub mod ui;

#[cfg(not(target_arch = "wasm32"))]
pub mod smoke;

#[cfg(not(target_arch = "wasm32"))]
pub mod storage;

#[cfg(target_arch = "wasm32")]
#[path = "storage_web.rs"]
pub mod storage;

#[cfg(target_arch = "wasm32")]
mod web;
