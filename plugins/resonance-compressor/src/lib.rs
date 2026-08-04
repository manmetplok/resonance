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

use std::sync::Arc;

use resonance_plugin::*;

pub mod dsp;
pub mod params;
pub mod presets;
pub mod viz;

#[cfg(feature = "editor")]
mod editor;

use dsp::CompressorDsp;
use params::{CompressorParams, PARAM_COUNT};
use viz::CompressorViz;

pub struct ResonanceCompressor {
    /// Params shared with the editor via `Arc`. All FloatParam/BoolParam
    /// storage is atomic internally so `&CompressorParams` is safe from
    /// both audio and UI threads.
    pub params: Arc<CompressorParams>,
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
    const FEATURES: &'static [&'static str] = &["audio-effect", "compressor", "stereo", "dynamics"];

    const INPUT_CHANNELS: Option<u32> = Some(2);
    const SIDECHAIN_INPUT: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(CompressorParams::default()),
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
        resonance_common::flush_denormals();

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
        resonance_common::flush_denormals();

        let Some(dsp) = &mut self.dsp else {
            return;
        };

        dsp.process_stereo(
            left,
            right,
            key.map(|k| (k.left, k.right)),
            &self.params,
            &self.viz,
        );
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::CompressorEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceCompressor);

