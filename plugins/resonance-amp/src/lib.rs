/// Resonance Amp - A guitar amp simulator CLAP plugin using NAM models.
use parking_lot::Mutex;
use std::sync::atomic::AtomicI32;
use std::sync::Arc;

use resonance_plugin::*;

// `pub` so `benches/amp_dsp.rs` can drive `AmpProcessor::process_block`
// directly — the same entry point `process()` uses, minus the CLAP
// buffer plumbing.
pub mod dsp;
pub mod library_rows;
pub mod library;
mod loader;
pub mod model_ref;
pub mod models;
pub mod nam;
pub mod params;
#[cfg(feature = "editor")]
pub mod tone3000;
pub mod tuner;
pub mod viz;

#[cfg(feature = "editor")]
pub mod editor;

use dsp::AmpProcessor;
use loader::{LoaderDeps, LoaderHandle};
use model_ref::{resolve_model, ModelRef, ModelState, ModelStatus, Resolved};
use nam::NamInference;
use params::AmpParams;
use tuner::Tuner;
use viz::AmpViz;

pub struct ResonanceAmp {
    /// Parameters — shared with the editor thread via `Arc` so the UI can
    /// read and write from a separate thread. The `FloatParam` / `IntParam`
    /// fields use atomic storage internally, so `&AmpParams` is safe to use
    /// concurrently from audio + UI. Also carries the model reference, the
    /// model status and the shared library.
    params: Arc<AmpParams>,
    /// Which preset is loaded and whether it has been edited since,
    /// chained in front of this plugin's own `AmpExtraState` so both ride
    /// along in `save_state` (ba todo #1358).
    presets: Arc<resonance_plugin::presets::PresetSession>,
    /// Lock-free meters + scope + transfer curve + tuner state shared
    /// with the editor.
    viz: Arc<AmpViz>,
    /// Monophonic pitch tracker fed with the pre-gain input signal.
    /// `Option` because it depends on the sample rate (known only at
    /// `initialize` time).
    tuner: Option<Tuner>,
    /// Audio-thread DSP: NAM model fade machinery, DC blockers, gain
    /// smoothers. All state that touches per-sample audio lives here.
    processor: AmpProcessor,

    model_mailbox: Mailbox<Box<dyn NamInference>>,
    /// Last file_select param value we acted on (to detect changes).
    last_file_index: i32,
    /// Atomic load request for the persistent loader thread (-1 = no
    /// request). The value is a library slot.
    load_request: Arc<AtomicI32>,
    /// Handle to the persistent loader thread.
    loader: Option<LoaderHandle>,
    /// Scratch buffer used to snapshot the input channel before the
    /// processing loop overwrites it in place. Sized from
    /// `max_buffer_size` in `initialize`.
    input_scratch: Vec<f32>,
    /// Right-channel twin of `input_scratch`, for the tuner's mono sum.
    input_scratch_r: Vec<f32>,
}

impl ResonanceAmp {
    /// Build an instance over a given shared library. `new()` uses the
    /// process-wide one at the default root; tests pass their own.
    pub fn with_library(library: Arc<library::SharedLibrary>) -> Self {
        let params = Arc::new(AmpParams::with_library(library));
        let load_request = Arc::new(AtomicI32::new(-1));
        // The preset identity wraps the model-reference saver rather than
        // replacing it: chaining is why `with_extra` exists.
        let presets = resonance_plugin::presets::PresetSession::with_extra(Arc::new(
            AmpExtraState {
                model_ref: params.model_ref.clone(),
            },
        ));

        Self {
            params,
            presets,
            viz: AmpViz::new(),
            tuner: None,
            processor: AmpProcessor::new(),
            model_mailbox: Mailbox::new(),
            last_file_index: -1,
            load_request,
            loader: None,
            input_scratch: Vec::new(),
            input_scratch_r: Vec::new(),
        }
    }

    /// What the instance is playing, or why not.
    pub fn model_status(&self) -> ModelStatus {
        self.params.status.lock().clone()
    }

    /// Synchronously load the model at `path`, prime it, sample its
    /// transfer curve, and install it. Used only from `initialize` so the
    /// first model is available before `process` runs.
    fn load_model_sync(&mut self, path: &str) -> Result<(), String> {
        let model = loader::prepare_model(path, &self.viz)?;
        self.processor.install_initial_model(model);
        Ok(())
    }

    /// Resolve the saved model reference against the library and load
    /// what it names (nam-model-library.md §5.2). A reference that does
    /// not resolve is kept verbatim and shown as missing.
    fn restore_model(&mut self) {
        let reference = self.params.model_ref.lock().clone();
        let resolved = {
            let lib = self.params.library.read();
            resolve_model(&reference, &lib, |p| {
                resonance_common::nam_library::hash_file(p).ok()
            })
        };
        let at_slot = self.params.file_select.value();
        self.restore_resolved(resolved, &reference, at_slot);
        let st = self.params.status.lock();
        let playing = (st.state == ModelState::Loaded).then(|| st.id.clone()).flatten();
        drop(st);
        self.params
            .library
            .set_usage(self.params.instance_id, playing.as_deref());
    }

    fn restore_resolved(&mut self, resolved: Resolved, reference: &ModelRef, at_slot: i32) {
        match resolved {
            Resolved::Nothing => {
                *self.params.status.lock() = ModelStatus::default();
            }
            Resolved::Load { path, entry } => {
                let path_str = path.to_string_lossy().into_owned();
                match self.load_model_sync(&path_str) {
                    Ok(()) => {
                        let mut st = self.params.status.lock();
                        *st = ModelStatus {
                            name: entry
                                .as_ref()
                                .map(|e| e.name.clone())
                                .unwrap_or_else(|| reference.display_name()),
                            id: entry.as_ref().map(|e| e.id.clone()).or(reference.id.clone()),
                            state: ModelState::Loaded,
                            external: entry.as_ref().is_none_or(|e| e.path != path),
                            ..ModelStatus::default()
                        };
                        drop(st);
                        if let Some(e) = &entry {
                            // Fill in what a v1 reference lacked; the path
                            // that loaded stays the path.
                            let mut r = ModelRef::from_entry(e);
                            r.path = path_str;
                            *self.params.model_ref.lock() = r;
                            if let Some(slot) = e.slot {
                                self.params.file_select.set_value(slot as i32);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("failed to load NAM model {path_str}: {e}");
                        *self.params.status.lock() = ModelStatus {
                            state: ModelState::Error(e),
                            ..ModelStatus::default()
                        };
                    }
                }
            }
            Resolved::Relinked { entry } => {
                let path_str = entry.path.to_string_lossy().into_owned();
                match self.load_model_sync(&path_str) {
                    Ok(()) => {
                        *self.params.model_ref.lock() = ModelRef::from_entry(&entry);
                        *self.params.status.lock() = ModelStatus {
                            name: entry.name.clone(),
                            id: Some(entry.id.clone()),
                            state: ModelState::Loaded,
                            notice: Some(format!("Relinked: {} (file had moved)", entry.name)),
                            ..ModelStatus::default()
                        };
                        if let Some(slot) = entry.slot {
                            self.params.file_select.set_value(slot as i32);
                        }
                    }
                    Err(e) => {
                        *self.params.status.lock() = ModelStatus {
                            state: ModelState::Error(e),
                            ..ModelStatus::default()
                        };
                    }
                }
            }
            Resolved::Missing {
                name,
                path,
                source,
                file_changed,
            } => {
                *self.params.status.lock() = ModelStatus {
                    state: ModelState::Missing {
                        name,
                        path,
                        source,
                        file_changed,
                        at_slot,
                    },
                    ..ModelStatus::default()
                };
            }
        }
    }
}

impl Drop for ResonanceAmp {
    fn drop(&mut self) {
        // Leave the "used in N open amps" count.
        self.params.library.set_usage(self.params.instance_id, None);
    }
}

impl ResonancePlugin for ResonanceAmp {
    const CLAP_ID: &'static str = "com.resonance.amp";
    const NAME: &'static str = "Resonance Amp";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "Guitar amp simulator using Neural Amp Modeler profiles";
    // `distortion` is CLAP's category for an amp/drive modeller; without
    // a sub-category a host files this under nothing (ba todo #1298).
    const FEATURES: &'static [&'static std::ffi::CStr] = &[
        features::AUDIO_EFFECT,
        features::DISTORTION,
        features::MONO,
        features::STEREO,
    ];

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self::with_library(library::shared())
    }

    fn param_count(&self) -> usize {
        params::PARAM_COUNT
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn initialize(&mut self, sample_rate: f32, max_buffer_size: u32) -> bool {
        self.processor.initialize(
            sample_rate,
            self.params.input_gain.value(),
            self.params.output_gain.value(),
        );

        self.tuner = Some(Tuner::new(sample_rate));
        self.input_scratch = vec![0.0; max_buffer_size as usize];
        self.input_scratch_r = vec![0.0; max_buffer_size as usize];
        self.viz.store_engine_sample_rate(sample_rate);

        // Bring the shared index up to date (a full scan once per process,
        // a `stat` of library.json after that), then resolve the saved
        // reference and block on loading it, so the first `process` call
        // has an active model to run. A reference that resolves into the
        // library also re-derives `file_select` from it: in a project the
        // reference wins over a stale slot value.
        self.params.library.ensure_scanned();
        self.restore_model();

        // Baseline the change detector against what the selector ACTUALLY
        // holds, on every activation and whether or not a model was
        // restored above.
        //
        // `process()` reads any `file_select != last_file_index` as "the
        // user picked a new model" and loads that slot. Leaving the
        // baseline at its `new()` value of -1 would make the FIRST
        // `process()` call fire a load request for slot 0 regardless of
        // what is loaded.
        //
        // That first call is not where you would look for it either. The
        // mixer skips the arrangement render while the transport is
        // stopped, so an effect on an audio track does not process at all
        // until playback starts or the track is monitored/record-armed:
        // the spurious load surfaces as "arming the track swapped my amp
        // model", one user action removed from the activation that
        // actually queued it.
        self.last_file_index = self.params.file_select.value();

        // Start the persistent loader thread for runtime file_select
        // changes. All subsequent loads go through it — priming and
        // transfer-curve sampling included.
        self.loader = Some(loader::start(LoaderDeps {
            params: self.params.clone(),
            mailbox: self.model_mailbox.clone(),
            load_request: self.load_request.clone(),
            viz: self.viz.clone(),
        }));

        true
    }

    fn reset(&mut self) {
        self.processor.reset();
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
        resonance_dsp::flush_denormals();

        // Snapshot the dry input before the processing loop overwrites
        // it. Used by the scope view and the tuner.
        let copy_n = frames.min(self.input_scratch.len());
        self.input_scratch[..copy_n].copy_from_slice(&left[..copy_n]);
        let copy_r = copy_n.min(self.input_scratch_r.len()).min(right.len());
        self.input_scratch_r[..copy_r].copy_from_slice(&right[..copy_r]);

        // Check mailbox for newly loaded model — start crossfade. The
        // model is already primed on the loader thread, so the fade only
        // has to mask the handoff itself.
        if let Some(model) = self.model_mailbox.try_take() {
            self.processor.install_pending_model(model);
        }

        // Detect file_select param change from host/DAW.
        let current_index = self.params.file_select.value();
        if current_index != self.last_file_index {
            self.last_file_index = current_index;
            self.load_request
                .store(current_index, std::sync::atomic::Ordering::Release);
        }

        self.processor.set_gain_targets(
            self.params.input_gain.value(),
            self.params.output_gain.value(),
        );

        let peaks = self.processor.process_block(left, right, frames);

        // Publish block-rate viz state.
        self.viz.store_peaks(
            linear_to_db(peaks.in_l),
            linear_to_db(peaks.in_r),
            linear_to_db(peaks.out_l),
            linear_to_db(peaks.out_r),
        );
        // Lock-free: `scope` is a pair of `AtomicHistoryRing`s, so the
        // audio thread never blocks on (or skips a push for) the editor's
        // per-frame `iter_chrono` scan.
        self.viz
            .scope
            .push_slice(&self.input_scratch[..copy_n], &left[..copy_n]);

        // Feed the tuner with the dry input (pre-gain, pre-model) so
        // the amp's nonlinear harmonics don't confuse the pitch tracker —
        // as the mono sum, so a guitar on either input tunes (DSP-11).
        if let Some(tuner) = self.tuner.as_mut() {
            tuner.feed_stereo(&self.input_scratch[..copy_r], &self.input_scratch_r[..copy_r]);
            if let Some((hz, conf)) = tuner.analyze() {
                self.viz.store_tuner(hz, conf);
            }
        }
    }


    fn extra_state_saver(&self) -> Option<Arc<dyn resonance_plugin::plugin::ExtraStateSaver>> {
        Some(self.presets.clone())
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::AmpEditorFactory::new(
            self.params.clone(),
            self.load_request.clone(),
            self.viz.clone(),
            self.presets.clone(),
        )))
    }
}

use resonance_dsp::linear_to_db;

/// Persists the model reference (state v2: `model_path` + optional
/// `model_id` / `model_name` / `model_source`) alongside the plugin's
/// params. Holds only the shared `Arc<Mutex<ModelRef>>` so the CLAP bridge
/// can serialize it while the plugin is in the audio processor.
struct AmpExtraState {
    model_ref: Arc<Mutex<ModelRef>>,
}

impl resonance_plugin::plugin::ExtraStateSaver for AmpExtraState {
    fn save(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut map = serde_json::Map::new();
        self.model_ref.lock().save_into(&mut map);
        map
    }

    fn load(&self, state: &serde_json::Value) {
        // A v1 document carries only `model_path`; the other keys are
        // optional. A document with no `model_path` at all leaves the
        // reference alone, as before v2.
        if let Some(reference) = ModelRef::load_from(state) {
            *self.model_ref.lock() = reference;
        }
    }
}

resonance_plugin::export_clap!(ResonanceAmp);
