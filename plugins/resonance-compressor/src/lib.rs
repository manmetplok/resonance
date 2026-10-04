//! Resonance Compressor — a stereo feed-forward compressor with soft
//! knee, peak/RMS-blended detector, an external sidechain key, an
//! optional sidechain HPF, parallel mix, and auto-makeup gain. DSP is
//! intentionally log-domain and cheap; the editor shows a live transfer
//! curve + GR history + In/GR/Out meters.
//!
//! Note that "sidechain HPF" and "sidechain key" are two different
//! things, and the plugin had only the first for a long time: the HPF is
//! an internal filter on the detector path, whereas the key is an
//! external signal — another track or bus — that replaces the detector's
//! source entirely. Ducking a pad from a kick needs the key; no amount
//! of internal filtering can do it.
//!
//! Whether a key is actually connected is published into the shared viz
//! object each block, together with the level the key reaches at the
//! detector, so the editor can name the detector's source and meter it
//! instead of leaving the user to guess why the GR meter moves while the
//! input meter is idle.

use std::sync::Arc;

use resonance_plugin::*;

pub mod dsp;
pub mod params;
pub mod presets;
pub mod viz;

#[cfg(feature = "editor")]
pub mod editor;

use dsp::CompressorDsp;
use params::{CompressorParams, PARAM_COUNT};
use viz::CompressorViz;

pub struct ResonanceCompressor {
    /// Params shared with the editor via `Arc`. All FloatParam/BoolParam
    /// storage is atomic internally so `&CompressorParams` is safe from
    /// both audio and UI threads.
    pub params: Arc<CompressorParams>,
    /// Which preset is loaded and whether it has been edited since.
    /// Shared with the editor thread and handed to the bridge as this
    /// plugin's extra state, so the identity survives closing the
    /// window (ba todo #1358).
    presets: Arc<resonance_plugin::presets::PresetSession>,
    /// Handed to the editor so its controls announce edits to the host
    /// (attached in `set_host`).
    editor_announcer: resonance_plugin::EditAnnouncer,
    /// Shared viz snapshots (meters + GR history ring) read by the editor.
    viz: Arc<CompressorViz>,
    dsp: Option<CompressorDsp>,
}

impl ResonancePlugin for ResonanceCompressor {
    const CLAP_ID: &'static str = "com.resonance.compressor";
    const NAME: &'static str = "Resonance Compressor";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str =
        "Stereo feed-forward compressor with soft knee, external sidechain key, \
         sidechain HPF, and parallel mix";
    // `compressor` is standard CLAP and never reached a host before
    // (ba todo #1298); `dynamics` was not a CLAP feature at all.
    const FEATURES: &'static [&'static std::ffi::CStr] = &[
        features::AUDIO_EFFECT,
        features::COMPRESSOR,
        features::STEREO,
    ];

    /// The factory bank, declared once here and read by the editor's
    /// PresetBank and by the exported `resonance_factory_presets`
    /// symbol the host lists over the control API (ba todo #1333).
    const FACTORY_PRESETS: &'static [resonance_plugin::presets::FactoryPreset] =
        presets::PRESETS;

    const INPUT_CHANNELS: Option<u32> = Some(2);
    const SIDECHAIN_INPUT: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(CompressorParams::default()),
            editor_announcer: resonance_plugin::EditAnnouncer::new(),
            presets: resonance_plugin::presets::PresetSession::for_plugin::<Self>(),
            viz: CompressorViz::new(),
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
        self.dsp = Some(CompressorDsp::new(sample_rate, &self.params));
        true
    }

    fn reset(&mut self) {
        if let Some(dsp) = &mut self.dsp {
            dsp.reset();
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

        dsp.process_stereo(left, right, None, &self.params, &self.viz);
    }

    fn process_with_key(
        &mut self,
        outputs: &mut [OutputBuffer<'_>],
        key: Option<KeyBuffer<'_>>,
        _frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        let Some(main) = outputs.first_mut() else {
            return;
        };
        let left = &mut *main.left;
        let right = &mut *main.right;
        resonance_dsp::flush_denormals();

        let Some(dsp) = &mut self.dsp else {
            return;
        };

        // The `Option` carries two things: the key samples, and the fact
        // that a key exists at all. Both go to the DSP, which publishes
        // the presence bit into the viz object for the editor.
        dsp.process_stereo(
            left,
            right,
            key.map(|k| (k.left, k.right)),
            &self.params,
            &self.viz,
        );
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
        Some(Arc::new(CompressorParamText(self.params.clone())))
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::CompressorEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
            self.editor_announcer.clone(),
        )))
    }
}

/// Parameter text and live values over the shared `CompressorParams`,
/// for the CLAP bridge while the plugin is active (FU-P1a).
struct CompressorParamText(Arc<CompressorParams>);

impl resonance_plugin::ParamTextSource for CompressorParamText {
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

resonance_plugin::export_clap!(ResonanceCompressor);

