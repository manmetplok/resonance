//! Resonance EQ — an 8-band parametric EQ in the spirit of FabFilter Pro-Q 3.
//!
//! Each band supports bell, low/high shelf, and low/high cut modes with
//! 12/24/48 dB/oct slopes on the cuts. The process loop is a simple
//! per-channel cascade of RBJ cookbook biquads updated once per block.

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
    dsp: Option<EqDsp>,
    /// Per-sample smoother for the output gain knob. Lives on the plugin
    /// struct (not inside the FloatParam) because Smoother::next() needs
    /// &mut self, which would require unsafe reborrowing through the Arc.
    /// Smooths in *linear-gain* space: the dB param value is converted via
    /// `db_to_linear` once when retargeting, so the per-sample path never
    /// pays for a dB→linear conversion.
    output_gain_smoother: Smoother,
    /// Spectrum snapshots published by the audio thread, read by the editor.
    /// Cloned into the editor factory when the host opens the GUI.
    analyzer_state: Arc<AnalyzerState>,
    /// Audio-thread-owned analyzer processors (pre/post FFT, ring buffers,
    /// mono mix scratch). `None` until `initialize` has been called.
    analyzers: Option<StereoAnalyzers>,
}

impl ResonancePlugin for ResonanceEq {
    const CLAP_ID: &'static str = "com.resonance.eq";
    const NAME: &'static str = "Resonance EQ";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str =
        "An 8-band parametric EQ with bell, shelf, and steep cut modes";
    const FEATURES: &'static [&'static str] = &["audio-effect", "equalizer", "stereo"];

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(EqParams::default()),
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

    fn initialize(&mut self, sample_rate: f32, max_buffer_size: u32) -> bool {
        self.output_gain_smoother.set_sample_rate(sample_rate);
        self.output_gain_smoother
            .reset(resonance_dsp::db_to_linear(self.params.output_gain.value()));
        self.dsp = Some(EqDsp::new(sample_rate));
        self.analyzers = Some(StereoAnalyzers::new(sample_rate, max_buffer_size as usize));
        true
    }

    fn reset(&mut self) {
        if let Some(dsp) = &mut self.dsp {
            dsp.clear_state();
        }
        if let Some(an) = &mut self.analyzers {
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
        resonance_common::flush_denormals();

        let Some(dsp) = &mut self.dsp else {
            return;
        };

        // Pre-EQ tap: feed the analyzer with the incoming signal before any
        // processing touches the buffer. Cheap — `feed_pre` only runs an
        // FFT every HOP_SIZE accumulated samples.
        if let Some(an) = &mut self.analyzers {
            an.feed_pre(left, right, &self.analyzer_state);
        }

        // Refresh coefficients once per block from the live parameter values.
        dsp.update_from_params(&self.params);

        // Drive the output-gain smoother towards its current target. The
        // dB→linear conversion happens once here at block rate; the smoother
        // ramps the linear value per sample.
        self.output_gain_smoother
            .set_target(resonance_dsp::db_to_linear(self.params.output_gain.value()));

        dsp.process_stereo(left, right, &mut self.output_gain_smoother);

        // Post-EQ tap: same buffer, now containing the processed signal.
        if let Some(an) = &mut self.analyzers {
            an.feed_post(left, right, &self.analyzer_state);
        }
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::EqEditorFactory::new(
            self.params.clone(),
            self.analyzer_state.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceEq);

