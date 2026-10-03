/// Resonance Wavetable - A wavetable synthesizer instrument CLAP plugin.
use std::sync::Arc;

use resonance_plugin::*;

#[cfg(feature = "editor")]
pub mod editor;
pub mod dsp;
pub mod params;
pub mod presets;
pub mod user_wavetable;
pub mod viz;

use dsp::engine::SynthEngine;
use params::{WavetableParams, PARAM_COUNT};
use user_wavetable::{UserTableSwap, UserWavetables};
use viz::WavetableVizState;

pub struct ResonanceWavetable {
    /// Parameters — shared with the editor thread via Arc so the UI can read
    /// and write from a separate thread. All `FloatParam` / `IntParam` /
    /// `BoolParam` fields use atomic storage internally, so `&WavetableParams`
    /// is safe to use concurrently from audio + UI.
    params: Arc<WavetableParams>,
    /// Which preset is loaded and whether it has been edited since.
    /// Shared with the editor thread and handed to the bridge as this
    /// plugin's extra state, so the identity survives closing the window
    /// (ba todo #1358).
    presets: Arc<resonance_plugin::presets::PresetSession>,
    /// Handed to the editor so its controls announce edits to the host
    /// (attached in `set_host`).
    editor_announcer: resonance_plugin::EditAnnouncer,
    engine: SynthEngine,
    /// Shared audio-thread → UI-thread visualisation state. Lives as long as
    /// the plugin instance. Cloned into the editor factory when the host
    /// opens the GUI.
    viz: Arc<WavetableVizState>,
    /// Per-oscillator user wavetables: what is loaded, and the mailboxes the
    /// loader posts finished tables through. Shared with the editor (which
    /// requests loads) and chained behind `presets` as extra state.
    user_tables: Arc<UserWavetables>,
    /// Audio-thread half of the user-table hand-off.
    user_swap: UserTableSwap,
}

impl ResonanceWavetable {
    /// The user-wavetable state, for driving imports without the editor
    /// (tests, a future control surface).
    pub fn user_wavetables(&self) -> &Arc<UserWavetables> {
        &self.user_tables
    }

    /// The synth engine, read-only.
    pub fn engine(&self) -> &SynthEngine {
        &self.engine
    }
}

impl ResonancePlugin for ResonanceWavetable {
    const CLAP_ID: &'static str = "com.resonance.wavetable";
    const NAME: &'static str = "Resonance Wavetable";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "A wavetable synthesizer instrument";
    const FEATURES: &'static [&'static std::ffi::CStr] = &[
        features::INSTRUMENT,
        features::SYNTHESIZER,
        features::STEREO,
    ];

    /// The factory bank, declared once here and read by the editor's
    /// PresetBank and by the exported `resonance_factory_presets`
    /// symbol the host lists over the control API (ba todo #1333).
    const FACTORY_PRESETS: &'static [resonance_plugin::presets::FactoryPreset] =
        presets::PRESETS;

    const INPUT_CHANNELS: Option<u32> = None;
    const MIDI_INPUT: bool = true;

    fn new() -> Self {
        let user_tables = Arc::new(UserWavetables::new());
        Self {
            params: Arc::new(WavetableParams::new()),
            // The preset identity wraps the user-table saver: chaining is
            // why `with_extra` exists.
            editor_announcer: resonance_plugin::EditAnnouncer::new(),
            presets: resonance_plugin::presets::PresetSession::for_plugin_with_extra::<Self>(user_tables.clone()),
            engine: SynthEngine::new(),
            viz: Arc::new(WavetableVizState::new()),
            user_tables,
            user_swap: UserTableSwap::new(),
        }
    }

    fn param_count(&self) -> usize {
        PARAM_COUNT
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn initialize(&mut self, sample_rate: f32, max_buffer_size: u32) -> bool {
        self.engine.initialize(sample_rate);
        // The janitor that frees displaced user tables off the audio thread,
        // and any table restored before activation, installed now rather
        // than on the first block.
        self.user_swap.prepare();
        self.user_swap.apply(&self.user_tables, &mut self.engine);
        // Publish the host's real audio config so the editor's status bar
        // reports it instead of printing invented literals.
        self.viz.store_io_config(sample_rate, max_buffer_size);
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
        tempo: Option<TempoInfo>,
    ) {
        let Some(main) = outputs.first_mut() else {
            return;
        };
        let left = &mut main.left[..frames];
        let right = &mut main.right[..frames];
        resonance_dsp::flush_denormals();

        // Pick up a freshly imported (or cleared) user wavetable. A
        // non-blocking mailbox take and two slot writes; the displaced
        // table leaves for the janitor thread.
        self.user_swap.apply(&self.user_tables, &mut self.engine);

        // The engine drains `events` with sample-accurate timing internally
        // and snapshots every atomic parameter once for the whole block --
        // the per-sample kernel reads only from stack locals from there on.
        // `tempo` drives the tempo-synced LFO modes.
        self.engine
            .render_block(left, right, frames, &self.params, events, tempo);

        // Publish audio-thread state to the shared viz atomics once per
        // block. The editor thread reads from these at ~60 Hz.
        self.engine.publish_viz(&self.params, &self.viz);
    }

    /// The loaded-preset identity rides along with the parameter values,
    /// on both bridge paths, so reopening a saved project shows the preset
    /// the sound came from instead of a blank picker. The user wavetables
    /// ride behind it (`user_wavetable::state`).
    fn extra_state_saver(&self) -> Option<Arc<dyn resonance_plugin::ExtraStateSaver>> {
        Some(self.presets.clone())
    }

    fn set_host(&mut self, host: Arc<resonance_plugin::HostHandle>) {
        self.editor_announcer.attach(host);
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::WavetableEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
            self.user_tables.clone(),
            self.editor_announcer.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceWavetable);
