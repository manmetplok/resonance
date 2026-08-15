//! Resonance Gate — a stereo noise gate / downward expander with an
//! external sidechain key.
//!
//! Two jobs in one plugin, separated only by what the host connects to
//! the key port:
//!
//! * **No key connected** — an ordinary noise gate. The detector reads
//!   the signal being gated, and everything below the threshold is
//!   attenuated by up to `range`.
//! * **Key connected** — the detector reads the external signal instead,
//!   which is what makes "open this pad only when the kick hits" (or the
//!   inverse, a trance gate keyed from a rhythm track) possible at all.
//!
//! This is the first plugin in the set to declare
//! [`SIDECHAIN_INPUT`](ResonancePlugin::SIDECHAIN_INPUT). The CLAP
//! machinery for a key port already existed in `resonance-plugin`; nothing
//! had used it, so the host never had a reason to connect one.

use std::sync::Arc;

use resonance_plugin::*;

pub mod dsp;
pub mod params;
pub mod presets;
pub mod viz;

/// Public so the editor's layout table (`editor::GROUPS`) can be
/// checked from `tests/`; the app itself stays crate-private.
#[cfg(feature = "editor")]
pub mod editor;

use dsp::{GateDsp, GateSettings};
use params::{GateParams, PARAM_COUNT};
use viz::GateViz;

pub struct ResonanceGate {
    /// Params shared with the editor via `Arc`; all storage is atomic
    /// internally so `&GateParams` is safe from audio and UI threads.
    pub params: Arc<GateParams>,
    dsp: Option<GateDsp>,
    /// Detector status shared with the editor: which detector is
    /// running, and what it is doing (ba todo #1314).
    viz: Arc<GateViz>,
}

impl ResonanceGate {
    /// Resolve the current parameter values into per-block settings.
    fn settings(&self) -> GateSettings {
        GateSettings {
            threshold_db: self.params.threshold.value(),
            ratio: self.params.ratio.value(),
            attack_ms: self.params.attack.value(),
            hold_ms: self.params.hold.value(),
            release_ms: self.params.release.value(),
            range_db: self.params.range.value(),
            hysteresis_db: self.params.hysteresis.value(),
            key_hpf_hz: self.params.key_hpf.value(),
        }
    }

    /// Whether the last processed block ran off an external key.
    pub fn key_connected(&self) -> bool {
        self.viz.key_connected()
    }

    /// Detector status the editor reads, shared by `Arc` with the audio
    /// thread.
    pub fn viz(&self) -> &Arc<GateViz> {
        &self.viz
    }
}

impl ResonancePlugin for ResonanceGate {
    const CLAP_ID: &'static str = "com.resonance.gate";
    const NAME: &'static str = "Resonance Gate";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str =
        "Stereo noise gate / downward expander with hold, hysteresis and an external sidechain key";
    const FEATURES: &'static [&'static str] = &["audio-effect", "gate", "stereo", "dynamics"];

    const INPUT_CHANNELS: Option<u32> = Some(2);
    const SIDECHAIN_INPUT: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(GateParams::default()),
            dsp: None,
            viz: GateViz::new(),
        }
    }

    fn param_count(&self) -> usize {
        PARAM_COUNT
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.dsp = Some(GateDsp::new(sample_rate));
        true
    }

    fn reset(&mut self) {
        if let Some(dsp) = &mut self.dsp {
            dsp.reset();
        }
        self.viz.clear();
    }

    /// Never called directly by the bridge (it always calls
    /// [`process_with_key`](ResonancePlugin::process_with_key)), but a
    /// host or test that uses the keyless entry point gets the plain
    /// noise-gate behaviour.
    fn process(
        &mut self,
        outputs: &mut [OutputBuffer<'_>],
        frames: usize,
        events: &mut EventIterator<'_>,
        tempo: Option<TempoInfo>,
    ) {
        self.process_with_key(outputs, None, frames, events, tempo);
    }

    fn process_with_key(
        &mut self,
        outputs: &mut [OutputBuffer<'_>],
        key: Option<KeyBuffer<'_>>,
        frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        let settings = self.settings();
        // Which detector is running is known even on the paths that
        // return before the DSP runs, so it is published first.
        self.viz.store_key_connected(key.is_some());

        let Some(main) = outputs.first_mut() else {
            return;
        };
        resonance_common::flush_denormals();

        let Some(dsp) = &mut self.dsp else {
            return;
        };
        dsp.process_block(
            main.left,
            main.right,
            key.map(|k| (k.left, k.right)),
            frames,
            &settings,
        );
        self.viz
            .store_block(dsp.last_state, dsp.last_gr_db, dsp.last_detector_db);
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::GateEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceGate);
