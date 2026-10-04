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
//! frame. See `ux-guidelines.md` → "View Performance".

use std::borrow::Borrow;
use std::rc::Rc;

use resonance_audio::MidiDeviceInfo;
use resonance_audio::types::{InputDeviceInfo, ScannedPlugin};

use crate::state::picks::{
    bank_choices, device_choices, input_channel_choices, midi_choices_base,
    output_channel_choices, output_choices_for, program_choices, BankChoice, DevicePresetChoice,
    MidiChannelChoice, MidiPickerChoice, OutputChoice, ProgramChoice,
};
use crate::state::BusState;

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
    /// "(no bank)" plus banks 0..=127 — the external-instrument Patch
    /// card's Bank Select picker. Built once at startup; never
    /// invalidates.
    pub bank_choices: Rc<[BankChoice]>,
    /// "(no program)" plus programs 0..=127 — the external-instrument
    /// Patch card's Program Change picker. Built once at startup; never
    /// invalidates.
    pub program_choices: Rc<[ProgramChoice]>,
    /// "(no device)" plus one entry per device definition in the registry —
    /// the External-Instrument inspector's device-preset picker (epic #40).
    /// Rebuilt only when the device registry changes (startup today).
    pub device_choices: Rc<[DevicePresetChoice]>,
    /// Revision memo keying the Compose right rail's `lazy` (UX-10).
    pub compose_rail: RevisionMemo<ComposeRailInputs>,
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
            bank_choices: Rc::from(bank_choices()),
            program_choices: Rc::from(program_choices()),
            // Seeded with the "(no device)" clear entry so the picker has a
            // valid option before the registry is scanned; `Resonance::new`
            // rebuilds it from the bundled + user definitions at startup.
            device_choices: Rc::from(device_choices(&[])),
            compose_rail: RevisionMemo::default(),
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

    /// Rebuild the device-preset picker options off the device registry's
    /// current `list()`. Call after the registry is (re-)scanned; today the
    /// registry is built once at startup.
    pub fn rebuild_device_choices(&mut self, defs: &[&resonance_common::DeviceDefinition]) {
        self.device_choices = Rc::from(device_choices(defs));
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

/// Combine a cached MIDI device option list with an override entry for
/// the rare case where the configured device is no longer enumerated by
/// the engine (controller unplugged). The normal path returns the
/// `Cached` variant — a cheap `Rc` clone with no allocation.
///
/// Shared by the mixer inspector's per-track MIDI pickers and the
/// Settings overlay's MIDI-clock pickers, so neither surface rebuilds
/// its option `Vec` per frame.
pub(crate) fn midi_choices_with_override(
    cached: &Rc<[MidiPickerChoice]>,
    configured: Option<&str>,
    available: &[MidiDeviceInfo],
) -> ChoiceList<MidiPickerChoice> {
    match configured.filter(|name| !available.iter().any(|d| d.name == *name)) {
        Some(stale) => {
            let mut v: Vec<MidiPickerChoice> = cached.iter().cloned().collect();
            v.push(MidiPickerChoice(Some(stale.to_string())));
            ChoiceList::Owned(v)
        }
        None => ChoiceList::Cached(cached.clone()),
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


/// Equality memo that turns "the inputs a view region reads" into a `u64`
/// revision for `iced::widget::lazy`. Many view inputs (section
/// definitions with float generator params, drum patterns) can't derive
/// `Hash`, and a hand-written fingerprint silently goes stale when someone
/// adds a field. Instead, the view snapshots everything the region reads
/// into one `PartialEq` value each frame and calls [`revision`]: an equal
/// snapshot keeps the revision (the lazy region is reused), any difference
/// — an edit, an undo, a control-API write, a project load — bumps it.
/// Comparing is a field walk with no allocation; the snapshot is only
/// stored (moved) when it changed.
///
/// [`revision`]: RevisionMemo::revision
#[derive(Debug)]
pub(crate) struct RevisionMemo<T> {
    inner: std::cell::RefCell<(Option<T>, u64)>,
}

impl<T> Default for RevisionMemo<T> {
    fn default() -> Self {
        Self {
            inner: std::cell::RefCell::new((None, 0)),
        }
    }
}

impl<T> Clone for RevisionMemo<T> {
    /// A clone starts empty: the memo is a per-view cache, never state.
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl<T: PartialEq> RevisionMemo<T> {
    /// The revision for `inputs`: unchanged when `inputs` equals the last
    /// snapshot, otherwise bumped (and `inputs` becomes the snapshot).
    pub(crate) fn revision(&self, inputs: T) -> u64 {
        let mut inner = self.inner.borrow_mut();
        if inner.0.as_ref() != Some(&inputs) {
            inner.0 = Some(inputs);
            inner.1 = inner.1.wrapping_add(1);
        }
        inner.1
    }
}

/// Everything the Compose right rail (`view::compose::lane_inspector`)
/// reads, owned, for [`RevisionMemo`]. Keep in step with
/// `lane_inspector::view`'s parameters: a field the rail reads but this
/// struct lacks is a stale-rail bug. The table registry is left out on
/// purpose — it is built once at startup and never mutated.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ComposeRailInputs {
    pub definition: crate::compose::SectionDefinitionState,
    pub selected_lane: crate::compose::SelectedLane,
    /// `(id, name, type)` of every track — the rail's only `TrackState`
    /// reads (EDITING header, Name field, generator options). Levels and
    /// the rest of `TrackState` tick constantly and are not read.
    pub tracks: Vec<(resonance_audio::types::TrackId, String, resonance_audio::types::TrackType)>,
    pub drumroll: crate::compose::DrumrollViewState,
    pub drum_groups: Vec<crate::compose::DrumGroup>,
    pub drum_patterns: Vec<crate::compose::drumroll::DrumPattern>,
    pub clip_id_for_drum: Option<u64>,
    /// Set equality is order-independent, so iteration order can't fake
    /// a change.
    pub collapsed_panels: std::collections::HashSet<crate::compose::RailPanelKey>,
    pub vocal_tempo_warning: Option<crate::compose::VocalTempoMismatch>,
}
