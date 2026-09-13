//! Saved stock sets — the Pro edition's material library.
//!
//! Named stock configurations (the stock textarea content plus kerf width)
//! persisted to `localStorage`. In the desktop webview this survives restarts
//! via the WebView2 profile; in the browser it survives per-origin.

use serde::{Deserialize, Serialize};

/// A named, saved stock configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSet {
    pub name: String,
    /// Full stock textarea content.
    pub stocks: String,
    /// Kerf width as typed.
    pub kerf: String,
}

const STORAGE_KEY: &str = "cutlist-stock-library";

/// Loads every saved set (empty on any storage error).
pub fn load() -> Vec<SavedSet> {
    let Some(window) = web_sys::window() else {
        return Vec::new();
    };
    let Ok(Some(storage)) = window.local_storage() else {
        return Vec::new();
    };
    storage
        .get_item(STORAGE_KEY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// Persists every saved set (best-effort, silently ignores errors).
pub fn store(sets: &[SavedSet]) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(Some(storage)) = window.local_storage() else {
        return;
    };
    if let Ok(json) = serde_json::to_string(sets) {
        let _ = storage.set_item(STORAGE_KEY, &json);
    }
}
