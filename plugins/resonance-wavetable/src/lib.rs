/// Resonance Wavetable - A wavetable synthesizer instrument CLAP plugin.
use std::sync::Arc;

use resonance_plugin::*;

#[cfg(feature = "editor")]
mod editor;
pub mod dsp;
pub mod params;
pub mod presets;
pub mod viz;

use dsp::engine::SynthEngine;
use params::{WavetableParams, PARAM_COUNT};
use viz::WavetableVizState;

pub struct ResonanceWavetable {
    /// Parameters — shared with the editor thread via Arc so the UI can read
    /// and write from a separate thread. All `FloatParam` / `IntParam` /
    /// `BoolParam` fields use atomic storage internally, so `&WavetableParams`
    /// is safe to use concurrently from audio + UI.
    params: Arc<WavetableParams>,
    engine: SynthEngine,
    /// Shared audio-thread → UI-thread visualisation state. Lives as long as
    /// the plugin instance. Cloned into the editor factory when the host
    /// opens the GUI.
    viz: Arc<WavetableVizState>,
}

impl ResonancePlugin for ResonanceWavetable {
    const CLAP_ID: &'static str = "com.resonance.wavetable";
    const NAME: &'static str = "Resonance Wavetable";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "A wavetable synthesizer instrument";
    const FEATURES: &'static [&'static str] = &["instrument", "synthesizer", "stereo"];

    const INPUT_CHANNELS: Option<u32> = None;
    const MIDI_INPUT: bool = true;

    fn new() -> Self {
        Self {
            params: Arc::new(WavetableParams::new()),
            engine: SynthEngine::new(),
            viz: Arc::new(WavetableVizState::new()),
        }
    }

    fn param_count(&self) -> usize {
        PARAM_COUNT
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.engine.initialize(sample_rate);
        // Start the master-volume smoother at the current param value so a
        // fresh instance doesn't fade in from zero (same pattern as
        // resonance-eq's output-gain smoother).
        self.engine
            .master_vol_smoother
            .reset(self.params.master_volume.value());
        true
    }

    fn reset(&mut self) {
        self.engine.reset();
    }

    fn process(
        &mut self,
        outputs: &mut [resonance_plugin::OutputBuffer<'_>],
        frames: usize,
        events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        let Some(main) = outputs.first_mut() else {
            return;
        };
        let left = &mut main.left[..frames];
        let right = &mut main.right[..frames];
        resonance_common::flush_denormals();

        // The engine drains `events` with sample-accurate timing internally
        // and snapshots every atomic parameter once for the whole block --
        // the per-sample kernel reads only from stack locals from there on.
        self.engine
            .render_block(left, right, frames, &self.params, events);

        // Publish audio-thread state to the shared viz atomics once per
        // block. The editor thread reads from these at ~60 Hz.
        self.engine.publish_viz(&self.params, &self.viz);
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::WavetableEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceWavetable);
