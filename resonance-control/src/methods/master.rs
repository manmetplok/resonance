//! `master.*` — the final summing stage, after every track and bus.
//!
//! The master bus has always existed in the app (volume, an insert
//! chain, an FX bypass); it was simply unreachable from the control API,
//! which is why a mix driven entirely over MCP could be balanced
//! correctly and still have nowhere to put a limiter (ba doc #273).
//!
//! [`SUMMARY`] and [`PLUGIN_PARAMS`] are read-only but need an open
//! project. The rest mutate and return
//! [`crate::common::MutationAck`], except [`ADD_EFFECT`], which returns
//! the new plugin's slot.

use crate::methods::track::PluginParamsEntry;
use serde::{Deserialize, Serialize};

/// `master.summary` — master volume, FX bypass and insert chain
/// ([`MasterSummary`]). No params. Read-only.
pub const SUMMARY: &str = "master.summary";
/// `master.set_volume` — set the master fader ([`SetMasterVolumeParams`]).
pub const SET_VOLUME: &str = "master.set_volume";
/// `master.add_effect` — append an effect to the master insert chain
/// ([`AddEffectParams`] -> [`crate::methods::track::AddPluginResult`]).
pub const ADD_EFFECT: &str = "master.add_effect";
/// `master.remove_effect` — take an effect off the master chain
/// ([`RemoveEffectParams`]).
pub const REMOVE_EFFECT: &str = "master.remove_effect";
/// `master.move_effect` — reorder the master chain ([`MoveEffectParams`]
/// -> `MutationAck`).
pub const MOVE_EFFECT: &str = "master.move_effect";
/// `master.replace_effect` — put a different plugin in a chain slot,
/// keeping its position ([`ReplaceEffectParams`] ->
/// [`ReplaceEffectResult`](crate::methods::track::ReplaceEffectResult)).
pub const REPLACE_EFFECT: &str = "master.replace_effect";
/// `master.set_fx_bypass` — bypass/unbypass the whole master chain
/// ([`SetFxBypassParams`]).
pub const SET_FX_BYPASS: &str = "master.set_fx_bypass";
/// `master.plugin_params` — the master chain's plugins and their
/// parameters, read-only ([`PluginParamsParams`] -> [`PluginParamsView`]).
pub const PLUGIN_PARAMS: &str = "master.plugin_params";
/// `master.set_plugin_param` — set one parameter on a master plugin
/// ([`SetPluginParamParams`] -> `MutationAck`).
pub const SET_PLUGIN_PARAM: &str = "master.set_plugin_param";
/// `master.set_plugin_bypass` — bypass/unbypass ONE slot in the master
/// chain ([`SetPluginBypassParams`] -> `MutationAck`).
pub const SET_PLUGIN_BYPASS: &str = "master.set_plugin_bypass";
/// `master.plugin_presets` — a master plugin's factory and user presets
/// ([`PluginPresetsParams`] ->
/// [`PluginPresetsView`](crate::methods::plugin_preset::PluginPresetsView)).
/// Read-only.
pub const PLUGIN_PRESETS: &str = "master.plugin_presets";
/// `master.load_plugin_preset` — recall a preset onto a master plugin
/// ([`LoadPluginPresetParams`] -> `MutationAck`).
pub const LOAD_PLUGIN_PRESET: &str = "master.load_plugin_preset";
/// `master.save_plugin_preset` — save a master plugin's current settings
/// as a named user preset ([`SavePluginPresetParams`] -> `MutationAck`).
pub const SAVE_PLUGIN_PRESET: &str = "master.save_plugin_preset";
/// `master.set_sidechain` — key a plugin on the master chain from a
/// track or bus ([`SetSidechainParams`] -> `MutationAck`).
pub const SET_SIDECHAIN: &str = "master.set_sidechain";
/// `master.clear_sidechain` — remove a master plugin's key route
/// ([`ClearSidechainParams`] -> `MutationAck`).
pub const CLEAR_SIDECHAIN: &str = "master.clear_sidechain";
/// `master.assist` — run the mastering assistant offline over the master
/// mix and return its suggestions WITHOUT applying them ([`AssistParams`]
/// -> job -> [`AssistResult`]). Changes nothing.
pub const ASSIST: &str = "master.assist";

/// All `master.*` method names.
pub const METHODS: &[&str] = &[
    SUMMARY,
    SET_VOLUME,
    ADD_EFFECT,
    REMOVE_EFFECT,
    MOVE_EFFECT,
    REPLACE_EFFECT,
    SET_FX_BYPASS,
    PLUGIN_PARAMS,
    SET_PLUGIN_PARAM,
    SET_PLUGIN_BYPASS,
    PLUGIN_PRESETS,
    LOAD_PLUGIN_PRESET,
    SAVE_PLUGIN_PRESET,
    SET_SIDECHAIN,
    CLEAR_SIDECHAIN,
    ASSIST,
];

/// Result of `master.summary`.
///
/// The plugin chain is reported here as well as by any future
/// `master.*_effect` method, because a client has to see what is already
/// on the master before it can decide what to add. Identity only,
/// though — for a plugin's parameters and their ranges, read
/// [`PLUGIN_PARAMS`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MasterSummary {
    /// Linear master gain (1.0 = unity).
    pub volume: f32,
    /// The same fader in decibels (0 dB = unity) — what the app stores
    /// and what the mixer's master strip shows.
    pub volume_db: f32,
    /// True while every plugin on the master chain is bypassed.
    pub fx_bypassed: bool,
    /// The master insert chain in processing order. Empty means nothing
    /// is inserted on the master at all.
    #[serde(default)]
    pub plugins: Vec<MasterPluginEntry>,
    /// The master's automation lanes, compact (no points): its volume
    /// lane and lanes on master-chain plugins. Empty (and elided) when
    /// nothing on the master is automated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub automation: Vec<super::automation::LaneSummary>,
}

/// One plugin on the master insert chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MasterPluginEntry {
    /// 0-based position in the master chain; matches processing order.
    pub slot: u32,
    /// The plugin's stable CLAP id, e.g. `"com.resonance.mastering"`.
    pub plugin_id: String,
    /// Human-readable plugin name.
    pub name: String,
}

/// Params for `master.set_volume`: give **exactly one** of `volume`
/// (linear gain) or `volume_db` (decibels). Supplying both, or neither,
/// is `invalid_params` — guessing which one the caller meant is how a
/// master fader ends up 20 dB off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetMasterVolumeParams {
    /// Linear gain (1.0 = unity, 0.0 = silence).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f32>,
    /// Decibels (0 = unity). Same range as a track fader:
    /// [`crate::methods::mixer::VOLUME_DB_MIN`]`..=`[`crate::methods::mixer::VOLUME_DB_MAX`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_db: Option<f32>,
}

/// Params for `master.add_effect`. Appends, exactly like
/// `track.add_effect`: calling it twice with the same id puts two
/// instances on the master.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddEffectParams {
    /// The effect's stable CLAP id, e.g. `"com.resonance.mastering"`.
    /// Instrument plugins are refused — the master chain processes an
    /// already-summed mix and has no notes to play.
    pub plugin_id: String,
}

/// Params for `master.remove_effect`: address the plugin **either** by
/// `slot` **or** by `plugin_id` (+ `occurrence` when the same effect is
/// on the master more than once). Giving both forms, or neither, is
/// `invalid_params` — a guess here removes the wrong processor from the
/// whole mix.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RemoveEffectParams {
    /// 0-based chain position, as reported by `master.summary`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// The effect's CLAP id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based. Defaults to `0`; only
    /// meaningful together with `plugin_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// Params for `master.move_effect`: same addressing as
/// [`RemoveEffectParams`], plus where the effect should end up.
///
/// Order is audible — the chain runs front to back over the finished
/// mix — and on the master it decides whether the chain does its job at
/// all: a limiter holding a ceiling has to be LAST, because anything
/// after it can push the sum back over that ceiling. The master chain
/// has no structural first slot (unlike a track's instrument), so any
/// order is valid.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MoveEffectParams {
    /// 0-based chain position of the effect to move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// CLAP id of the effect to move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    /// Only meaningful together with `plugin_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Destination chain position, 0-based. Past the end clamps to the
    /// last slot; moving an effect to where it already sits is an
    /// accepted no-op.
    pub to_slot: u32,
}

/// Params for `master.replace_effect` — the master twin of
/// [`track::ReplaceEffectParams`](crate::methods::track::ReplaceEffectParams),
/// with the same addressing and the same relocate-vs-swap rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ReplaceEffectParams {
    /// 0-based chain position of the plugin to replace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// CLAP id of the plugin to replace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// CLAP id of the plugin that takes the slot. Naming the plugin
    /// already there relocates a missing one and keeps its settings.
    pub new_plugin_id: String,
}

/// Params for `master.set_fx_bypass`. Idempotent: this SETS the state
/// rather than toggling it, so a client that lost track of the current
/// value cannot flip it the wrong way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetFxBypassParams {
    /// `true` bypasses every plugin on the master chain (the mix passes
    /// through unprocessed); `false` re-engages them.
    pub bypassed: bool,
}

// ---------------------------------------------------------------------------
// Master plugin parameters (ba doc #273 follow-up)
// ---------------------------------------------------------------------------
//
// Without these the master chain is write-only: `master.add_effect`
// loads a limiter and it then sits at its defaults forever, which is why
// a mix driven over the control API could reach the master and still not
// reach a release level. Master is a singleton, so these are the `bus.*`
// methods with `bus_id` dropped — same addressing, same errors, same
// acks.

/// Params for `master.plugin_params` — read the master chain the way
/// `bus.plugin_params` reads a bus's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamsParams {
    /// CLAP id of a plugin on the master. Omitted returns every plugin
    /// on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance to address when the master carries `plugin_id`
    /// more than once; 0-based, defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// Result of `master.plugin_params`.
///
/// The entries are the SAME [`PluginParamsEntry`] shape
/// `track.plugin_params` and `bus.plugin_params` return — including
/// `slot`, the 0-based chain position that IS processing order — so a
/// client reads the master chain with the code it already has. Every
/// entry is `kind: "effect"`: the master has no instrument slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamsView {
    pub plugins: Vec<PluginParamsEntry>,
    pub revision: u64,
}

/// Params for `master.set_plugin_param`.
///
/// The counterpart to `bus.set_plugin_param`, minus the id: this is what
/// turns a limiter on the master from "present at its default patch"
/// into one that actually holds a ceiling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetPluginParamParams {
    /// CLAP id of the plugin on the master. Omitted targets the FIRST
    /// plugin on the chain, which is unambiguous only on a one-effect
    /// master — name it whenever the master carries more than one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// The parameter, by name (case-insensitive), by its numeric id as a
    /// string, or by a first-party plugin's string key (`"lim_on"`).
    /// Names and ids come from [`PLUGIN_PARAMS`].
    pub param: String,
    /// New value — a number, or a choice label from
    /// `master.plugin_params`. Same handling as
    /// [`crate::methods::track::SetPluginParamParams::value`]: a value
    /// that rounds onto an f32-declared bound is accepted and clamped,
    /// anything past that tolerance is rejected with the range, and a
    /// label resolves to the step it names.
    pub value: crate::methods::track::ParamValue,
}

// ---------------------------------------------------------------------------
// Plugin presets on the master chain (ba todo #1333)
// ---------------------------------------------------------------------------
//
// The `bus.*` trio with `bus_id` dropped — master is a singleton. An
// omitted `plugin_id` targets the FIRST plugin on the chain, as
// `master.set_plugin_param` does; there is no instrument slot to default
// to the way `track.plugin_presets` has.

/// Params for `master.plugin_presets`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginPresetsParams {
    /// CLAP id of the plugin on the master. Omitted targets the first
    /// plugin on the chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance to address when the master carries `plugin_id`
    /// more than once; 0-based, defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// Params for `master.load_plugin_preset`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LoadPluginPresetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Preset name, as `master.plugin_presets` reports it.
    pub preset: String,
    /// Which set to take it from. Omitted prefers a user preset, then a
    /// factory one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::methods::plugin_preset::PluginPresetSource>,
}

/// Params for `master.save_plugin_preset`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SavePluginPresetParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Name for the user preset. Saving over an existing user preset
    /// replaces it and needs `overwrite: true`; factory presets are never
    /// touched.
    pub name: String,
    #[serde(default)]
    pub overwrite: bool,
}

// ---------------------------------------------------------------------------
// Sidechain (key) routing onto the master chain (ba doc #275 P4, todo #1311)
// ---------------------------------------------------------------------------

/// Params for `master.set_sidechain` — feed a track's or bus's audio
/// into the key input of a plugin **on the master chain**.
///
/// The mastering use for this is narrow but real: a bus compressor on
/// the master keyed from the kick, so the whole mix breathes with the
/// rhythm rather than with whatever transient happens to be loudest.
/// There is no `master_id` field because there is exactly one master.
///
/// Addressing and the two safety rules match
/// [`crate::methods::track::SetSidechainParams`]: an omitted `plugin_id`
/// targets the first plugin on the master that declares a key port, and
/// a route onto a plugin with no key port is refused rather than stored
/// where it could never be delivered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetSidechainParams {
    /// CLAP id of the plugin on the master to address; omitted targets
    /// the first plugin on the chain that declares a key port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Feed the key from this track's audio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_track_id: Option<crate::ids::TrackId>,
    /// Feed the key from this bus's audio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_bus_id: Option<crate::ids::TrackId>,
    /// A disabled route keeps its configuration but delivers no key, so
    /// the plugin falls back to keying off its own input. Defaults true.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Params for `master.clear_sidechain`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ClearSidechainParams {
    /// Omitted targets the same plugin `master.set_sidechain` would: the
    /// first on the master with a key port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

fn default_true() -> bool {
    true
}

/// Params for `master.set_plugin_bypass` — bypass ONE slot in the chain
/// (ba doc #275 finding X3, todo #1305).
///
/// Distinct from `master.set_fx_bypass`, which mutes the whole chain at
/// once. The two are independent: a chain-bypassed master chain still remembers
/// which of its slots were individually bypassed, so re-engaging the
/// chain restores the mix rather than turning everything on.
///
/// The plugin is addressed exactly as `master.set_plugin_param` addresses
/// it. Idempotent: this SETS the state rather than toggling, so a retried
/// request cannot flip a slot back on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetPluginBypassParams {
    /// CLAP id of the plugin in this chain; omitted targets the first slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance to address when the chain carries `plugin_id` more
    /// than once; 0-based, defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// `true` takes the plugin out of the signal path; `false` puts it
    /// back. The engine crossfades over a few milliseconds rather than
    /// switching, so a toggle mid-playback does not click.
    pub bypassed: bool,
}

// ---------------------------------------------------------------------------
// The mastering assistant (warmth-width-depth.md §7.4)
// ---------------------------------------------------------------------------

/// The CLAP id of the plugin whose params `master.assist` suggests.
pub const MASTERING_PLUGIN_ID: &str = "com.resonance.mastering";

/// What `master.assist` compares the master against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum AssistMode {
    /// A built-in genre target band (`genre` required).
    Genre,
    /// A reference track from the media pool (`pool_asset_id` required).
    Reference,
}

/// A built-in genre target, the same five the plugin's assistant panel
/// offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum AssistGenre {
    Rock,
    Indie,
    Acoustic,
    Jazz,
    Pop,
}

/// Params for `master.assist`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AssistParams {
    /// `"genre"` or `"reference"`.
    pub mode: AssistMode,
    /// The genre target. Required with `mode: "genre"`, refused otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genre: Option<AssistGenre>,
    /// The reference track, a `pool.list` asset id. Required with
    /// `mode: "reference"`, refused otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_asset_id: Option<crate::ids::AssetId>,
    /// The part of the song to analyse; defaults to the whole song and is
    /// clamped to it, exactly as on `meter.measure`. Pick the loudest
    /// representative section (a chorus) when the song has quiet parts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<crate::methods::render::RangeSpec>,
}

/// The target a `master.assist` result was computed against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AssistTargetInfo {
    pub mode: AssistMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genre: Option<AssistGenre>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool_asset_id: Option<crate::ids::AssetId>,
    /// Display name: the genre, or the reference's name.
    pub label: String,
    /// The loudness the suggestions aim at, LUFS: the genre's target, or
    /// the reference's own integrated loudness.
    pub target_lufs: f64,
}

/// The master as the assistant measured it (the same figures
/// `meter.measure` reports, over the same range).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AssistMeasured {
    /// `null` for silence.
    pub lufs_integrated: Option<f64>,
    pub true_peak_db: f64,
    /// Peak-to-RMS over the whole range, dB.
    pub crest_db: f64,
    /// L/R correlation over the whole range.
    pub correlation: f64,
    pub measured_seconds: f64,
}

/// One param write of a suggestion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AssistParamValue {
    /// The mastering plugin's string param key (`"lim_ceiling"`,
    /// `"tone_b0_gain"`, ...), as `master.plugin_params` lists it and
    /// `master.set_plugin_param` accepts it.
    pub key: String,
    /// Plain value: a bool as 0/1, a choice as its index.
    pub value: f64,
}

/// The assistant's suggestion for one stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AssistSuggestion {
    /// `input_trim`, `tonal_low_shelf`, `tonal_high_shelf`, `glue`,
    /// `imager`, `limiter`, `target_lufs` or `diagnostic`.
    pub stage: String,
    /// Why, with the number that justified it.
    pub rationale: Vec<String>,
    /// Exactly the writes the plugin's own Apply would make for this
    /// stage. Empty means the stage needs no change (the rationale says
    /// why); `diagnostic` never carries any.
    pub params: Vec<AssistParamValue>,
}

/// One ISO 1/3-octave band of the master against the target band, after
/// aligning the master's midrange (400 Hz-2.5 kHz) to the target's — a
/// comparison of spectral SHAPE, never of level.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AssistBandDeviation {
    /// Band centre, Hz.
    pub hz: f64,
    /// Lowest on-target level, dB (relative).
    pub lo_db: f64,
    /// Highest on-target level, dB (relative).
    pub hi_db: f64,
    /// The master here, aligned, dB (relative).
    pub measured_db: f64,
    /// Distance outside the band: positive = above it (too much), negative
    /// = below it (too little), 0 = inside it.
    pub deviation_db: f64,
}

/// Job payload once a `master.assist` job completes. Nothing was applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AssistResult {
    pub target: AssistTargetInfo,
    /// Always `com.resonance.mastering`: every key below is its param.
    pub plugin_id: String,
    /// Chain slot of the first `com.resonance.mastering` on the master, or
    /// `null` when there is none — add it with `master.add_effect` before
    /// setting any of the params.
    pub master_slot: Option<u32>,
    /// What the analysis read: the master output as it is NOW, after the
    /// master chain (including any mastering plugin already on it).
    pub measured: AssistMeasured,
    /// Stage by stage, in the order the assistant decides them.
    pub suggestions: Vec<AssistSuggestion>,
    /// 31 bands, 20 Hz first.
    pub deviations: Vec<AssistBandDeviation>,
}
