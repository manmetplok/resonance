//! Resonance Stereo — a mono-safe width tool (warmth-width-depth.md
//! §6.2, slice W7).
//!
//! M/S width, a mono-maker for the bass, a choice of wideners (the
//! mono-sum-preserving velvet decorrelator by default, an all-pass
//! diffuser, a micro-shift doubler, and a Haas mode that is labelled as
//! the mono risk it is), balance and rotation, plus solo-side and
//! mono-check auditions. The editor shows a goniometer and a correlation
//! strip.
//!
//! Every default is transparent and the default plugin is a bit-exact
//! passthrough; the signal path and the latency decision (0 samples in
//! every mode, by design) are documented in [`dsp`].

use std::sync::Arc;

use resonance_plugin::*;

pub mod dsp;
pub mod params;
pub mod presets;
pub mod viz;

/// Public so the editor's layout table (`editor::GROUPS`) can be
/// checked from `tests/`.
#[cfg(feature = "editor")]
pub mod editor;

use dsp::StereoDsp;
use params::{StereoParams, PARAM_COUNT};
use viz::StereoViz;

pub struct ResonanceStereo {
    /// Params shared with the editor via `Arc`; all storage is atomic.
    pub params: Arc<StereoParams>,
    /// Which preset is loaded and whether it has been edited since;
    /// rides along in the saved state.
    presets: Arc<resonance_plugin::presets::PresetSession>,
    viz: Arc<StereoViz>,
    dsp: Option<StereoDsp>,
}

impl ResonanceStereo {
    /// Goniometer / correlation state shared with the editor.
    pub fn viz(&self) -> &Arc<StereoViz> {
        &self.viz
    }
}

impl ResonancePlugin for ResonanceStereo {
    const CLAP_ID: &'static str = "com.resonance.stereo";
    const NAME: &'static str = "Resonance Stereo";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str =
        "Mono-safe stereo width: M/S width, mono-maker, decorrelation widening, \
         balance and rotation";
    const FEATURES: &'static [&'static std::ffi::CStr] = &[
        features::AUDIO_EFFECT,
        features::UTILITY,
        features::MIXING,
        features::STEREO,
    ];

    const FACTORY_PRESETS: &'static [resonance_plugin::presets::FactoryPreset] =
        presets::PRESETS;

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(StereoParams::default()),
            presets: resonance_plugin::presets::PresetSession::new(),
            viz: StereoViz::new(),
            dsp: None,
        }
    }

    fn param_count(&self) -> usize {
        PARAM_COUNT
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.dsp = Some(StereoDsp::new(sample_rate, &self.params));
        true
    }

    fn reset(&mut self) {
        if let Some(dsp) = &mut self.dsp {
            dsp.reset();
        }
        self.viz.clear();
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
        resonance_dsp::flush_denormals();
        let Some(dsp) = &mut self.dsp else {
            return;
        };
        let frames = frames.min(main.left.len()).min(main.right.len());
        dsp.process(
            &mut main.left[..frames],
            &mut main.right[..frames],
            &self.params,
            &self.viz,
        );
    }

    fn extra_state_saver(&self) -> Option<Arc<dyn resonance_plugin::ExtraStateSaver>> {
        Some(self.presets.clone())
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::StereoEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceStereo);
