//! Resonance Mastering — automatic mastering plugin for band music.
//!
//! A fixed-order mastering chain ([`chain`]): input trim → corrective
//! linear-phase EQ → glue compressor → saturator → tonal linear-phase EQ
//! → linear-phase multiband compressor → M/S imager → clipper →
//! true-peak limiter → dither, every stage off by default. The EQ bands
//! each filter the stereo pair, the mid or the side; the imager has a
//! global and a per-band width (on the multiband's crossover); the
//! saturator has the original Tube↔Tape blend plus the character modes
//! of [`stages::sat_modes`]. The latency is constant: the linear-phase
//! stages and the limiter lookahead, whatever is switched on.
//!
//! Alongside it: a metering tap built on `resonance-metering` (LUFS,
//! true peak, correlation, spectrum; [`dsp`]) and the one-shot master
//! assistant ([`assistant`]), which suggests settings from stored genre
//! target curves or a user-loaded reference track.

use std::sync::Arc;

use resonance_plugin::*;

pub mod assistant;
pub mod chain;
pub mod dsp;
pub mod params;
pub mod stages;
pub mod viz;

#[cfg(feature = "editor")]
pub mod editor;

use chain::Chain;
use params::MasteringParams;
use viz::MasteringViz;

pub use params::PARAM_COUNT;

pub struct ResonanceMastering {
    params: Arc<MasteringParams>,
    viz: Arc<MasteringViz>,
    /// Which preset is loaded and whether it has been edited since.
    /// Shared with the editor thread and handed to the bridge as this
    /// plugin's extra state, so the identity survives closing the window
    /// (ba todo #1358).
    presets: Arc<resonance_plugin::presets::PresetSession>,
    chain: Option<Chain>,
}

impl ResonanceMastering {
    /// Test helper: direct access to the parameter struct.
    pub fn params(&self) -> &MasteringParams {
        &self.params
    }

    /// Test helper: direct access to the shared viz state (and its
    /// embedded assistant).
    pub fn viz(&self) -> &MasteringViz {
        &self.viz
    }
}

impl ResonancePlugin for ResonanceMastering {
    const CLAP_ID: &'static str = "com.resonance.mastering";
    const NAME: &'static str = "Resonance Mastering";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str =
        "Automatic mastering plugin — analyzer + linear-phase mastering chain";
    // `mastering` and `analyzer` are both standard CLAP and neither
    // reached a host before (ba todo #1298).
    const FEATURES: &'static [&'static std::ffi::CStr] = &[
        features::AUDIO_EFFECT,
        features::MASTERING,
        features::ANALYZER,
        features::STEREO,
    ];

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        let viz = MasteringViz::new();
        Self {
            params: Arc::new(MasteringParams::default()),
            // The assistant's target choice rides along with the preset
            // identity (warmth-width-depth.md §7.4).
            presets: resonance_plugin::presets::PresetSession::with_extra(
                assistant::AssistantStateSaver::new(viz.clone()),
            ),
            viz,
            chain: None,
        }
    }

    fn param_count(&self) -> usize {
        PARAM_COUNT
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn initialize(&mut self, sample_rate: f32, max_buffer_size: u32) -> bool {
        self.viz.assistant.set_sample_rate(sample_rate);
        self.chain = Some(Chain::new(sample_rate, max_buffer_size as usize, &self.viz));
        true
    }

    fn reset(&mut self) {
        if let Some(chain) = &mut self.chain {
            chain.reset();
        }
    }

    fn latency_samples(&self) -> u32 {
        self.chain.as_ref().map(|c| c.latency()).unwrap_or(0)
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

        // `Chain::process` honours the bypass param itself: a
        // latency-matched dry path, crossfaded, with the stages kept warm.
        if let Some(chain) = &mut self.chain {
            chain.process(left, right, &self.params, &self.viz);
        }
    }

    /// The loaded-preset identity rides along with the parameter values,
    /// on both bridge paths, so reopening a saved project shows the preset
    /// the sound came from instead of a blank picker.
    fn extra_state_saver(&self) -> Option<Arc<dyn resonance_plugin::ExtraStateSaver>> {
        Some(self.presets.clone())
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::MasteringEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceMastering);
