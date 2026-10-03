//! Resonance Color — the character plugin for tracks and busses
//! (warmth-width-depth.md §6.1, decisions D1–D3).
//!
//! One saturation stage in five voicings — Tube, Tape, Transformer,
//! Console, Warm — built from the `resonance-dsp` W5 primitives
//! (`saturate`, `tape`, `transformer`, the IIR `Oversampler`), with a drive
//! tilt (`response`), an output tilt (`tone`), parallel `mix`, and
//! auto-gain on by default so a before/after judgement is made at matched
//! loudness. Tape mode has two qualities (decision D2, `tape_quality`):
//! Standard, the ADAA shaper plus filters, and HQ (slice W6b), which swaps
//! the shaper for a Jiles-Atherton hysteresis stage at 2× oversampling or
//! more and keeps every filter.
//!
//! Latency is 0 at every `oversample` setting and in both tape qualities:
//! the oversampler is the half-band IIR pair (decision D3), which has no
//! fixed latency to report, so changing the factor (or HQ forcing it up)
//! never needs a host restart.

use std::sync::Arc;

use resonance_plugin::*;

pub mod dsp;
pub mod params;
pub mod presets;
pub mod probe;
pub mod viz;

#[cfg(feature = "editor")]
pub mod editor;

use dsp::{ColorDsp, Settings};
use params::{ColorParams, PARAM_COUNT};
use viz::ColorViz;

pub struct ResonanceColor {
    /// Params shared with the editor. All storage is atomic, so the
    /// audio and UI threads both read through `&ColorParams`.
    pub params: Arc<ColorParams>,
    /// The loaded-preset identity, persisted beside the params.
    presets: Arc<resonance_plugin::presets::PresetSession>,
    /// Handed to the editor so its controls announce edits to the host
    /// (attached in `set_host`).
    editor_announcer: resonance_plugin::EditAnnouncer,
    viz: Arc<ColorViz>,
    dsp: Option<ColorDsp>,
}

impl ResonancePlugin for ResonanceColor {
    const CLAP_ID: &'static str = "com.resonance.color";
    const NAME: &'static str = "Resonance Color";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str =
        "Character saturation for tracks and busses: tube, tape, transformer, \
         console and warm voicings with auto-gain";
    const FEATURES: &'static [&'static std::ffi::CStr] = &[
        features::AUDIO_EFFECT,
        features::DISTORTION,
        features::STEREO,
    ];

    const FACTORY_PRESETS: &'static [resonance_plugin::presets::FactoryPreset] =
        presets::PRESETS;

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(ColorParams::default()),
            editor_announcer: resonance_plugin::EditAnnouncer::new(),
            presets: resonance_plugin::presets::PresetSession::for_plugin::<Self>(),
            viz: ColorViz::new(),
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
        self.dsp = Some(ColorDsp::new(sample_rate, &Settings::from_params(&self.params)));
        true
    }

    fn reset(&mut self) {
        let settings = Settings::from_params(&self.params);
        if let Some(dsp) = &mut self.dsp {
            dsp.reset(&settings);
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
        resonance_dsp::flush_denormals();
        let Some(dsp) = &mut self.dsp else {
            return;
        };
        let frames = frames.min(main.left.len()).min(main.right.len());
        let settings = Settings::from_params(&self.params);
        dsp.process(
            &mut main.left[..frames],
            &mut main.right[..frames],
            &settings,
            Some(&self.viz),
        );
    }

    /// Always 0, whatever `oversample` says (decision D3).
    fn latency_samples(&self) -> u32 {
        0
    }

    fn extra_state_saver(&self) -> Option<Arc<dyn resonance_plugin::ExtraStateSaver>> {
        Some(self.presets.clone())
    }

    fn set_host(&mut self, host: Arc<resonance_plugin::HostHandle>) {
        self.editor_announcer.attach(host);
    }

    fn param_text_source(&self) -> Option<Arc<dyn resonance_plugin::ParamTextSource>> {
        // The params are shared, so a host reads a live instance's real
        // values (not just display text) while the plugin is in the
        // audio processor — FU-P1a: without this, a third-party host
        // that never flushes between blocks sees a stale mirror for
        // any value an editor edit moved while the transport is
        // stopped.
        Some(Arc::new(ColorParamText(self.params.clone())))
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::ColorEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
            self.editor_announcer.clone(),
        )))
    }
}

/// Parameter text and live values over the shared `ColorParams`, for the
/// CLAP bridge while the plugin is active (FU-P1a).
struct ColorParamText(Arc<ColorParams>);

impl resonance_plugin::ParamTextSource for ColorParamText {
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

resonance_plugin::export_clap!(ResonanceColor);
