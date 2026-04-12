/// Resonance Amp - A guitar amp simulator CLAP plugin using NAM models.

use parking_lot::Mutex;
use std::path::Path;
use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use resonance_plugin::*;

mod dsp;
mod loader;
pub mod nam;
pub mod params;
#[cfg(feature = "editor")]
pub mod tone3000;
mod tuner;
pub mod viz;

#[cfg(feature = "editor")]
mod editor;

use dsp::DcBlocker;
use loader::{LoaderDeps, LoaderHandle};
use nam::NamInference;
use params::AmpParams;
use tuner::Tuner;
use viz::AmpViz;

/// Scan a directory for .nam files, returning sorted paths.
fn scan_directory(dir: &Path) -> Vec<String> {
    resonance_common::scan_directory(dir, "nam")
}

/// Crossfade length in samples (~23 ms at 44.1 kHz). Long enough to
/// mask any residual transient when a freshly-loaded model takes over
/// mid-audio, even after the loader thread has primed it.
const SWAP_FADE_SAMPLES: u32 = 1024;
/// Precomputed `1.0 / SWAP_FADE_SAMPLES as f32`. LLVM won't fold
/// float division with a runtime counter, so express the per-sample
/// fade step as a multiply.
const SWAP_FADE_STEP: f32 = 1.0 / SWAP_FADE_SAMPLES as f32;

pub struct ResonanceAmp {
    /// Parameters — shared with the editor thread via `Arc` so the UI can
    /// read and write from a separate thread. The `FloatParam` / `IntParam`
    /// fields use atomic storage internally, so `&AmpParams` is safe to use
    /// concurrently from audio + UI.
    params: Arc<AmpParams>,
    /// Tone3000 API browser worker, lazily created on first editor open.
    /// Held as `Option` because it depends on `Arc<AmpParams>` and is only
    /// useful with the editor feature enabled.
    #[cfg(feature = "editor")]
    tone3000: Option<Arc<tone3000::worker::WorkerHandle>>,
    /// Lock-free meters + scope + transfer curve + tuner state shared
    /// with the editor.
    viz: Arc<AmpViz>,
    /// Monophonic pitch tracker fed with the pre-gain input signal.
    /// `Option` because it depends on the sample rate (known only at
    /// `initialize` time).
    tuner: Option<Tuner>,
    /// Output DC blocker (L/R). Strips any residual DC bias the model
    /// emits at rest — the final layer of the "plop" fix on top of the
    /// loader-thread priming and the extended crossfade.
    dc_l: DcBlocker,
    dc_r: DcBlocker,

    active_model: Option<Box<dyn NamInference>>,
    model_mailbox: Arc<Mutex<Option<Box<dyn NamInference>>>>,
    model_name: Arc<Mutex<String>>,
    /// Last file_select param value we acted on (to detect changes).
    last_file_index: i32,
    /// Atomic load request for the persistent loader thread (-1 = no request).
    load_request: Arc<AtomicI32>,
    /// Handle to the persistent loader thread.
    loader: Option<LoaderHandle>,
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
    /// Scratch buffer used to snapshot the input channel before the
    /// processing loop overwrites it in place. Sized from
    /// `max_buffer_size` in `initialize`.
    input_scratch: Vec<f32>,
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

    /// Synchronously load a model, prime it, sample its transfer curve,
    /// and place it in the mailbox. Used only from `initialize` so the
    /// first model is available before `process` runs.
    fn load_model_sync(&self, path: String) {
        match nam::parse::load_model_from_file(&path) {
            Ok(mut model) => {
                model.reset();
                loader::prime_model(&mut *model, 2048);
                let curve = loader::sample_transfer_curve(&mut *model);
                self.viz.store_transfer_curve(curve);
                model.reset();
                loader::prime_model(&mut *model, 2048);

                let name = Path::new(&path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                *self.model_name.lock() = name;
                *self.model_mailbox.lock() = Some(model);
            }
            Err(e) => {
                eprintln!("Failed to load NAM model: {e}");
                *self.model_name.lock() = format!("Error: {e}");
            }
        }
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
        let params = Arc::new(AmpParams::default());
        let load_request = Arc::new(AtomicI32::new(-1));

        #[cfg(feature = "editor")]
        let tone3000 = {
            let params_for_setter = params.clone();
            let hooks = tone3000::worker::PluginHooks {
                file_list: params.file_list.clone(),
                model_path: params.model_path.clone(),
                load_request: load_request.clone(),
                file_select_setter: Arc::new(move |v| {
                    params_for_setter.file_select.set_value(v);
                }),
            };
            Some(Arc::new(tone3000::worker::spawn(hooks)))
        };

        Self {
            params,
            #[cfg(feature = "editor")]
            tone3000,
            viz: AmpViz::new(),
            tuner: None,
            dc_l: DcBlocker::default(),
            dc_r: DcBlocker::default(),
            active_model: None,
            model_mailbox: Arc::new(Mutex::new(None)),
            model_name: Arc::new(Mutex::new(String::new())),
            last_file_index: -1,
            load_request,
            loader: None,
            pending_model: None,
            fade_out_remaining: 0,
            fade_in_remaining: 0,
            input_gain_smoother: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
            output_gain_smoother: Smoother::new(SmoothingStyle::Logarithmic(50.0)),
            input_scratch: Vec::new(),
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

    fn initialize(&mut self, sample_rate: f32, max_buffer_size: u32) -> bool {
        self.input_gain_smoother.set_sample_rate(sample_rate);
        self.output_gain_smoother.set_sample_rate(sample_rate);
        self.input_gain_smoother.reset(self.params.input_gain.value());
        self.output_gain_smoother.reset(self.params.output_gain.value());

        self.tuner = Some(Tuner::new(sample_rate));
        self.dc_l.reset();
        self.dc_r.reset();
        self.input_scratch = vec![0.0; max_buffer_size as usize];

        let path = self.params.model_path.lock().clone();
        if !path.is_empty() {
            let idx = self.rescan_directory(&path);
            self.last_file_index = idx as i32;
            self.params.file_select.set_value(idx as i32);

            // Block on loading the model during init so the first
            // `process` call has an active model to run.
            self.load_model_sync(path);
            if let Some(model) = self.model_mailbox.lock().take() {
                self.active_model = Some(model);
            }
        }

        // Start the persistent loader thread for runtime file_select
        // changes. All subsequent loads go through it — priming and
        // transfer-curve sampling included.
        self.loader = Some(loader::start(LoaderDeps {
            params: self.params.clone(),
            mailbox: self.model_mailbox.clone(),
            model_name: self.model_name.clone(),
            load_request: self.load_request.clone(),
            viz: self.viz.clone(),
        }));

        true
    }

    fn reset(&mut self) {
        if let Some(model) = &mut self.active_model {
            model.reset();
        }
        self.dc_l.reset();
        self.dc_r.reset();
    }

    fn process(
        &mut self,
        outputs: &mut [resonance_plugin::OutputBuffer<'_>],
        frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        // Single-output effect: operate on port 0 only. The CLAP bridge
        // has already seeded this buffer with the incoming audio.
        let Some(main) = outputs.first_mut() else {
            return;
        };
        let left = &mut *main.left;
        let right = &mut *main.right;
        resonance_common::flush_denormals();

        // Snapshot the dry input before the processing loop overwrites
        // it. Used by the scope view and the tuner.
        let copy_n = frames.min(self.input_scratch.len());
        self.input_scratch[..copy_n].copy_from_slice(&left[..copy_n]);

        // Check mailbox for newly loaded model — start crossfade. The
        // model is already primed on the loader thread, so the fade only
        // has to mask the handoff itself.
        if let Some(mut guard) = self.model_mailbox.try_lock() {
            if guard.is_some() {
                self.pending_model = guard.take();
                if self.active_model.is_some() {
                    self.fade_out_remaining = SWAP_FADE_SAMPLES;
                    self.fade_in_remaining = 0;
                } else {
                    // No previous model — swap directly with fade-in.
                    self.active_model = self.pending_model.take();
                    self.fade_in_remaining = SWAP_FADE_SAMPLES;
                }
            }
        }

        // Detect file_select param change from host/DAW.
        let current_index = self.params.file_select.value();
        if current_index != self.last_file_index {
            self.last_file_index = current_index;
            self.load_request
                .store(current_index, std::sync::atomic::Ordering::Release);
        }

        self.input_gain_smoother
            .set_target(self.params.input_gain.value());
        self.output_gain_smoother
            .set_target(self.params.output_gain.value());

        // Track block-peak values for the meters. Computed inline with
        // the DSP loops below to avoid an extra pass over the buffer.
        let mut in_peak_l = 0.0f32;
        let mut in_peak_r = 0.0f32;
        let mut out_peak_l = 0.0f32;
        let mut out_peak_r = 0.0f32;

        if self.fade_out_remaining == 0 {
            match self.active_model.as_mut() {
                Some(model) => {
                    for i in 0..frames {
                        let dry_l = left[i];
                        let dry_r = right[i];
                        in_peak_l = in_peak_l.max(dry_l.abs());
                        in_peak_r = in_peak_r.max(dry_r.abs());

                        let input_gain = self.input_gain_smoother.next();
                        let output_gain = self.output_gain_smoother.next();
                        let fade_gain = if self.fade_in_remaining > 0 {
                            self.fade_in_remaining -= 1;
                            1.0 - self.fade_in_remaining as f32 * SWAP_FADE_STEP
                        } else {
                            1.0
                        };
                        let input = dry_l * input_gain;
                        let raw = model.process_sample(input) * output_gain * fade_gain;
                        let out_l = self.dc_l.process(raw);
                        let out_r = self.dc_r.process(raw);
                        left[i] = out_l;
                        right[i] = out_r;
                        out_peak_l = out_peak_l.max(out_l.abs());
                        out_peak_r = out_peak_r.max(out_r.abs());
                    }
                }
                None => {
                    // No model loaded: only `input_gain * output_gain` is
                    // applied. `fade_in_remaining` is zero in this arm
                    // because a fade-in is always paired with a newly
                    // installed model.
                    for i in 0..frames {
                        let dry_l = left[i];
                        let dry_r = right[i];
                        in_peak_l = in_peak_l.max(dry_l.abs());
                        in_peak_r = in_peak_r.max(dry_r.abs());

                        let input_gain = self.input_gain_smoother.next();
                        let output_gain = self.output_gain_smoother.next();
                        let gain = input_gain * output_gain;
                        let out_l = dry_l * gain;
                        let out_r = dry_r * gain;
                        left[i] = out_l;
                        right[i] = out_r;
                        out_peak_l = out_peak_l.max(out_l.abs());
                        out_peak_r = out_peak_r.max(out_r.abs());
                    }
                }
            }
        } else {
            // Slow path: fade-out in progress, `active_model` will be
            // replaced mid-block when `fade_out_remaining` hits zero.
            for i in 0..frames {
                let dry_l = left[i];
                let dry_r = right[i];
                in_peak_l = in_peak_l.max(dry_l.abs());
                in_peak_r = in_peak_r.max(dry_r.abs());

                let input_gain = self.input_gain_smoother.next();
                let output_gain = self.output_gain_smoother.next();

                let fade_gain = if self.fade_out_remaining > 0 {
                    self.fade_out_remaining -= 1;
                    let g = self.fade_out_remaining as f32 * SWAP_FADE_STEP;
                    if self.fade_out_remaining == 0 {
                        self.active_model = self.pending_model.take();
                        self.fade_in_remaining = SWAP_FADE_SAMPLES;
                    }
                    g
                } else if self.fade_in_remaining > 0 {
                    self.fade_in_remaining -= 1;
                    1.0 - self.fade_in_remaining as f32 * SWAP_FADE_STEP
                } else {
                    1.0
                };

                let (out_l, out_r) = match &mut self.active_model {
                    Some(model) => {
                        let input = dry_l * input_gain;
                        let raw = model.process_sample(input) * output_gain * fade_gain;
                        (self.dc_l.process(raw), self.dc_r.process(raw))
                    }
                    None => {
                        let gain = input_gain * output_gain * fade_gain;
                        (dry_l * gain, dry_r * gain)
                    }
                };
                left[i] = out_l;
                right[i] = out_r;
                out_peak_l = out_peak_l.max(out_l.abs());
                out_peak_r = out_peak_r.max(out_r.abs());
            }
        }

        // Publish block-rate viz state.
        self.viz.store_peaks(
            linear_to_db(in_peak_l),
            linear_to_db(in_peak_r),
            linear_to_db(out_peak_l),
            linear_to_db(out_peak_r),
        );
        {
            let mut scope = self.viz.scope.lock();
            scope.push_slice(&self.input_scratch[..copy_n], &left[..copy_n]);
        }

        // Feed the tuner with the dry input (pre-gain, pre-model) so
        // the amp's nonlinear harmonics don't confuse the pitch tracker.
        if let Some(tuner) = self.tuner.as_mut() {
            tuner.feed(&self.input_scratch[..copy_n]);
            if let Some((hz, conf)) = tuner.analyze() {
                self.viz.store_tuner(hz, conf);
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
        let tone3000 = self.tone3000.clone()?;
        Some(Arc::new(editor::AmpEditorFactory::new(
            self.params.clone(),
            self.model_name.clone(),
            self.load_request.clone(),
            self.viz.clone(),
            tone3000,
        )))
    }
}

/// Convert a linear amplitude to dBFS. `0.0` → `-inf`, `1.0` → `0 dB`.
fn linear_to_db(linear: f32) -> f32 {
    if linear <= 1e-9 {
        f32::NEG_INFINITY
    } else {
        20.0 * linear.log10()
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

resonance_plugin::export_clap!(ResonanceAmp);
