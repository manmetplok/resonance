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
}

impl resonance_plugin::plugin::ExtraStateSaver for IrExtraState {
    fn save(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        map.insert(
            "ir_path".to_string(),
            serde_json::Value::String(self.ir_path.lock().clone()),
        );
        map
    }

    fn load(&self, state: &serde_json::Value) {
        let Some(path) = state.get("ir_path").and_then(|v| v.as_str()) else {
            return;
        };
        *self.ir_path.lock() = path.to_string();
        if path.is_empty() {
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonance_plugin::plugin::ExtraStateSaver;

    /// Direct round-trip through the `ExtraStateSaver` interface. This
    /// simulates the audio-processor code path in the CLAP bridge: the
    /// owned plugin has been moved into `ClapAudioProcessor`, so
    /// `save()` on the bridge side talks to the cached saver Arc
    /// instead of calling the plugin's trait default.
    #[test]
    fn extra_saver_roundtrip_active_path() {
        let src_path = Arc::new(Mutex::new(
            "/definitely/not/real/active_cab.wav".to_string(),
        ));
        let saver = IrExtraState {
            ir_path: src_path.clone(),
            file_list: Arc::new(Mutex::new(Vec::new())),
            load_request: Arc::new(AtomicI32::new(-1)),
        };

        let mut json = serde_json::json!({ "params": {} });
        for (k, v) in saver.save() {
            json.as_object_mut().unwrap().insert(k, v);
        }

        let dst_path = Arc::new(Mutex::new(String::new()));
        let restored_saver = IrExtraState {
            ir_path: dst_path.clone(),
            file_list: Arc::new(Mutex::new(Vec::new())),
            load_request: Arc::new(AtomicI32::new(-1)),
        };
        restored_saver.load(&json);

        assert_eq!(
            dst_path.lock().clone(),
            "/definitely/not/real/active_cab.wav",
            "ir_path should round-trip through the ExtraStateSaver"
        );
    }

    /// Restoring a project file that references an IR must leave the
    /// plugin in a state where the persistent loader thread will
    /// actually rebuild the convolver. The saver must populate all
    /// three load-side side-effects: `ir_path`, `file_list`, and
    /// `load_request`.
    #[test]
    fn extra_saver_load_populates_file_list_and_queues_loader() {
        let dir = std::env::temp_dir().join(format!(
            "resonance-ir-saver-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let wav_a = dir.join("aaa_first.wav");
        let wav_b = dir.join("bbb_second.wav");
        let wav_c = dir.join("ccc_third.wav");
        for p in [&wav_a, &wav_b, &wav_c] {
            std::fs::write(p, b"").unwrap();
        }
        let target = wav_b.to_string_lossy().into_owned();

        let ir_path = Arc::new(Mutex::new(String::new()));
        let file_list = Arc::new(Mutex::new(Vec::<String>::new()));
        let load_request = Arc::new(AtomicI32::new(-1));

        let saver = IrExtraState {
            ir_path: ir_path.clone(),
            file_list: file_list.clone(),
            load_request: load_request.clone(),
        };

        let state = serde_json::json!({
            "params": {},
            "ir_path": target,
        });
        saver.load(&state);

        assert_eq!(ir_path.lock().clone(), target);
        let files = file_list.lock().clone();
        assert_eq!(files.len(), 3, "all three .wav files should be listed");
        assert!(files.iter().any(|f| f == &target));
        let expected_idx = files.iter().position(|f| f == &target).unwrap() as i32;
        assert_eq!(load_request.load(Ordering::Acquire), expected_idx);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
