//! Resonance Mastering — automatic mastering plugin for band music.
//!
//! A fixed-order mastering chain ([`chain`]): input trim → corrective
//! linear-phase EQ → de-harsh → glue compressor → saturator → tonal
//! linear-phase EQ
//! → linear-phase multiband compressor → M/S imager → clipper →
//! true-peak limiter → dither, every stage off by default. The EQ bands
//! each filter the stereo pair, the mid or the side; the imager has a
//! global and a per-band width (on the multiband's crossover); the
//! saturator has the original Tube↔Tape blend plus the character modes
//! of [`stages::sat_modes`]. The reported latency is constant: the
//! linear-phase stages, the de-harsh frame and the limiter lookahead,
//! whatever is switched on. Two stages add a small frequency-dependent
//! group delay on top that the host's delay compensation does not see:
//! the clipper (~6.9 samples at low frequencies) and the non-Blend
//! saturator modes (~5.5), both from their IIR half-band oversampling
//! (see [`chain`]).
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
    /// Handed to the editor so its controls announce edits to the host
    /// (attached in `set_host`).
    editor_announcer: resonance_plugin::EditAnnouncer,
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
    const CLAP_ID: &'static str = resonance_plugin::first_party::MASTERING;
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
            editor_announcer: resonance_plugin::EditAnnouncer::new(),
            presets: resonance_plugin::presets::PresetSession::for_plugin_with_extra::<Self>(
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

    fn set_host(&mut self, host: Arc<resonance_plugin::HostHandle>) {
        self.editor_announcer.attach(host);
    }

    fn param_text_source(&self) -> Option<Arc<dyn resonance_plugin::ParamTextSource>> {
        // FU-P1a: the params are shared, so a host reads a live
        // instance's real values while the plugin is in the audio
        // processor — without this, a third-party host that never
        // flushes between blocks sees a stale mirror for any value an
        // editor edit moved while the transport is stopped.
        Some(Arc::new(MasteringParamText(self.params.clone())))
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::MasteringEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
            self.editor_announcer.clone(),
        )))
    }
}

/// Parameter text and live values over the shared `MasteringParams`,
/// for the CLAP bridge while the plugin is active (FU-P1a).
struct MasteringParamText(Arc<MasteringParams>);

impl resonance_plugin::ParamTextSource for MasteringParamText {
    fn display(&self, index: usize, value: f64) -> Option<String> {
        (index < PARAM_COUNT).then(|| self.0.param_at(index).display(value))
    }

    fn parse(&self, index: usize, text: &str) -> Option<f64> {
        if index >= PARAM_COUNT {
            return None;
        }
        self.0.param_at(index).parse(text)
    }

    fn live_value(&self, index: usize) -> Option<f64> {
        (index < PARAM_COUNT).then(|| self.0.param_at(index).get_plain())
    }
}

resonance_plugin::export_clap!(ResonanceMastering);
