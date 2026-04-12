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
    params: Arc<EqParams>,
    dsp: Option<EqDsp>,
    /// Per-sample smoother for the output gain knob. Lives on the plugin
    /// struct (not inside the FloatParam) because Smoother::next() needs
    /// &mut self, which would require unsafe reborrowing through the Arc.
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
        self.output_gain_smoother.reset(self.params.output_gain.value());
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

        // Drive the output-gain smoother towards its current target.
        self.output_gain_smoother
            .set_target(self.params.output_gain.value());

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::PARAM_COUNT;
    use crate::presets::PRESETS;

    #[test]
    fn param_enumeration_covers_declared_count() {
        let plugin = ResonanceEq::new();
        assert_eq!(plugin.param_count(), PARAM_COUNT);
        let mut seen = std::collections::HashSet::new();
        for i in 0..plugin.param_count() {
            let id = plugin.param(i).id().to_string();
            assert!(seen.insert(id.clone()), "duplicate param id: {id}");
        }
    }

    #[test]
    fn every_factory_preset_parses_and_loads() {
        assert!(!PRESETS.is_empty());
        for entry in PRESETS {
            let mut plugin = ResonanceEq::new();
            assert!(
                plugin.load_state(entry.json.as_bytes()),
                "preset {:?} failed to load",
                entry.name
            );
        }
    }

    #[test]
    fn state_round_trips_through_save_load() {
        let plugin = ResonanceEq::new();
        // Mutate a handful of params to non-default values.
        plugin.params.bands[2].enabled.set_value(true);
        plugin.params.bands[2].freq.set_value(1234.0);
        plugin.params.bands[2].gain.set_value(-7.5);
        plugin.params.bands[2].q.set_value(2.5);
        plugin.params.bands[2].kind.set_value(0);
        plugin.params.output_gain.set_value(3.25);

        let saved = plugin.save_state();

        let mut other = ResonanceEq::new();
        assert!(other.load_state(&saved));

        let a = &plugin.params.bands[2];
        let b = &other.params.bands[2];
        assert_eq!(a.enabled.value(), b.enabled.value());
        assert!((a.freq.value() - b.freq.value()).abs() < 1e-3);
        assert!((a.gain.value() - b.gain.value()).abs() < 1e-3);
        assert!((a.q.value() - b.q.value()).abs() < 1e-3);
        assert_eq!(a.kind.value(), b.kind.value());
        assert!(
            (plugin.params.output_gain.value() - other.params.output_gain.value()).abs() < 1e-3
        );
    }

    #[test]
    fn dsp_processes_without_nans() {
        let mut plugin = ResonanceEq::new();
        plugin.initialize(48_000.0, 512);
        // Enable a couple of bands and set extreme settings.
        plugin.params.bands[0].enabled.set_value(true);
        plugin.params.bands[0].kind.set_value(3); // low cut
        plugin.params.bands[0].freq.set_value(60.0);
        plugin.params.bands[0].slope.set_value(2); // 48 dB/oct
        plugin.params.bands[3].enabled.set_value(true);
        plugin.params.bands[3].kind.set_value(0); // bell
        plugin.params.bands[3].freq.set_value(2_000.0);
        plugin.params.bands[3].gain.set_value(12.0);
        plugin.params.bands[3].q.set_value(4.0);

        let mut left = vec![0.5_f32; 256];
        let mut right = vec![-0.3_f32; 256];
        let mut outs = [resonance_plugin::OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        let mut ev = resonance_plugin::EventIterator::empty();
        plugin.process(&mut outs, 256, &mut ev, None);

        for &x in left.iter().chain(right.iter()) {
            assert!(x.is_finite(), "output contains non-finite value: {x}");
        }
    }
}
