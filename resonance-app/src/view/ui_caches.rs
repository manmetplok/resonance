//! Cached pick-list option lists for the view layer.
//!
//! Iced rebuilds the entire widget tree every frame, including pick_list
//! options. Without caching, a continuous window resize allocates dozens
//! of option `Vec`s per frame (one per pick_list per strip, plus the
//! inspector's input/output/MIDI pickers). These caches hold an
//! `Rc<[T]>` per option list — the view clones the Rc cheaply each
//! frame and the underlying Vec only rebuilds when the source data
//! changes (a device list update, bus add/remove, plugin scan).
//!
//! **When to add a new cache here:** any time you reach for a Vec
//! inside `view()` that's a function of state that doesn't change every
//! frame. See `.claude/skills/ui-work.md` §11.

use std::borrow::Borrow;
use std::rc::Rc;

use resonance_audio::MidiDeviceInfo;
use resonance_audio::types::{InputDeviceInfo, ScannedPlugin};

use crate::state::BusState;
use crate::view::mixer::picks::{
    input_channel_choices, midi_choices_base, output_channel_choices, output_choices_for,
    MidiChannelChoice, MidiPickerChoice, OutputChoice,
};

#[derive(Debug, Clone)]
pub(crate) struct UiViewCaches {
    /// "(None)" entry plus every currently-enumerated MIDI input device.
    /// Used by the per-track MIDI input picker in the mixer inspector.
    pub midi_input_choices: Rc<[MidiPickerChoice]>,
    /// "(None)" entry plus every currently-enumerated MIDI output device.
    pub midi_output_choices: Rc<[MidiPickerChoice]>,
    /// `→ Master` plus `→ <bus>` for every bus, with the ARROW_RIGHT
    /// glyph baked into each label.
    pub output_choices: Rc<[OutputChoice]>,
    /// "Omni" plus channels 1..=16 — input-side MIDI channel filter.
    /// Built once at startup; never invalidates.
    pub input_channel_choices: Rc<[MidiChannelChoice]>,
    /// Channels 1..=16 — output-side MIDI channel selector. No Omni
    /// entry since outputs always emit on a specific channel. Built
    /// once at startup.
    pub output_channel_choices: Rc<[MidiChannelChoice]>,
    /// `available_plugins` filtered to non-instruments. Used by the
    /// `+ FX` picker on every strip, bus, and the master.
    pub fx_plugins: Rc<[ScannedPlugin]>,
    /// `available_plugins` filtered to instruments. Used by the
    /// `+ Instrument` picker on instrument tracks with no plugin yet.
    pub instrument_plugins: Rc<[ScannedPlugin]>,
    /// Audio input devices enumerated by the engine. Cached so
    /// per-track input-device pickers in the mixer inspector and the
    /// bounce-dialog don't clone the full Vec every frame.
    pub input_devices: Rc<[InputDeviceInfo]>,
}

impl Default for UiViewCaches {
    fn default() -> Self {
        Self {
            midi_input_choices: Rc::from(Vec::<MidiPickerChoice>::new()),
            midi_output_choices: Rc::from(Vec::<MidiPickerChoice>::new()),
            // Always seed with the Master entry so the inspector's
            // output picker has at least one choice on a fresh project
            // (no busses, no project load, no demo seed). Without this
            // seed, opening the Mixer tab right after adding a track to
            // a brand-new project panics on `choices[0]` in
            // `inspector::output_block`. `output_choices_for(&[])`
            // returns exactly `[Master]`, matching what the first
            // `rebuild_output(&[])` would produce.
            output_choices: Rc::from(output_choices_for(&[])),
            input_channel_choices: Rc::from(input_channel_choices()),
            output_channel_choices: Rc::from(output_channel_choices()),
            fx_plugins: Rc::from(Vec::<ScannedPlugin>::new()),
            instrument_plugins: Rc::from(Vec::<ScannedPlugin>::new()),
            input_devices: Rc::from(Vec::<InputDeviceInfo>::new()),
        }
    }
}

impl UiViewCaches {
    /// Rebuild the MIDI-input picker option list. Call after the engine
    /// re-enumerates devices (or after project load if the cache might
    /// be stale).
    pub fn rebuild_midi_input(&mut self, devices: &[MidiDeviceInfo]) {
        self.midi_input_choices = Rc::from(midi_choices_base(devices));
    }

    /// Same as `rebuild_midi_input` for the MIDI-out picker.
    pub fn rebuild_midi_output(&mut self, devices: &[MidiDeviceInfo]) {
        self.midi_output_choices = Rc::from(midi_choices_base(devices));
    }

    /// Rebuild the Master + every-bus output destination options. Call
    /// after any bus add/remove/rename.
    pub fn rebuild_output(&mut self, busses: &[BusState]) {
        self.output_choices = Rc::from(output_choices_for(busses));
    }

    /// Rebuild the FX-only and instrument-only filters off the supplied
    /// available-plugins list. Call after the plugin scan completes or
    /// when new plugins are added at runtime.
    pub fn rebuild_plugins(&mut self, available: &[ScannedPlugin]) {
        let fx: Vec<ScannedPlugin> = available
            .iter()
            .filter(|p| !p.is_instrument)
            .cloned()
            .collect();
        let inst: Vec<ScannedPlugin> = available
            .iter()
            .filter(|p| p.is_instrument)
            .cloned()
            .collect();
        self.fx_plugins = Rc::from(fx);
        self.instrument_plugins = Rc::from(inst);
    }

    /// Rebuild the audio-input-device list cache. Call after the engine
    /// re-enumerates devices.
    pub fn rebuild_input_devices(&mut self, devices: &[InputDeviceInfo]) {
        self.input_devices = Rc::from(devices.to_vec());
    }
}

/// Borrowed-or-owned wrapper for a `pick_list`'s option slice. The
/// common path is the `Cached` branch — a refcounted slice cloned from
/// `UiViewCaches`. The `Owned` branch covers rare cases (a track with
/// a configured-but-unplugged MIDI device) where the call site needs
/// to append an entry on top of the cached list. iced's `pick_list`
/// accepts any `L: Borrow<[T]>`, so this enum slots straight in.
#[derive(Debug)]
pub(crate) enum ChoiceList<T: 'static> {
    Cached(Rc<[T]>),
    Owned(Vec<T>),
}

impl<T: 'static> Borrow<[T]> for ChoiceList<T> {
    fn borrow(&self) -> &[T] {
        match self {
            Self::Cached(rc) => rc,
            Self::Owned(v) => v,
        }
    }
}

impl<T: Clone + 'static> Clone for ChoiceList<T> {
    fn clone(&self) -> Self {
        match self {
            Self::Cached(rc) => Self::Cached(rc.clone()),
            Self::Owned(v) => Self::Owned(v.clone()),
        }
    }
}

