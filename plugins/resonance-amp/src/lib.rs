/// Resonance Amp - A guitar amp simulator CLAP plugin using NAM models.

use parking_lot::Mutex;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use resonance_plugin::*;

pub mod nam;
pub mod params;

#[cfg(feature = "editor")]
mod editor;

use nam::NamInference;
use params::AmpParams;

/// Scan a directory for .nam files, returning sorted paths.
fn scan_directory(dir: &Path) -> Vec<String> {
    resonance_common::scan_directory(dir, "nam")
}

/// Crossfade length in samples (~1.5ms at 44.1kHz) to avoid pops on model swap.
const SWAP_FADE_SAMPLES: u32 = 64;

pub struct ResonanceAmp {
    /// Parameters — shared with the editor thread via `Arc` so the UI can
    /// read and write from a separate thread. The `FloatParam` / `IntParam`
    /// fields use atomic storage internally, so `&AmpParams` is safe to use
    /// concurrently from audio + UI.
    params: Arc<AmpParams>,
    active_model: Option<Box<dyn NamInference>>,
    model_mailbox: Arc<Mutex<Option<Box<dyn NamInference>>>>,
    model_name: Arc<Mutex<String>>,
    /// Last file_select param value we acted on (to detect changes).
    last_file_index: i32,
    /// Atomic load request for the persistent loader thread (-1 = no request).
    load_request: Arc<AtomicI32>,
    /// Signal the loader thread to stop.
    loader_stop: Arc<AtomicBool>,
    /// Handle to the persistent loader thread.
    loader_handle: Option<std::thread::JoinHandle<()>>,
    /// Model waiting to be swapped in after fade-out completes.
    pending_model: Option<Box<dyn NamInference>>,
    /// Samples remaining in fade-out before model swap.
    fade_out_remaining: u32,
    /// Samples remaining in fade-in after model swap.
    fade_in_remaining: u32,
    /// Plugin-local smoothers for the two gain params. Live here (not on
    /// `AmpParams`) so that `params` can be `Arc`-shared with the editor
    /// thread while the smoothers stay audio-thread mutable.
    input_gain_smoother: Smoother,
    output_gain_smoother: Smoother,
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

    /// Start the persistent loader thread that polls `load_request`.
    fn start_loader_thread(&mut self) {
        // Stop any existing loader
        self.stop_loader_thread();

        let load_request = self.load_request.clone();
        let stop_flag = self.loader_stop.clone();
        let file_list = self.params.file_list.clone();
        let model_path = self.params.model_path.clone();
        let mailbox = self.model_mailbox.clone();
        let model_name = self.model_name.clone();

        self.loader_handle = Some(
            std::thread::Builder::new()
                .name("amp-loader".into())
                .spawn(move || {
                    while !stop_flag.load(Ordering::Relaxed) {
                        let idx = load_request.swap(-1, Ordering::AcqRel);
                        if idx >= 0 {
                            let path = {
                                let list = file_list.lock();
                                if list.is_empty() {
                                    continue;
                                }
                                let clamped = (idx as usize).min(list.len() - 1);
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
                        } else {
                            std::thread::sleep(std::time::Duration::from_millis(50));
                        }
                    }
                })
                .expect("failed to spawn amp-loader thread"),
        );
    }

    fn stop_loader_thread(&mut self) {
        self.loader_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.loader_handle.take() {
            let _ = handle.join();
        }
        self.loader_stop.store(false, Ordering::Relaxed);
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

    fn new() -> Self {
        Self {
            params: Arc::new(AmpParams::default()),
            active_model: None,
            model_mailbox: Arc::new(Mutex::new(None)),
            model_name: Arc::new(Mutex::new(String::new())),
            last_file_index: -1,
            load_request: Arc::new(AtomicI32::new(-1)),
            loader_stop: Arc::new(AtomicBool::new(false)),
            loader_handle: None,
            pending_model: None,
            fade_out_remaining: 0,
            fade_in_remaining: 0,
            input_gain_smoother: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
            output_gain_smoother: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
        }
    }

    fn param_count(&self) -> usize { 3 }

    fn param(&self, index: usize) -> &dyn Param {
        match index {
            0 => &self.params.file_select,
            1 => &self.params.input_gain,
            2 => &self.params.output_gain,
            _ => unreachable!("invalid param index {index}"),
        }
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        // Configure plugin-local smoothers and seed them with the
        // current parameter values.
        self.input_gain_smoother.set_sample_rate(sample_rate);
        self.output_gain_smoother.set_sample_rate(sample_rate);
        self.input_gain_smoother.reset(self.params.input_gain.value());
        self.output_gain_smoother.reset(self.params.output_gain.value());

        let path = self.params.model_path.lock().clone();
        if !path.is_empty() {
            let idx = self.rescan_directory(&path);
            self.last_file_index = idx as i32;
            self.params.file_select.set_value(idx as i32);

            // Block on loading the model during init
            self.load_model_sync(path);
            if let Some(model) = self.model_mailbox.lock().take() {
                self.active_model = Some(model);
            }
        }

        // Start persistent loader thread for runtime file_select changes
        self.start_loader_thread();

        true
    }

    fn reset(&mut self) {
        if let Some(model) = &mut self.active_model {
            model.reset();
        }
    }

    fn process(
        &mut self,
        outputs: &mut [resonance_plugin::OutputBuffer<'_>],
        frames: usize,
        _events: &mut EventIterator<'_>,
    ) {
        // Single-output effect: operate on port 0 only. The CLAP bridge
        // has already seeded this buffer with the incoming audio.
        let main = outputs
            .first_mut()
            .expect("resonance-amp always has a main output");
        let left = &mut *main.left;
        let right = &mut *main.right;
        resonance_common::flush_denormals();

        // Check mailbox for newly loaded model — start crossfade
        if let Some(mut guard) = self.model_mailbox.try_lock() {
            if guard.is_some() {
                self.pending_model = guard.take();
                if self.active_model.is_some() {
                    self.fade_out_remaining = SWAP_FADE_SAMPLES;
                    self.fade_in_remaining = 0;
                } else {
                    // No previous model — swap directly with fade-in
                    self.active_model = self.pending_model.take();
                    self.fade_in_remaining = SWAP_FADE_SAMPLES;
                }
            }
        }

        // Detect file_select param change from host/DAW
        let current_index = self.params.file_select.value();
        if current_index != self.last_file_index {
            self.last_file_index = current_index;
            // Signal the persistent loader thread (no allocation, no spawn)
            self.load_request.store(current_index, Ordering::Release);
        }

        // Set smoother targets from current param values
        self.input_gain_smoother
            .set_target(self.params.input_gain.value());
        self.output_gain_smoother
            .set_target(self.params.output_gain.value());

        for i in 0..frames {
            let input_gain = self.input_gain_smoother.next();
            let output_gain = self.output_gain_smoother.next();

            // Crossfade envelope: fade out old model, swap, fade in new model
            let fade_gain = if self.fade_out_remaining > 0 {
                self.fade_out_remaining -= 1;
                let g = self.fade_out_remaining as f32 / SWAP_FADE_SAMPLES as f32;
                if self.fade_out_remaining == 0 {
                    self.active_model = self.pending_model.take();
                    self.fade_in_remaining = SWAP_FADE_SAMPLES;
                }
                g
            } else if self.fade_in_remaining > 0 {
                self.fade_in_remaining -= 1;
                1.0 - self.fade_in_remaining as f32 / SWAP_FADE_SAMPLES as f32
            } else {
                1.0
            };

            match &mut self.active_model {
                Some(model) => {
                    let input = left[i] * input_gain;
                    let output = model.process_sample(input) * output_gain * fade_gain;
                    left[i] = output;
                    right[i] = output;
                }
                None => {
                    let gain = input_gain * output_gain * fade_gain;
                    left[i] *= gain;
                    right[i] *= gain;
                }
            }
        }
    }

    fn extra_state_saver(&self) -> Option<Arc<dyn resonance_plugin::plugin::ExtraStateSaver>> {
        Some(Arc::new(AmpExtraState {
            model_path: self.params.model_path.clone(),
        }))
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::AmpEditorFactory::new(
            self.params.clone(),
            self.model_name.clone(),
            self.load_request.clone(),
        )))
    }
}

/// Persists the NAM model path alongside the plugin's params. Holds only
/// the shared `Arc<Mutex<String>>` so the CLAP bridge can serialize it
/// while the plugin is in the audio processor.
struct AmpExtraState {
    model_path: Arc<Mutex<String>>,
}

impl resonance_plugin::plugin::ExtraStateSaver for AmpExtraState {
    fn save(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        map.insert(
            "model_path".to_string(),
            serde_json::Value::String(self.model_path.lock().clone()),
        );
        map
    }

    fn load(&self, state: &serde_json::Value) {
        if let Some(path) = state.get("model_path").and_then(|v| v.as_str()) {
            *self.model_path.lock() = path.to_string();
        }
    }
}

impl Drop for ResonanceAmp {
    fn drop(&mut self) {
        self.stop_loader_thread();
    }
}

resonance_plugin::export_clap!(ResonanceAmp);
