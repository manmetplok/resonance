//! `midi_map.*` — the project's MIDI Learn bindings (doc #167): which
//! hardware control (a CC or a note on the control-surface port) drives
//! which track fader / pan / mute / solo, send level, plugin parameter or
//! transport action.
//!
//! An agent cannot move a knob, so it does not create bindings itself:
//! [`LEARN`] arms learn for a target and the USER moves the control they
//! want; the next [`BINDINGS`] read shows the result. Clearing is direct.
//! Every binding edit is one undo entry (`edit.undo` backs it out), and
//! the bindings are saved with the project. Which input port is the
//! control surface is the user's machine setting, reported here but set
//! in the app's Settings › MIDI.

use crate::ids::TrackId;
use serde::{Deserialize, Serialize};

/// `midi_map.bindings` — every binding, the armed learn target and the
/// control-surface port ([`BindingsResult`]). Read-only. No params.
pub const BINDINGS: &str = "midi_map.bindings";
/// `midi_map.learn` — arm learn for one target ([`LearnParams`] →
/// [`LearnResult`]): the next control the user moves is bound to it.
pub const LEARN: &str = "midi_map.learn";
/// `midi_map.cancel_learn` — disarm learn without binding anything
/// ([`CancelLearnResult`]). No params.
pub const CANCEL_LEARN: &str = "midi_map.cancel_learn";
/// `midi_map.clear` — remove one binding by id ([`ClearParams`] →
/// [`ClearResult`]).
pub const CLEAR: &str = "midi_map.clear";
/// `midi_map.clear_all` — remove every binding ([`ClearResult`]). No
/// params.
pub const CLEAR_ALL: &str = "midi_map.clear_all";

/// All `midi_map.*` method names.
pub const METHODS: &[&str] = &[BINDINGS, LEARN, CANCEL_LEARN, CLEAR, CLEAR_ALL];

/// A mixer control on a track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum MidiControl {
    Volume,
    Pan,
    Mute,
    Solo,
}

/// A transport action a pad can trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum MidiTransport {
    Play,
    Stop,
    Record,
    /// Toggle the loop.
    Loop,
}

/// What a binding drives. Exactly one of: `transport`; or `track_id`
/// with one of `control`, `send_id` or `param`.
///
/// Busses and the master have no MIDI-learnable fader; a plugin
/// parameter on a track is addressed as in `track_set_plugin_param`
/// (`param` by name or numeric id, `plugin_id` + `occurrence` when it is
/// not the track's instrument).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MidiTargetSpec {
    /// The track (ids from `song_summary`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_id: Option<TrackId>,
    /// The track's `volume`, `pan`, `mute` or `solo`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<MidiControl>,
    /// One of the track's aux sends, by send id (`track_sends`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_id: Option<u64>,
    /// A plugin parameter on the track, by name or numeric id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    /// With `param`: the plugin's CLAP id. Omitted = the instrument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// With `param`: which of several same-id plugins (0-based).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// A transport action (no `track_id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<MidiTransport>,
}

/// The hardware control a binding listens to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MidiControlSource {
    /// `"cc"` (a knob, fader or encoder) or `"note"` (a pad or button).
    pub kind: String,
    /// MIDI channel, 1-16 as hardware labels it.
    pub channel: u8,
    /// The CC number or note number, 0-127.
    pub number: u8,
    /// CC only: `"absolute"`, or `"relative"` for an endless encoder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
}

/// A target as it reads back: the spec that addresses it, plus its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MidiTargetView {
    /// The same fields [`MidiTargetSpec`] takes. A plugin parameter
    /// reads back by numeric id (`param: "12"`) with its `plugin_id`.
    #[serde(flatten)]
    pub spec: MidiTargetSpec,
    /// What the app's MIDI lists call it: `"Bass · Volume"`,
    /// `"EQ · Gain"`, `"Transport · Play"`. A target whose track or
    /// plugin has been deleted says so (`"Deleted track · Volume"`).
    pub label: String,
}

/// One binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MidiBindingView {
    /// Pass to `midi_map.clear`.
    pub id: u64,
    pub source: MidiControlSource,
    pub target: MidiTargetView,
}

/// Result of `midi_map.bindings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct BindingsResult {
    /// Every binding, by id.
    pub bindings: Vec<MidiBindingView>,
    /// The target learn is armed for, or `null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning: Option<MidiTargetView>,
    /// The MIDI input port the bindings are played from (the user's
    /// machine setting), or `null` when none is chosen — then no
    /// hardware move reaches a binding and learn captures nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_surface_input: Option<String>,
}

/// Params of `midi_map.learn`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct LearnParams {
    #[serde(flatten)]
    pub target: MidiTargetSpec,
}

/// Result of `midi_map.learn`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LearnResult {
    /// The armed target, resolved.
    pub learning: MidiTargetView,
    pub revision: u64,
}

/// Result of `midi_map.cancel_learn`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CancelLearnResult {
    /// Whether learn was armed.
    pub cancelled: bool,
    pub revision: u64,
}

/// Params of `midi_map.clear`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ClearParams {
    /// A binding id from `midi_map.bindings`.
    pub id: u64,
}

/// Result of `midi_map.clear` / `midi_map.clear_all`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ClearResult {
    /// How many bindings were removed.
    pub cleared: u32,
    pub revision: u64,
}
