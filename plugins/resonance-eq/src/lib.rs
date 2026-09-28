//! Resonance EQ — an 8-band parametric EQ in the spirit of FabFilter Pro-Q 3.
//!
//! Each band supports bell, low/high shelf, and low/high cut modes with
//! 12/24/48 dB/oct slopes on the cuts, plus three one-knob warmth/air
//! kinds — Tilt, LF Lift+Dip and Air — and a per-band Stereo/Mid/Side
//! mode (warmth-width-depth.md §6.4). The process loop is a simple
//! per-channel cascade of biquads updated once per block. An optional
//! auto-gain trims the output by a static loudness estimate of the curve
//! (`dsp::static_gain_db`).

use std::sync::Arc;

use resonance_plugin::*;

pub mod analyzer;
pub mod band;
pub mod dsp;
pub mod params;
pub mod presets;

#[cfg(feature = "editor")]
mod editor;

use analyzer::{AnalyzerState, StereoAnalyzers};
use dsp::EqDsp;
use params::{EqParams, PARAM_COUNT};

pub struct ResonanceEq {
    /// Shared param block — behind Arc so the editor thread can read params
    /// concurrently with the audio thread (all FloatParam/IntParam/BoolParam
    /// values are internally atomic).
    pub params: Arc<EqParams>,
    /// Which preset is loaded and whether it has been edited since.
    /// Shared with the editor thread and handed to the bridge as this
    /// plugin's extra state, so the identity survives closing the window
    /// (ba todo #1358).
    presets: Arc<resonance_plugin::presets::PresetSession>,
    dsp: Option<EqDsp>,
    /// Per-sample smoother for the output gain knob. Lives on the plugin
    /// struct (not inside the FloatParam) because Smoother::next() needs
    /// &mut self, which would require unsafe reborrowing through the Arc.
    /// Smooths in *linear-gain* space: the dB param value is converted via
    /// `db_to_linear` once when retargeting, so the per-sample path never
    /// pays for a dB→linear conversion.
    output_gain_smoother: Smoother,
    /// Spectrum handles published by `initialize`, read by the editor.
    /// Cloned into the editor factory when the host opens the GUI.
    analyzer_state: Arc<AnalyzerState>,
    /// Producer side of the spectrum taps: two SPSC rings the audio thread
    /// pushes into, each drained by its own background FFT worker (see
    /// `analyzer.rs`). `None` until `initialize` has been called; dropping
    /// it joins the workers.
    analyzers: Option<StereoAnalyzers>,
}

impl ResonanceEq {
    /// Shared spectrum handles the editor reads. Public so the crate's
    /// integration tests can observe the published spectra the same way
    /// the editor does.
    pub fn analyzer_state(&self) -> &Arc<AnalyzerState> {
        &self.analyzer_state
    }
}

impl ResonancePlugin for ResonanceEq {
    const CLAP_ID: &'static str = "com.resonance.eq";
    const NAME: &'static str = "Resonance EQ";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str =
        "An 8-band parametric EQ with bell, shelf, and steep cut modes";
    const FEATURES: &'static [&'static std::ffi::CStr] = &[
        features::AUDIO_EFFECT,
        features::EQUALIZER,
        features::STEREO,
    ];

    /// The factory bank, declared once here and read by the editor's
    /// PresetBank and by the exported `resonance_factory_presets`
    /// symbol the host lists over the control API (ba todo #1333).
    const FACTORY_PRESETS: &'static [resonance_plugin::presets::FactoryPreset] =
        presets::PRESETS;

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(EqParams::default()),
            presets: resonance_plugin::presets::PresetSession::new(),
            dsp: None,
            output_gain_smoother: Smoother::new(SmoothingStyle::Logarithmic(20.0)),
            analyzer_state: AnalyzerState::new(),
            analyzers: None,
        }
    }

    fn param_count(&self) -> usize {
        PARAM_COUNT
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.output_gain_smoother.set_sample_rate(sample_rate);
        // Start at the auto-gain trim too, so a project reopened with
        // auto-gain on doesn't ramp into it on the first block.
        let auto_db = if self.params.auto_gain.value() {
            let snaps: [params::BandSnapshot; params::NUM_BANDS] =
                std::array::from_fn(|i| self.params.bands[i].snapshot());
            dsp::auto_gain_trim_db(&snaps, sample_rate)
        } else {
            0.0
        };
        self.output_gain_smoother.reset(resonance_dsp::db_to_linear(
            self.params.output_gain.value() + auto_db,
        ));
        self.dsp = Some(EqDsp::new(sample_rate));
        // Replacing the previous `StereoAnalyzers` (a re-initialize, e.g.
        // after a sample-rate change) drops it, which joins the old worker
        // threads before the new ones spawn.
        self.analyzers = Some(StereoAnalyzers::new(sample_rate, &self.analyzer_state));
        true
    }

    fn reset(&mut self) {
        if let Some(dsp) = &mut self.dsp {
            dsp.clear_state();
        }
        // Audio-thread safe: analyzer reset only sets an atomic clear
        // request; the FFT workers (the ring consumers) service it. The
        // audio thread never writes the rings' consumer index.
        if let Some(an) = &self.analyzers {
            an.reset();
        }
    }

    fn process(
        &mut self,
        outputs: &mut [OutputBuffer<'_>],
        frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        let Some(main) = outputs.first_mut() else {
            return;
        };
        let left = &mut main.left[..frames];
        let right = &mut main.right[..frames];
        resonance_dsp::flush_denormals();

        let Some(dsp) = &mut self.dsp else {
            return;
        };

        // Pre-EQ tap: feed the analyzer with the incoming signal before any
        // processing touches the buffer. Cheap — a mono downmix pushed into
        // a lock-free ring; the FFT runs on a background worker thread.
        if let Some(an) = &self.analyzers {
            an.feed_pre(left, right);
        }

        // Refresh coefficients once per block from the live parameter values.
        dsp.update_from_params(&self.params);

        // Drive the output-gain smoother towards its current target. The
        // dB→linear conversion happens once here at block rate; the smoother
        // ramps the linear value per sample. The auto-gain trim is exactly
        // 0.0 while auto-gain is off, so the target is what it always was.
        self.output_gain_smoother.set_target(resonance_dsp::db_to_linear(
            self.params.output_gain.value() + dsp.auto_gain_db(),
        ));

        dsp.process_stereo(left, right, &mut self.output_gain_smoother);

        // Post-EQ tap: same buffer, now containing the processed signal.
        if let Some(an) = &self.analyzers {
            an.feed_post(left, right);
        }
    }

    /// The loaded-preset identity rides along with the parameter values,
    /// on both bridge paths, so reopening a saved project shows the
    /// preset the sound came from instead of a blank picker.
    fn extra_state_saver(&self) -> Option<Arc<dyn resonance_plugin::ExtraStateSaver>> {
        Some(self.presets.clone())
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::EqEditorFactory::new(
            self.params.clone(),
            self.analyzer_state.clone(),
            self.presets.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceEq);

