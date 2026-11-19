//! Browser entry point for the WebAssembly build.
//!
//! The desktop build renders into a native window with `eframe`'s native
//! runner. In the browser the same `eframe::App` implementation is driven by
//! `eframe::WebRunner` against a canvas, and file dialogs are replaced with
//! the browser's own download and file-input flows.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::storage::Storage;
use crate::ui::EJournalApp;

/// The canvas element the runner draws into. It is declared in `index.html`.
const CANVAS_ID: &str = "e-journal-canvas";

thread_local! {
    /// Whether the editor holds unsaved changes, read by the unload guard.
    static HAS_UNSAVED_CHANGES: Cell<bool> = const { Cell::new(false) };
}

/// Tell the unload guard whether the editor has unsaved changes.
pub fn set_unsaved_changes(dirty: bool) {
    HAS_UNSAVED_CHANGES.with(|flag| flag.set(dirty));
}

/// Warn before a reload or tab close would discard an open draft.
fn install_unload_guard() {
    let guard = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
        if HAS_UNSAVED_CHANGES.with(Cell::get) {
            // Browsers show their own wording; a return value is what asks.
            event.prevent_default();
            if let Some(event) = event.dyn_ref::<web_sys::BeforeUnloadEvent>() {
                event.set_return_value("");
            }
        }
    });
    if let Some(window) = web_sys::window() {
        let _ =
            window.add_event_listener_with_callback("beforeunload", guard.as_ref().unchecked_ref());
    }
    guard.forget();
}

#[wasm_bindgen(start)]
pub async fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    install_unload_guard();

    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window unavailable"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("document unavailable"))?;
    let canvas = document
        .get_element_by_id(CANVAS_ID)
        .ok_or_else(|| JsValue::from_str("the e-journal-canvas element is missing"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;

    let storage = Storage::open(Path::new("browser-storage"))
        .map_err(|error| JsValue::from_str(&error.to_string()))?;

    let options = eframe::WebOptions {
        // WebGL2 is available in every current browser, while WebGPU support is
        // still uneven and can stall while it negotiates an adapter.
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::WebRunner::new()
        .start(
            canvas,
            options,
            Box::new(move |cc| {
                Ok(Box::new(EJournalApp::new(
                    cc,
                    storage,
                    PathBuf::from("Browser local storage"),
                )))
            }),
        )
        .await
}

/// Offer the browser's own download flow for a text file.
pub fn download_text(file_name: &str, contents: &str) -> Result<(), JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window unavailable"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("document unavailable"))?;

    let parts = js_sys::Array::new();
    parts.push(&JsValue::from_str(contents));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/json");
    let blob =
        web_sys::Blob::new_with_str_sequence_and_options(&parts, &options).map_err(describe)?;

    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(describe)?;

    let anchor = document
        .create_element("a")
        .map_err(describe)?
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| JsValue::from_str("could not create the download link"))?;
    anchor.set_href(&url);
    anchor.set_download(file_name);
    anchor.click();

    // Release the object URL after the download has been handed to the browser.
    let url_for_cleanup = url.clone();
    let cleanup = Closure::once(move || {
        let _ = web_sys::Url::revoke_object_url(&url_for_cleanup);
    });
    let window_for_cleanup =
        web_sys::window().ok_or_else(|| JsValue::from_str("window unavailable"))?;
    window_for_cleanup.set_timeout_with_callback_and_timeout_and_arguments_0(
        cleanup.as_ref().unchecked_ref(),
        10_000,
    )?;
    cleanup.forget();
    Ok(())
}

/// Ask the user for a text file and read its contents.
///
/// Returns `Ok(None)` when the user dismisses the file picker.
pub async fn pick_text_file(accept: &str) -> Result<Option<(String, String)>, JsValue> {
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window unavailable"))?;
    let document = window
        .document()
        .ok_or_else(|| JsValue::from_str("document unavailable"))?;
    let body = document
        .body()
        .ok_or_else(|| JsValue::from_str("document body is unavailable"))?;

    let input = document
        .create_element("input")
        .map_err(describe)?
        .dyn_into::<web_sys::HtmlInputElement>()
        .map_err(|_| JsValue::from_str("could not create the file input"))?;
    input.set_type("file");
    input.set_accept(accept);
    input.style().set_property("display", "none")?;
    body.append_child(&input)?;

    let file = pick_file(&input).await;
    input.remove();

    let Some(file) = file? else {
        return Ok(None);
    };
    let name = file.name();
    let buffer = file.array_buffer().await?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    let text = String::from_utf8(bytes)
        .map_err(|_| JsValue::from_str("the backup file is not valid UTF-8"))?;
    Ok(Some((name, text)))
}

/// Wait for the file input to report a chosen file, or for the picker to be
/// dismissed.
///
/// The browser's file picker is modal, so the page does not receive the change
/// event until the user has chosen a file or dismissed the picker. Browsers
/// announce dismissal with a `cancel` event; if a browser does not send one,
/// the window regaining focus is used as the signal that the picker closed.
async fn pick_file(input: &web_sys::HtmlInputElement) -> Result<Option<web_sys::File>, JsValue> {
    use std::cell::RefCell;
    use std::rc::Rc;

    let result: Rc<RefCell<Option<web_sys::File>>> = Rc::new(RefCell::new(None));
    let finished = Rc::new(RefCell::new(false));

    let settle = {
        let input = input.clone();
        let result = Rc::clone(&result);
        let finished = Rc::clone(&finished);
        move || {
            if let Some(file) = input.files().and_then(|files| files.get(0)) {
                *result.borrow_mut() = Some(file);
            }
            *finished.borrow_mut() = true;
        }
    };

    let on_change = {
        let settle = settle.clone();
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| settle())
    };
    let on_cancel = {
        let settle = settle.clone();
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| settle())
    };
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window unavailable"))?;
    let on_focus = {
        let settle = settle.clone();
        Closure::<dyn FnMut(web_sys::Event)>::new(move |_event: web_sys::Event| {
            // Wait long enough for a change event to arrive first.
            if let Some(window) = web_sys::window() {
                let settle = settle.clone();
                let delay = Closure::once(settle);
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                    delay.as_ref().unchecked_ref(),
                    500,
                );
                delay.forget();
            }
        })
    };

    input.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())?;
    input.add_event_listener_with_callback("cancel", on_cancel.as_ref().unchecked_ref())?;
    window.add_event_listener_with_callback("focus", on_focus.as_ref().unchecked_ref())?;
    input.click();

    while !*finished.borrow() {
        sleep(100).await;
    }

    let _ = input.remove_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
    let _ = input.remove_event_listener_with_callback("cancel", on_cancel.as_ref().unchecked_ref());
    let _ = window.remove_event_listener_with_callback("focus", on_focus.as_ref().unchecked_ref());

    let file = result.borrow_mut().take();
    Ok(file)
}

/// Wait for the given number of milliseconds.
async fn sleep(milliseconds: i32) {
    let window = match web_sys::window() {
        Some(window) => window,
        None => return,
    };
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let resolve_in_timeout = resolve.clone();
        let closure = Closure::once(move || {
            let _ = resolve_in_timeout.call0(&JsValue::undefined());
        });
        if window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                closure.as_ref().unchecked_ref(),
                milliseconds,
            )
            .is_err()
        {
            let _ = resolve.call0(&JsValue::undefined());
            return;
        }
        closure.forget();
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Turn a `JsValue` error into a readable message.
fn describe(error: JsValue) -> JsValue {
    if let Some(message) = error.as_string() {
        return JsValue::from_str(&message);
    }
    let text = js_sys::JSON::stringify(&error)
        .ok()
        .and_then(|value| value.as_string())
        .unwrap_or_else(|| "unknown browser error".to_owned());
    JsValue::from_str(&text)
}
