//! Project-file round-trip for the IR plugin. The plugin persists the
//! loaded IR's file path alongside its params and — crucially —
//! rebuilds the in-memory file list and kicks the persistent loader
//! thread when the state is restored.

use std::path::Path;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

/// Persisted IR-plugin state. Holds only shared Arcs so the CLAP
/// bridge can call save/load while the plugin has been moved into the
/// audio processor.
///
/// Why the rescan + load_request live here (instead of in `initialize`):
/// the CLAP bridge's load path runs **after** the plugin has been moved
/// into the audio processor and `initialize` has already returned, so by
/// the time the saved path shows up in `ir_path` the loader thread has
/// no file list to walk and process() has no way to kick it. This saver
/// closes that gap by publishing both the path and the matching directory
/// scan as a single synchronous step, then bumping the load-request
/// atomic so the loader thread rebuilds the convolver on its next poll.
pub struct IrExtraState {
    pub ir_path: Arc<Mutex<String>>,
    pub file_list: Arc<Mutex<Vec<String>>>,
    pub load_request: Arc<AtomicI32>,
    /// What the audio thread should adopt from a load: the new directory
    /// index for `file_select` (so Prev/Next and the change detector walk
    /// the new folder), [`ADOPT_CLEAR`] to drop the IR, or [`ADOPT_NONE`].
    pub adopt: Arc<AtomicI32>,
}

/// Nothing to adopt.
pub const ADOPT_NONE: i32 = -1;
/// The loaded state names no IR (`ir_path: ""`): fade the current one out.
pub const ADOPT_CLEAR: i32 = -2;

impl resonance_plugin::plugin::ExtraStateSaver for IrExtraState {
    fn save(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        map.insert(
            "ir_path".to_string(),
            serde_json::Value::String(self.ir_path.lock().clone()),
        );
        map
    }

    /// The impulse response is the sound. Path-only until IRs become a
    /// library kind with content ids (plugin-preset-library.md §9.3).
    fn preset_keys(&self) -> &'static [&'static str] {
        &["ir_path"]
    }

    fn load(&self, state: &serde_json::Value) {
        // No key: keep the current IR (a params-only preset). An empty
        // path: no IR — the current one must stop playing.
        let Some(path) = state.get("ir_path").and_then(|v| v.as_str()) else {
            return;
        };
        *self.ir_path.lock() = path.to_string();
        if path.is_empty() {
            self.file_list.lock().clear();
            self.adopt.store(ADOPT_CLEAR, Ordering::Release);
            return;
        }
        // Rescan the containing directory so Prev/Next in the editor and
        // the audio-thread's param-change detector both have a populated
        // list to work with.
        if let Some(dir) = Path::new(path).parent() {
            let files = resonance_common::scan_directory(dir, "wav");
            let idx = files.iter().position(|f| f == path).unwrap_or(0);
            *self.file_list.lock() = files;
            // Bump the loader thread so it rebuilds the convolver for
            // the restored path. Without this the plugin would sit
            // silent after a project reopen even though `ir_path` was
            // set.
            self.load_request.store(idx as i32, Ordering::Release);
            // And the directory index follows (it is excluded from
            // presets, so nothing else would move it into the new folder).
            self.adopt.store(idx as i32, Ordering::Release);
        }
    }
}

