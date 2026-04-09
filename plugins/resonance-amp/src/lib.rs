/// Resonance Amp - A guitar amp simulator CLAP plugin using NAM models.

use parking_lot::Mutex;
use std::path::Path;
use std::sync::Arc;

use resonance_plugin::*;

pub mod nam;
pub mod params;

#[cfg(feature = "ui")]
pub mod ui;

use nam::NamInference;
use params::AmpParams;

/// Scan a directory for .nam files, returning sorted paths.
fn scan_directory(dir: &Path) -> Vec<String> {
    resonance_common::scan_directory(dir, "nam")
}

pub struct ResonanceAmp {
    params: AmpParams,
    active_model: Option<Box<dyn NamInference>>,
    model_mailbox: Arc<Mutex<Option<Box<dyn NamInference>>>>,
    model_name: Arc<Mutex<String>>,
    /// Last file_select param value we acted on (to detect changes).
    last_file_index: i32,
}

impl ResonanceAmp {
    /// Scan the directory of the given file and update the file list.
    /// Returns the index of `path` in the new list, or 0.
    fn rescan_directory(&self, path: &str) -> usize {
        if let Some(dir) = Path::new(path).parent() {
            let files = scan_directory(dir);
            let idx = files.iter().position(|f| f == path).unwrap_or(0);
            *self.params.file_list.lock() = files;
            idx
        } else {
            0
        }
    }

    /// Load a model by path, blocking the current thread.
    /// On success the model is placed in the mailbox.
    fn load_model_sync(&self, path: String) {
        let mailbox = self.model_mailbox.clone();
        let model_name = self.model_name.clone();

        match nam::parse::load_model_from_file(&path) {
            Ok(model) => {
                let name = Path::new(&path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                *model_name.lock() = name;
                *mailbox.lock() = Some(model);
            }
            Err(e) => {
                eprintln!("Failed to load NAM model: {e}");
                *model_name.lock() = format!("Error: {e}");
            }
        }
    }

    /// Load a model by index from the file list, in a background thread (fire-and-forget).
    fn load_model_by_index_background(&self, idx: usize) {
        let file_list = self.params.file_list.clone();
        let model_path = self.params.model_path.clone();
        let mailbox = self.model_mailbox.clone();
        let model_name = self.model_name.clone();

        std::thread::spawn(move || {
            let path = {
                let list = file_list.lock();
                if list.is_empty() {
                    return;
                }
                let clamped = idx.min(list.len() - 1);
                let p = list[clamped].clone();
                drop(list);
                if let Some(mut mp) = model_path.try_lock() {
                    *mp = p.clone();
                }
                p
            };
            match nam::parse::load_model_from_file(&path) {
                Ok(model) => {
                    let name = Path::new(&path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    *model_name.lock() = name;
                    *mailbox.lock() = Some(model);
                }
                Err(e) => {
                    eprintln!("Failed to load NAM model: {e}");
                    *model_name.lock() = format!("Error: {e}");
                }
            }
        });
    }
}

impl ResonancePlugin for ResonanceAmp {
    const CLAP_ID: &'static str = "com.resonance.amp";
    const NAME: &'static str = "Resonance Amp";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str =
        "Guitar amp simulator using Neural Amp Modeler profiles";
    const FEATURES: &'static [&'static str] = &["audio-effect", "mono", "stereo"];

    const INPUT_CHANNELS: Option<u32> = Some(2);
    const OUTPUT_CHANNELS: u32 = 2;

    fn new() -> Self {
        Self {
            params: AmpParams::default(),
            active_model: None,
            model_mailbox: Arc::new(Mutex::new(None)),
            model_name: Arc::new(Mutex::new(String::new())),
            last_file_index: -1,
        }
    }

    fn params(&self) -> Vec<&dyn Param> {
        vec![
            &self.params.file_select,
            &self.params.input_gain,
            &self.params.output_gain,
        ]
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        // Set smoother sample rates
        self.params.input_gain.smoother.set_sample_rate(sample_rate);
        self.params.output_gain.smoother.set_sample_rate(sample_rate);

        // Initialize smoother targets to current values
        self.params
            .input_gain
            .smoother
            .reset(self.params.input_gain.value());
        self.params
            .output_gain
            .smoother
            .reset(self.params.output_gain.value());

        let path = self.params.model_path.lock().clone();
        if !path.is_empty() {
            let idx = self.rescan_directory(&path);
            self.last_file_index = idx as i32;

            // Block on loading the model during init
            self.load_model_sync(path);
            if let Some(model) = self.model_mailbox.lock().take() {
                self.active_model = Some(model);
            }
        }
        true
    }

    fn reset(&mut self) {
        if let Some(model) = &mut self.active_model {
            model.reset();
        }
    }

    fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        frames: usize,
        _events: &mut EventIterator,
    ) {
        resonance_common::flush_denormals();

        // Check mailbox for newly loaded model
        if let Some(mut guard) = self.model_mailbox.try_lock() {
            if guard.is_some() {
                self.active_model = guard.take();
            }
        }

        // Detect file_select param change from host/DAW
        let current_index = self.params.file_select.value();
        if current_index != self.last_file_index {
            self.last_file_index = current_index;
            // Dispatch to background thread (fire-and-forget)
            self.load_model_by_index_background(current_index as usize);
        }

        // Set smoother targets from current param values
        self.params
            .input_gain
            .smoother
            .set_target(self.params.input_gain.value());
        self.params
            .output_gain
            .smoother
            .set_target(self.params.output_gain.value());

        match &mut self.active_model {
            Some(model) => {
                for i in 0..frames {
                    let input_gain = self.params.input_gain.smoother.next();
                    let output_gain = self.params.output_gain.smoother.next();

                    let input = left[i] * input_gain;
                    let output = model.process_sample(input) * output_gain;

                    // Write same mono output to both channels
                    left[i] = output;
                    right[i] = output;
                }
            }
            None => {
                for i in 0..frames {
                    let input_gain = self.params.input_gain.smoother.next();
                    let output_gain = self.params.output_gain.smoother.next();
                    let gain = input_gain * output_gain;
                    left[i] *= gain;
                    right[i] *= gain;
                }
            }
        }
    }

    fn save_state(&self) -> Vec<u8> {
        let mut json = resonance_plugin::state::params_to_json(&self.params());
        json["model_path"] =
            serde_json::Value::String(self.params.model_path.lock().clone());
        serde_json::to_vec(&json).unwrap_or_default()
    }

    fn load_state(&mut self, data: &[u8]) -> bool {
        if let Ok(state) = serde_json::from_slice::<serde_json::Value>(data) {
            resonance_plugin::state::load_params_from_json(&self.params(), &state);
            if let Some(path) = state.get("model_path").and_then(|v| v.as_str()) {
                *self.params.model_path.lock() = path.to_string();
            }
            true
        } else {
            false
        }
    }
}

#[cfg(not(feature = "ui"))]
resonance_plugin::export_clap!(ResonanceAmp);
