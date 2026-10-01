//! `track.*` — track lifecycle and per-track plugins.
//!
//! Volume/pan/mute/solo live under `mixer.*`. Simple mutations return
//! [`crate::common::MutationAck`].

use crate::common::{TrackKind, TrackOutput};
use crate::ids::{SendId, TrackId};
use crate::methods::plugin_preset::PluginPresetSource;
use serde::{Deserialize, Serialize};

/// `track.add` — add a track ([`AddParams`] -> [`AddResult`]).
pub const ADD: &str = "track.add";
/// `track.rename` — rename a track ([`RenameParams`] -> `MutationAck`).
///
/// Generated section clips embed the track's name (`"<section> ·
/// <track>"`), so renaming re-derives them — but each keeps the clip id
/// of the (section, placement, track) slot it belongs to, so cached ids
/// stay valid (ba doc #275 P1.7).
pub const RENAME: &str = "track.rename";
/// `track.delete` — delete a track; destructive, requires
/// `"confirm": true` ([`DeleteParams`] -> `MutationAck`).
pub const DELETE: &str = "track.delete";
/// `track.add_instrument` — set a built-in instrument by stable plugin
/// id ([`AddPluginParams`] -> [`AddPluginResult`]).
///
/// SET semantics despite the name: a track that already has an
/// instrument gets it swapped in place (the old one's settings go with
/// it, like `track.replace_effect`), never a second one appended; naming
/// the instrument that is already loaded is a no-op. So there is only
/// ever one instrument, and the reply's `occurrence` is always 0.
pub const ADD_INSTRUMENT: &str = "track.add_instrument";
/// `track.add_effect` — append a built-in effect to the insert chain
/// ([`AddPluginParams`] -> [`AddPluginResult`]). Instrument plugins are
/// rejected — an effect slot has no notes to play; use
/// `track.add_instrument`.
pub const ADD_EFFECT: &str = "track.add_effect";
/// `track.remove_effect` — take an effect off the insert chain
/// ([`RemoveEffectParams`] -> `MutationAck`).
pub const REMOVE_EFFECT: &str = "track.remove_effect";
/// `track.move_effect` — reorder the insert chain
/// ([`MoveEffectParams`] -> `MutationAck`).
pub const MOVE_EFFECT: &str = "track.move_effect";
/// `track.replace_effect` — put a different plugin in a chain slot,
/// keeping its position ([`ReplaceEffectParams`] ->
/// [`ReplaceEffectResult`]).
pub const REPLACE_EFFECT: &str = "track.replace_effect";
/// `track.set_output` — route a track to master or into a bus
/// ([`SetOutputParams`] -> `MutationAck`).
pub const SET_OUTPUT: &str = "track.set_output";
/// `track.add_send` — tap a track into a return bus
/// ([`AddSendParams`] -> [`AddSendResult`]).
pub const ADD_SEND: &str = "track.add_send";
/// `track.set_send` — edit an existing send ([`SetSendParams`]).
pub const SET_SEND: &str = "track.set_send";
/// `track.remove_send` — delete a send ([`RemoveSendParams`]).
pub const REMOVE_SEND: &str = "track.remove_send";
/// `track.plugin_params` — a track's plugins and their parameters,
/// read-only ([`PluginParamsParams`] -> [`PluginParamsView`]).
pub const PLUGIN_PARAMS: &str = "track.plugin_params";
/// `track.set_plugin_param` — set one plugin parameter
/// ([`SetPluginParamParams`] -> `MutationAck`).
pub const SET_PLUGIN_PARAM: &str = "track.set_plugin_param";
/// `track.set_fx_bypass` — bypass/unbypass a track's whole insert chain
/// ([`SetFxBypassParams`] -> `MutationAck`).
///
/// Closes the audit's one INVERSE parity gap (ba doc #276 section 0): the
/// GUI has had this button since forever and `bus`/`master` have had the
/// method, while `track` had neither wire nor tool.
pub const SET_FX_BYPASS: &str = "track.set_fx_bypass";
/// `track.set_plugin_bypass` — bypass/unbypass ONE slot in a track's
/// chain ([`SetPluginBypassParams`] -> `MutationAck`).
pub const SET_PLUGIN_BYPASS: &str = "track.set_plugin_bypass";
/// `track.set_sidechain` — route another track's or bus's audio into a
/// plugin's external sidechain key ([`SetSidechainParams`] ->
/// `MutationAck`).
/// `track.plugin_presets` — a plugin's factory and user presets, and
/// which one is loaded ([`PluginPresetsParams`] ->
/// [`PluginPresetsView`](crate::methods::plugin_preset::PluginPresetsView)).
/// Read-only.
pub const PLUGIN_PRESETS: &str = "track.plugin_presets";
/// `track.load_plugin_preset` — recall a preset onto a plugin
/// ([`LoadPluginPresetParams`] -> [`MutationAck`](crate::common::MutationAck)).
pub const LOAD_PLUGIN_PRESET: &str = "track.load_plugin_preset";
/// `track.save_plugin_preset` — save a plugin's current settings as a
/// named user preset ([`SavePluginPresetParams`] ->
/// [`MutationAck`](crate::common::MutationAck)).
pub const SAVE_PLUGIN_PRESET: &str = "track.save_plugin_preset";
pub const SET_SIDECHAIN: &str = "track.set_sidechain";
/// `track.clear_sidechain` — remove a plugin's key route
/// ([`ClearSidechainParams`] -> `MutationAck`).
pub const CLEAR_SIDECHAIN: &str = "track.clear_sidechain";
/// `track.save_preset` — capture a dialled-in track as a reusable
/// preset ([`SavePresetParams`] -> `MutationAck`).
pub const SAVE_PRESET: &str = "track.save_preset";
/// `track.presets` — the preset library, built-in and user-saved
/// (no params -> [`PresetsView`]).
pub const PRESETS: &str = "track.presets";
/// `track.apply_preset` — stamp out a new track from a preset
/// ([`ApplyPresetParams`] -> [`AddResult`]).
pub const APPLY_PRESET: &str = "track.apply_preset";

/// All `track.*` method names.
pub const METHODS: &[&str] = &[
    ADD,
    RENAME,
    DELETE,
    ADD_INSTRUMENT,
    ADD_EFFECT,
    REMOVE_EFFECT,
    MOVE_EFFECT,
    REPLACE_EFFECT,
    SET_OUTPUT,
    ADD_SEND,
    SET_SEND,
    REMOVE_SEND,
    PLUGIN_PARAMS,
    SET_PLUGIN_PARAM,
    SET_FX_BYPASS,
    SET_PLUGIN_BYPASS,
    PLUGIN_PRESETS,
    LOAD_PLUGIN_PRESET,
    SAVE_PLUGIN_PRESET,
    SET_SIDECHAIN,
    CLEAR_SIDECHAIN,
    SAVE_PRESET,
    PRESETS,
    APPLY_PRESET,
];

/// Params for `track.set_sidechain`.
///
/// The plugin is addressed as `track.set_plugin_param` addresses it —
/// `track_id` plus an optional `plugin_id` / `occurrence` — and the key
/// source is named by `source_track_id` **or** `source_bus_id`, exactly
/// one of which must be given.
///
/// Two rules keep a route from being accepted and then ignored (ba doc
/// #275 P0):
///
/// - An omitted `plugin_id` targets the first plugin on the track that
///   declares a key port, NOT the instrument at slot 0 (a synth has no
///   key input, so that default routed every unqualified call onto the
///   one plugin guaranteed to drop it).
/// - A route onto a plugin with no key port is **refused**, naming the
///   plugins on that track that accept one.
///
/// Any track or bus can be the source, including a sub-track (one tap of
/// a multi-output instrument, e.g. the kick of a drum kit) — that is
/// usually the only address a single kit piece has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetSidechainParams {
    /// The track hosting the plugin whose key is being routed.
    pub track_id: TrackId,
    /// CLAP id of the plugin to address; omitted targets the first
    /// plugin on the track that declares a key port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Feed the key from this track's audio. A sub-track (drum tap) is a
    /// valid source and is captured post-FX, pre-fader — so a kick can
    /// key a ducker while its own fader sits at -inf.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_track_id: Option<TrackId>,
    /// Feed the key from this bus's audio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_bus_id: Option<TrackId>,
    /// A disabled route keeps its configuration but delivers no key, so
    /// the plugin falls back to keying off its own input. Defaults true.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Params for `track.clear_sidechain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ClearSidechainParams {
    pub track_id: TrackId,
    /// Omitted targets the same plugin `track.set_sidechain` would: the
    /// first on the track with a key port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// Params for `track.add`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddParams {
    pub kind: TrackKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Result of `track.add`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddResult {
    pub track_id: TrackId,
    pub revision: u64,
}

/// Params for `track.rename`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RenameParams {
    pub track_id: TrackId,
    pub name: String,
}

/// Params for `track.delete`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct DeleteParams {
    pub track_id: TrackId,
    /// Required (`true`); the error otherwise summarizes what would be lost.
    #[serde(default)]
    pub confirm: bool,
}

/// Params for `track.add_instrument` / `track.add_effect`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddPluginParams {
    pub track_id: TrackId,
    /// Stable plugin id from `plugins.catalog`, e.g.
    /// `"com.resonance.wavetable"`.
    pub plugin_id: String,
    /// Load this preset onto the new plugin as soon as it exists: a
    /// preset id or name, as `*.plugin_presets` / `presets.search` report
    /// it. Checked before anything is added; the add and the load are one
    /// undo step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

/// Result of `track.add_effect` / `track.add_instrument`: a handle on
/// the plugin that was just created.
///
/// `(plugin_id, occurrence)` is exactly what `track.set_plugin_param`,
/// `track.plugin_params` and `track.remove_effect` take, so the caller
/// can configure or undo what it just added without re-reading the chain
/// and guessing which instance is the new one. Adding the same plugin
/// twice gives `occurrence` 0 then 1.
///
/// The plugin is visible to `track.plugin_params` in the same update
/// cycle as this reply — no engine round-trip is needed to see that it
/// exists. Its PARAMETER LIST, however, arrives with the engine's echo,
/// so a `track.plugin_params` read in the same tick may show an empty
/// `params` array; `track.set_plugin_param` says so explicitly rather
/// than claiming the plugin has no such parameter.
///
/// Additive: the previous reply was an object carrying only `revision`,
/// so a client reading just that field keeps working.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddPluginResult {
    /// The CLAP id that was added — echoed back so a reply is
    /// self-describing.
    pub plugin_id: String,
    /// 0-based index of this instance among the track's copies of
    /// `plugin_id`. Pass as `occurrence` to address it.
    pub occurrence: u32,
    /// 0-based position in the track's insert chain, the instrument
    /// included, at the moment of the add. Slots renumber when anything
    /// is removed or moved, so `(plugin_id, occurrence)` is the stabler
    /// handle.
    pub slot: u32,
    pub revision: u64,
}

/// Plugin roles, lowercase on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    Instrument,
    Effect,
}

// ---------------------------------------------------------------------------
// Plugin parameters (ba doc #272 V-3)
// ---------------------------------------------------------------------------

/// Params for `track.plugin_params`.
///
/// A plugin is addressed by the CLAP id `song.tracks` already reports —
/// its `instrument` field or an entry of its `effects` array — so no
/// extra identifier has to be discovered first. A track carrying the
/// same plugin twice disambiguates with `occurrence`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamsParams {
    pub track_id: TrackId,
    /// CLAP id of the plugin on this track, e.g.
    /// `"com.resonance.wavetable"`. Omitted returns every plugin on the
    /// track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance to address when the track carries `plugin_id`
    /// more than once; 0-based, defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// Result of `track.plugin_params`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamsView {
    pub track_id: TrackId,
    pub plugins: Vec<PluginParamsEntry>,
    pub revision: u64,
}

/// One plugin on the track, with its parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamsEntry {
    /// CLAP id — what [`PluginParamsParams::plugin_id`] and
    /// [`SetPluginParamParams::plugin_id`] take.
    pub plugin_id: String,
    pub name: String,
    /// 0-based position in the track's insert chain, the instrument
    /// included, so slot order IS processing order. `slot` and
    /// `(plugin_id, occurrence)` are two ways to address the same
    /// plugin: `slot` says *where* it sits, the pair says *which* copy
    /// of a repeated effect it is. Slots renumber when a plugin is
    /// removed, so re-read this view between removals.
    #[serde(default)]
    pub slot: u32,
    /// 0-based position of this plugin among the track's instances of
    /// the same `plugin_id` — pass as `occurrence` to address it.
    pub occurrence: u32,
    pub kind: PluginKind,
    /// Whether this ONE slot is bypassed (ba todo #1305).
    ///
    /// Independent of the chain's own bypass: `track.set_fx_bypass` mutes
    /// every insert at once, this reports the per-slot flag that
    /// `track.set_plugin_bypass` sets. A chain-bypassed track therefore
    /// still reports its slots' individual states, which is what makes
    /// un-bypassing the chain predictable.
    #[serde(default)]
    pub bypassed: bool,
    /// Whether there is actually a plugin behind this slot on the
    /// machine the app is running on (ba doc #275 P5, todo #1309).
    ///
    /// Defaults to [`PluginSlotStatus::Loaded`], which is what every
    /// slot was assumed to be before this field existed — and the
    /// assumption that made a missing plugin invisible over the wire: a
    /// dead slot reported an empty `params` list, indistinguishable
    /// from a plugin that genuinely has no parameters, and every
    /// `set_plugin_param` against it succeeded and changed no sound.
    #[serde(default)]
    pub status: PluginSlotStatus,
    /// Why the plugin could not be loaded, in the plugin loader's own
    /// words. Present only when `status` is `missing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
    /// The plugin's parameters. **Empty when `status` is `missing`** —
    /// there is no instance to ask, and the values the project saved are
    /// held opaquely against the slot until the plugin comes back
    /// (ba todo #1308), not exposed as live parameters that could be
    /// read as current.
    pub params: Vec<PluginParamView>,
}

/// Whether a chain slot has a live plugin behind it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum PluginSlotStatus {
    /// Instantiated and processing audio.
    #[default]
    Loaded,
    /// The project asked for a plugin this machine hasn't got, or the
    /// `.clap` refused to instantiate. The slot keeps its position in
    /// the chain and keeps the settings the project saved for it, so
    /// installing the plugin (then `plugins.rescan`) or calling
    /// `replace_effect` recovers it. Removing the slot is what discards
    /// those settings.
    Missing,
}

/// Params for `track.set_output` — where a track's post-fader audio
/// goes.
///
/// `song.summary` / `song.tracks` report the current destination as the
/// same [`TrackOutput`] shape, so a read and a write use one vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetOutputParams {
    pub track_id: TrackId,
    /// `"master"` to sum directly into the master output, or
    /// `{"bus_id": N}` to route through a group bus first.
    pub output: TrackOutput,
}

// ---------------------------------------------------------------------------
// Aux sends (ba doc #273, todo #1229)
// ---------------------------------------------------------------------------

/// Lowest send level the engine accepts, in dB (it clamps to this).
pub const SEND_LEVEL_DB_MIN: f32 = -120.0;
/// Highest send level the engine accepts, in dB.
pub const SEND_LEVEL_DB_MAX: f32 = 24.0;

/// Params for `track.add_send` — an extra tap from a track into a return
/// bus, independent of where the track's main output goes.
///
/// This is how several tracks share ONE reverb (or delay): each sends
/// some of its signal to a common return, which puts them in the same
/// room. A reverb inserted per track puts every instrument in a
/// different building and costs far more CPU.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddSendParams {
    pub track_id: TrackId,
    /// The destination bus (from `bus.create`, or a `kind: "bus"` entry
    /// in `song.summary`). It is flagged as a RETURN bus automatically.
    pub to_bus: TrackId,
    /// Send gain in dB; `0` taps the source at unity. Defaults to `0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_db: Option<f32>,
    /// `true` taps before the track's own fader (the send level then
    /// ignores the fader), `false` — the default, and what you almost
    /// always want for reverb — taps after it, so moving the track down
    /// takes its reverb with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_fader: Option<bool>,
}

/// Result of `track.add_send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AddSendResult {
    /// Address this send in `track.set_send` / `track.remove_send`; it
    /// is also reported in `song.tracks`.
    pub send_id: SendId,
    pub revision: u64,
}

/// Params for `track.set_send`: change one or more properties of an
/// existing send. Omitted fields keep their current value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetSendParams {
    pub send_id: SendId,
    /// New send gain in dB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_db: Option<f32>,
    /// Move the tap before (`true`) or after (`false`) the fader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_fader: Option<bool>,
    /// Silence the send without losing its routing and level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Re-route the send into a different return bus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_bus: Option<TrackId>,
}

/// Params for `track.remove_send`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RemoveSendParams {
    pub send_id: SendId,
}

/// Params for `track.remove_effect`: address the plugin **either** by
/// `slot` **or** by `plugin_id` (+ `occurrence`). Both forms together,
/// or neither, is `invalid_params` — with `track.add_effect` appending a
/// fresh instance on every call, guessing which copy was meant is how
/// the wrong processor comes off.
///
/// The track's INSTRUMENT cannot be removed this way; replace it with
/// `track.add_instrument` instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct RemoveEffectParams {
    pub track_id: TrackId,
    /// 0-based chain position, as reported by
    /// [`PluginParamsEntry::slot`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// CLAP id of the effect to remove.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    /// Only meaningful together with `plugin_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
}

/// Params for `track.move_effect`: change WHERE an effect sits in the
/// chain, addressed exactly like [`RemoveEffectParams`] — **either**
/// `slot` **or** `plugin_id` (+ `occurrence`), never both and never
/// neither.
///
/// Order is audible. A compressor after an EQ reacts to the EQ'd signal;
/// the same compressor before it does not, and the two are different
/// sounds, not different spellings of one. A limiter belongs last,
/// because anything after it can push the signal back over the ceiling
/// it was there to hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct MoveEffectParams {
    pub track_id: TrackId,
    /// 0-based chain position of the effect to move, as reported by
    /// [`PluginParamsEntry::slot`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// CLAP id of the effect to move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    /// Only meaningful together with `plugin_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Destination chain position, 0-based. A value past the end of the
    /// chain clamps to the last slot rather than failing. Moving an
    /// effect to the slot it already occupies is an accepted no-op.
    pub to_slot: u32,
}

// ---------------------------------------------------------------------------
// replace_effect (ba doc #275 P5, todo #1309)
// ---------------------------------------------------------------------------

/// Params for `track.replace_effect`: swap the plugin in one chain slot
/// for another **without moving the slot**. The effect to replace is
/// addressed exactly as [`RemoveEffectParams`] addresses it — either
/// `slot` or `plugin_id` (+ `occurrence`), never both and never neither.
///
/// Why this exists rather than remove-then-add: `add_effect` only ever
/// APPENDS, so the pair puts the replacement at the END of the chain,
/// and chain order is audible — a compressor that was before the reverb
/// ends up after it. It also cannot be used at all on a plugin that is
/// MISSING, because removing that slot is what throws away the settings
/// the project saved for it (ba todo #1308).
///
/// Unlike `remove_effect` and `move_effect`, this accepts the track's
/// INSTRUMENT slot: a synth that will not load is the worst version of
/// this problem, not an exempt one, and swapping it leaves the track
/// with a sound source instead of none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ReplaceEffectParams {
    pub track_id: TrackId,
    /// 0-based chain position of the plugin to replace, as reported by
    /// [`PluginParamsEntry::slot`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u32>,
    /// CLAP id of the plugin to replace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance of `plugin_id`, 0-based; defaults to the first.
    /// Only meaningful together with `plugin_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// CLAP id of the plugin that takes the slot, from
    /// `plugins.catalog`.
    ///
    /// Naming the plugin that is ALREADY in the slot is meaningful and
    /// not a mistake: it relocates a missing plugin to wherever the
    /// catalog now finds it, keeping the instance and therefore the
    /// settings saved against it. That is the "I moved my plugin folder,
    /// then ran `plugins.rescan`" recovery, and it is the only form of
    /// replace that brings the original sound back.
    pub new_plugin_id: String,
}

/// Result of `*.replace_effect`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ReplaceEffectResult {
    /// The chain position the replacement occupies — the same one the
    /// plugin it replaced had. Reported so a caller can confirm the
    /// position survived without re-reading `plugin_params`.
    pub slot: u32,
    /// The plugin now in that slot.
    pub plugin_id: String,
    pub outcome: ReplaceOutcome,
    pub revision: u64,
}

/// What a `replace_effect` turned out to mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReplaceOutcome {
    /// The slot's own plugin was re-instantiated from the path the
    /// catalog now has for it. Its saved settings come back with it.
    Relocated,
    /// A different plugin took the position. The outgoing plugin's saved
    /// settings went with it — they could not mean anything to a plugin
    /// that never had them.
    Swapped,
    /// The slot already carries this plugin and it is loaded. Nothing
    /// was changed; reported rather than silently acked so a caller can
    /// tell "already the case" from "did something".
    AlreadyLoaded,
}

/// One plugin parameter — its number, and what that number means.
///
/// [`value`](Self::value) alone is rarely enough to work with: `2.0` on
/// a `0..=4` range is a filter type, a waveform or an oversampling
/// factor and nothing in the number says which. So every parameter also
/// reports the plugin's own rendering of it ([`text`](Self::text)), the
/// [`unit`](Self::unit) that rendering carries, the
/// [`module`](Self::module) it is grouped under, whether it is
/// [`stepped`](Self::stepped) and, when it names its values, the
/// [`choices`](Self::choices) `set_plugin_param` will accept by name
/// (ba todo #1290, finding X8).
///
/// `min`/`max`/`default`/`value` are f64 on the wire, but plugins
/// declare their ranges in **f32**, so a bound with no exact binary
/// representation arrives with a long tail of digits: a minimum the
/// plugin calls `0.1` is reported as `0.10000000149011612`. That is not
/// a rounding bug to work around — see [`SetPluginParamParams::value`]
/// for what `track.set_plugin_param` accepts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginParamView {
    /// Stable CLAP parameter id. Either this (as a decimal string) or
    /// `name` addresses the parameter in `track.set_plugin_param`.
    pub id: u32,
    pub name: String,
    pub value: f64,
    /// Lowest accepted value, as an f64 rendering of the plugin's f32
    /// declaration — so it may read `0.10000000149011612` where the
    /// plugin means `0.1`. Sending the tidy decimal is fine: it rounds
    /// onto the bound and is clamped, not rejected.
    pub min: f64,
    /// Highest accepted value, with the same f32-widening caveat as
    /// [`min`](Self::min).
    pub max: f64,
    pub default: f64,
    /// What the plugin CALLS the current value: `"40 %"`, `"-6.0 dB"`,
    /// `"Low-pass"` (ba todo #1290).
    ///
    /// This is the parameter as its own editor shows it, and it is the
    /// field to read before deciding whether a number means anything.
    /// `value: 2.0` on a `0..=4` range says nothing; `text: "Band-pass"`
    /// says what the plugin is doing. Empty when the plugin declares no
    /// formatting, in which case the number is all there is.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// The unit alone — `"dB"`, `"%"`, `"Hz"`, `"ms"` — for a reader
    /// that wants to label the number rather than reprint the plugin's
    /// whole rendering. Empty for a unitless parameter and for one whose
    /// display is a name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
    /// The parameter's group inside the plugin, `/`-separated
    /// (`"Multiband/Low"`); empty when it is ungrouped. On a plugin with
    /// dozens of parameters this is which stage a parameter belongs to —
    /// the difference between "the low band's threshold" and one of six
    /// parameters called Threshold.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub module: String,
    /// True when the parameter moves in whole numbers — a choice, a
    /// count, a switch. Send `3`, never `2.7`: a fractional value on a
    /// stepped parameter rounds, so it is a request the plugin cannot
    /// honour as written.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stepped: bool,
    /// The names of a stepped parameter's values, in order from
    /// [`min`](Self::min) — so `choices[0]` is the value at the minimum
    /// and `choices[n]` the value at `min + n`.
    ///
    /// `set_plugin_param` accepts any of these as `value` directly
    /// (case-insensitive), which is how a caller sends `"Low-pass"`
    /// without having to work out that it is `3`. Empty for a continuous
    /// parameter, and for a stepped one whose steps are plain numbers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    /// True when the plugin asks that this parameter not be shown to a
    /// user. It is still readable, writable and saved — it is reported
    /// so a client can leave it out of a listing the way the app's own
    /// panels do, rather than present it as an ordinary control.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// True when only the plugin writes this parameter (CLAP
    /// `IS_READONLY`): an output such as a load progress or a meter. Read
    /// it; `set_plugin_param` refuses to write it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    /// False when the plugin offers no automation lane for this parameter
    /// (CLAP `IS_AUTOMATABLE` unset) — a control whose every change is
    /// heavy work, like a drum kit selector, or a read-only output. It can
    /// still be set; `automation.*` refuses to put a lane on it. Omitted
    /// from the wire when true.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub automatable: bool,
}

fn is_true(value: &bool) -> bool {
    *value
}

/// The new value for `set_plugin_param`: a number, or the NAME of a
/// stepped parameter's choice (ba todo #1290).
///
/// A choice parameter's number is an implementation detail of the
/// plugin's enum. `"Low-pass"` is what the parameter means, it is what
/// [`PluginParamView::text`] reports back, and requiring a caller to
/// translate it into `3` — then silently doing the wrong thing when the
/// plugin's order differs from the guess — is exactly the round trip
/// this API exists to remove.
///
/// Labels match case-insensitively and come from
/// [`PluginParamView::choices`]. A label sent to a parameter that has no
/// choices, or one that matches none of them, is rejected with the
/// accepted labels rather than parsed as a number.
///
/// A string that parses as a number is taken as that NUMBER, before any
/// label is considered — clients that stringify every value are common,
/// and `"0.75"` from one of them means three quarters. The ambiguity is
/// therefore known and resolved number-first: a plugin whose choice
/// labels were themselves numeric (`"0.5"`, `"2"`) could not be reached
/// by name. Nothing in the fleet has such a table — divisions, filter
/// types, LFO shapes and cabinet names all fail to parse as numbers —
/// and such a parameter would still be settable by its index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum ParamValue {
    /// A plain value on the parameter's `min..=max`.
    Number(f64),
    /// A choice label from [`PluginParamView::choices`].
    Label(String),
}

impl ParamValue {
    /// The number this value denotes for `param`, or an explanation of
    /// why it denotes none.
    ///
    /// The whole resolution lives here so `track`, `bus` and `master`
    /// answer a label — and refuse a wrong one — identically; three
    /// copies of a lookup is how they would stop agreeing.
    pub fn resolve(&self, param: &PluginParamView) -> Result<f64, ParamValueError> {
        let text = match self {
            ParamValue::Number(v) => return Ok(*v),
            ParamValue::Label(text) => text.trim(),
        };
        // A number that arrived as a string is still a number: clients
        // that stringify everything should not be told their value is
        // an unknown choice.
        if let Ok(v) = text.parse::<f64>() {
            return Ok(v);
        }
        if let Some(index) = param
            .choices
            .iter()
            .position(|label| label.eq_ignore_ascii_case(text))
        {
            return Ok(param.min + index as f64);
        }
        Err(ParamValueError {
            wanted: text.to_string(),
            choices: param.choices.clone(),
        })
    }
}

/// A `value` that named a choice the parameter does not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamValueError {
    /// What the caller asked for.
    pub wanted: String,
    /// The labels the parameter does accept — empty when it is not a
    /// choice parameter at all.
    pub choices: Vec<String>,
}

impl std::fmt::Display for ParamValueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.choices.is_empty() {
            write!(
                f,
                "{:?} is not a number, and this parameter names no choices — send a value \
                 within its min..=max",
                self.wanted
            )
        } else {
            write!(
                f,
                "{:?} is not one of this parameter's choices: [{}]",
                self.wanted,
                self.choices.join(", ")
            )
        }
    }
}

/// Params for `track.set_plugin_param`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetPluginParamParams {
    pub track_id: TrackId,
    /// CLAP id of the plugin to address; omitted targets the track's
    /// instrument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// The parameter, by name (case-insensitive), by its numeric id as a
    /// string, or — on a first-party `com.resonance.*` plugin — by its
    /// string key (`"lim_on"`), which survives a display rename. Names
    /// and ids come from `track.plugin_params`.
    pub param: String,
    /// New value: a number, or — for a stepped parameter that names its
    /// values — one of the labels `track.plugin_params` reports in
    /// `choices`, matched case-insensitively. `"Low-pass"` and `3` are
    /// the same request; the label is the one that survives the plugin
    /// reordering its enum.
    ///
    /// On a parameter with no `choices`, a text is asked of the plugin
    /// itself (CLAP `text_to_value`): what the parameter displays as its
    /// `text` is accepted back — `"-6 dB"` on a gain, or an amp model's
    /// name (`"Friedman BE-100"`) on Resonance Amp's `Model Select`.
    ///
    /// A number must lie within the parameter's `min..=max`, which an
    /// out-of-range request reports back.
    ///
    /// Those bounds are f64 renderings of **f32** plugin declarations
    /// ([`PluginParamView::min`]), so a value that rounds onto a bound —
    /// `0.1` against a reported minimum of `0.10000000149011612` — is
    /// ACCEPTED and clamped to the true bound, never rejected for being
    /// a few ULPs out. Anything past that tolerance is still refused
    /// rather than clamped, so a genuinely wrong number (`-60` on a
    /// `0..1` parameter) comes back as an error instead of silently
    /// becoming something else.
    pub value: ParamValue,
}

// ---------------------------------------------------------------------------
// Plugin presets (ba todo #1333)
// ---------------------------------------------------------------------------

/// Params for `track.plugin_presets`.
///
/// The plugin is addressed exactly as `track.plugin_params` addresses it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PluginPresetsParams {
    pub track_id: TrackId,
    /// CLAP id of the plugin on this track; omitted targets the track's
    /// instrument.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// Which instance to address when the track carries `plugin_id` more
    /// than once; 0-based, defaults to the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Search, facet filters, sort and paging (plugin-preset-library.md
    /// §12.1). Omitted lists everything in bank order.
    #[serde(flatten)]
    pub filter: super::plugin_preset::PresetFilter,
}

/// Params for `track.load_plugin_preset`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct LoadPluginPresetParams {
    pub track_id: TrackId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Preset name, as `track.plugin_presets` reports it.
    pub preset: String,
    /// Which set to take it from. Omitted prefers a user preset, then a
    /// factory one — so a user preset deliberately shadowing a factory
    /// name wins unless the caller asks for the factory original.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PluginPresetSource>,
    /// The preset's stable id (from the list); wins over `preset` (the
    /// name), which may then be empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_id: Option<String>,
    /// Load the preset's sound-bearing extra state too (a NAM model, an
    /// IR, user wavetables). Default true; false recalls the params only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<bool>,
}

/// Params for `track.save_plugin_preset`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SavePluginPresetParams {
    pub track_id: TrackId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence: Option<u32>,
    /// Name for the user preset. Saving over an existing user preset
    /// replaces it and needs `overwrite: true`, per the control API's
    /// destructive-operation convention; factory presets are never
    /// touched.
    pub name: String,
    #[serde(default)]
    pub overwrite: bool,
    /// Metadata for the preset (category, instrument, genres, character,
    /// tags, author, description), so it is findable next session. A new
    /// preset otherwise inherits the loaded preset's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<super::plugin_preset::PresetMetaInput>,
    /// Star it for this user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    /// Update this user preset (by id) in place instead of addressing one
    /// by `name`; it keeps its id and may be renamed to `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overwrite_id: Option<String>,
}

// ---------------------------------------------------------------------------
// Track presets (ba todo #1303, finding P1)
// ---------------------------------------------------------------------------
//
// A preset is a track worth keeping: its type, its mixer settings, its
// instrument identity and its whole plugin chain WITH each plugin's
// opaque state, so recalling it restores the sound and not just the
// names. The app has been able to apply one since forever and could not
// create one — the preset menu listed only what someone had hand-written
// as JSON. These three methods are the same capability the GUI's
// "Save as preset..." now has, so a dialled-in track can be captured and
// stamped out again over the wire.

/// Params for `track.save_preset`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SavePresetParams {
    /// The track to capture, exactly as it stands.
    pub track_id: TrackId,
    /// What to call it. Omitted takes the track's own name — which is
    /// usually right, and always visible in `track.presets` afterwards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Required (`true`) to replace a preset that already carries this
    /// name, per the control API's destructive-operation convention: a
    /// preset is a file, and saving over one loses whatever was in it.
    /// Without it, a colliding name is refused and says so.
    #[serde(default)]
    pub overwrite: bool,
}

/// Result of `track.presets`: everything a new track can be stamped from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PresetsView {
    pub presets: Vec<PresetView>,
}

/// One preset in the library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct PresetView {
    /// The name `track.apply_preset` takes, and the name a save
    /// collides with.
    pub name: String,
    /// The kind of track it makes.
    pub kind: TrackKind,
    /// Whether it ships with the app or was saved from a track.
    /// Built-ins carry no plugin chain; a saved one usually does.
    pub builtin: bool,
    /// CLAP ids of the plugins it restores, in chain order — the
    /// instrument first on an instrument preset. Empty for a preset that
    /// only carries mixer settings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<String>,
}

/// Params for `track.apply_preset`.
///
/// Applying a preset CREATES a track from it, exactly as picking it in
/// the app's add-track menu does — it does not overwrite an existing
/// track's chain, and so needs no confirmation. The new track's id comes
/// back in [`AddResult`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ApplyPresetParams {
    /// A name from `track.presets`, matched case-insensitively.
    pub preset: String,
    /// Name for the new track. Omitted takes the preset's name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Params for `track.set_fx_bypass`. Idempotent: this SETS the state
/// rather than toggling it, so a client that lost track of the current
/// value — or that retried a request it never saw the answer to — cannot
/// flip the track's processing back on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetFxBypassParams {
    pub track_id: TrackId,
    /// `true` bypasses every insert on this track (it passes through
    /// unprocessed); `false` re-engages them. Does not touch the
    /// per-slot flags — see [`SetPluginBypassParams`].
    pub bypassed: bool,
}

/// Params for `track.set_plugin_bypass` — bypass ONE slot in the chain
/// (ba doc #275 finding X3, todo #1305).
///
/// Distinct from `track.set_fx_bypass`, which mutes the whole chain at
/// once. The two are independent: a chain-bypassed track still remembers
/// which of its slots were individually bypassed, so re-engaging the
/// chain restores the mix rather than turning everything on.
///
/// The plugin is addressed exactly as `track.set_plugin_param` addresses
/// it. Idempotent: this SETS the state rather than toggling, so a retried
/// request cannot flip a slot back on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SetPluginBypassParams {
    pub track_id: TrackId,
    /// CLAP id of the plugin in this chain; omitted targets the track's instrument.
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
