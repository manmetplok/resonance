//! MIDI Learn & hardware-controller mapping data model (architecture doc #167,
//! epic #21).
//!
//! One definition of the mapping between a hardware control surface (knobs,
//! faders, buttons, transport) and Resonance's mixer/plugin/transport targets,
//! shared by the realtime/control engine (`resonance-audio`), the app
//! (`resonance-app`), project I/O and the controller-map preset file so they all
//! agree on what a binding is and how an incoming MIDI message maps to a
//! normalized target value.
//!
//! Like the automation model (doc #162, [`crate::automation`]) the mapping math
//! lives here as the single source of truth: [`cc_to_norm`] /
//! [`decode_relative`] / [`apply_delta`] / [`takeover_value`] so UI labels and
//! the engine never disagree.
//!
//! Target values are always **normalized** `0.0..=1.0`; how that maps to a real
//! value (dB for volume, `-1..=1` for pan, the plugin's own `min..=max` for a
//! CLAP param) is the target domain's job — the same convention as automation.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::automation::{PluginInstanceId, TrackId};

/// Identifier for a [`MidiBinding`], unique within a [`ControllerMap`] / project.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct BindingId(pub u64);

/// Identifier for a track send, used by [`MidiTarget::SendLevel`]. Mirrors the
/// engine's send addressing; a plain `u64` like the other id newtypes.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct SendId(pub u64);

/// The three common endless-encoder transmit formats. Each encodes a signed
/// per-message delta in a single 7-bit CC value; [`decode_relative`] turns the
/// raw byte into that delta.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelativeEnc {
    /// 7-bit two's complement: `0..=63` ⇒ `+0..=+63`, `64..=127` ⇒ `-64..=-1`.
    TwosComplement,
    /// Sign-and-magnitude: bit 6 (`0x40`) is the sign (set ⇒ increment),
    /// bits 0–5 (`0x3F`) the magnitude. `0x41` ⇒ `+1`, `0x01` ⇒ `-1`.
    SignedBit,
    /// Binary offset, centered on `64`: `delta = raw - 64`. `65` ⇒ `+1`,
    /// `63` ⇒ `-1`.
    BinaryOffset,
}

/// How a CC control's value is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CcMode {
    /// 0–127 maps absolutely onto the binding's `min..=max` sub-range.
    Absolute,
    /// Endless encoder: each message is a signed delta in the given format.
    Relative(RelativeEnc),
}

/// The physical control a binding listens to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ControlSource {
    /// A continuous controller (knob/fader/encoder).
    Cc { channel: u8, cc: u8, mode: CcMode },
    /// A note, used for toggles (mute/solo/loop) and triggers (play/stop/record)
    /// on note-on.
    Note { channel: u8, note: u8 },
}

/// Transport actions a binding can drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransportAction {
    Play,
    Stop,
    Record,
    LoopToggle,
}

/// What a binding controls. Volume/pan map to a continuous range; mute/solo and
/// transport actions are toggles/triggers; `PluginParam` carries only the
/// addressing (its `min..=max` lives in the plugin's `ParamInfo`, applied by the
/// engine — mirrors [`crate::automation::AutomationTarget::PluginParam`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MidiTarget {
    TrackVolume(TrackId),
    TrackPan(TrackId),
    TrackMute(TrackId),
    TrackSolo(TrackId),
    SendLevel { track: TrackId, send: SendId },
    PluginParam {
        instance: PluginInstanceId,
        param_id: u32,
    },
    Transport(TransportAction),
}

/// Soft-takeover strategy: how an incoming hardware value reconciles with the
/// target's current value to avoid parameter jumps when they disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Takeover {
    /// Adopt the incoming value immediately. Default, and the only sane choice
    /// for endless encoders (relative) and note toggles/triggers.
    #[default]
    Jump,
    /// Ignore the control until its value crosses (comes within tolerance of)
    /// the target's current value, then engage.
    Pickup,
    /// Move proportionally toward the incoming value so the two converge,
    /// meeting at the extremes.
    Scale,
}

/// One hardware-control → target mapping.
///
/// `min`/`max` are the normalized `0.0..=1.0` sub-range the control spans (a
/// fader limited to the top half is `0.5..=1.0`); `invert` flips direction.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MidiBinding {
    pub id: BindingId,
    pub source: ControlSource,
    pub target: MidiTarget,
    /// Low end of the normalized sub-range the control spans.
    pub min: f32,
    /// High end of the normalized sub-range the control spans.
    pub max: f32,
    /// Flip the control's direction (raw 0 ⇒ `max`, raw 127 ⇒ `min`).
    pub invert: bool,
    pub takeover: Takeover,
}

impl MidiBinding {
    /// Build a binding spanning the full `0.0..=1.0` range with no inversion and
    /// the default [`Takeover`] — the sensible defaults the app uses when a
    /// control is first learned.
    pub fn new(id: BindingId, source: ControlSource, target: MidiTarget) -> Self {
        Self {
            id,
            source,
            target,
            min: 0.0,
            max: 1.0,
            invert: false,
            takeover: Takeover::default(),
        }
    }
}

/// A named, project-independent set of bindings (a controller preset).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ControllerMap {
    pub name: String,
    pub bindings: Vec<MidiBinding>,
}

// --- Mapping helpers (single source of truth) ---------------------------------

/// Pickup engages once the incoming value is within this tolerance of the
/// target's current value (~1.5 CC steps). See [`takeover_value`].
pub const PICKUP_TOLERANCE: f32 = 1.5 / 127.0;

/// Per-message convergence fraction for [`Takeover::Scale`]: each accepted
/// message moves the value this far from current toward incoming. See
/// [`takeover_value`].
pub const SCALE_RATE: f32 = 0.5;

/// Map an absolute CC value (`0..=127`) onto the binding's `min..=max`
/// normalized sub-range, honoring `invert`. Result is clamped to `0.0..=1.0`.
pub fn cc_to_norm(raw: u8, min: f32, max: f32, invert: bool) -> f32 {
    let mut frac = (raw.min(127) as f32) / 127.0;
    if invert {
        frac = 1.0 - frac;
    }
    (min + frac * (max - min)).clamp(0.0, 1.0)
}

/// Decode a relative-encoder CC value into a signed per-message delta in
/// encoder counts (positive = clockwise / increase). See [`RelativeEnc`] for the
/// per-format conventions.
pub fn decode_relative(enc: RelativeEnc, raw: u8) -> i32 {
    let raw = (raw & 0x7F) as i32;
    match enc {
        RelativeEnc::TwosComplement => {
            if raw < 64 {
                raw
            } else {
                raw - 128
            }
        }
        RelativeEnc::SignedBit => {
            let magnitude = raw & 0x3F;
            if raw & 0x40 != 0 {
                magnitude
            } else {
                -magnitude
            }
        }
        RelativeEnc::BinaryOffset => raw - 64,
    }
}

/// Apply a relative-encoder `delta` (from [`decode_relative`]) to the target's
/// `current_norm` value. The delta is scaled so a full `±127`-count sweep covers
/// the binding's `min..=max` span; `invert` flips direction. Result is clamped to
/// the binding's range (and `0.0..=1.0`).
pub fn apply_delta(current_norm: f32, delta: i32, min: f32, max: f32, invert: bool) -> f32 {
    let (lo, hi) = (min.min(max), min.max(max));
    let span = hi - lo;
    let dir = if invert { -1.0 } else { 1.0 };
    let step = (delta as f32) * span / 127.0;
    (current_norm + dir * step).clamp(lo, hi).clamp(0.0, 1.0)
}

/// Reconcile an `incoming_norm` hardware value with the target's `current_norm`
/// per the soft-takeover `strategy`.
///
/// - [`Takeover::Jump`] ⇒ always `Some(incoming_norm)`.
/// - [`Takeover::Pickup`] ⇒ `Some(incoming_norm)` once within
///   [`PICKUP_TOLERANCE`] of current, else `None` (swallow the message until the
///   control catches up).
/// - [`Takeover::Scale`] ⇒ `Some` of a value moved [`SCALE_RATE`] of the way
///   from current toward incoming (or `incoming_norm` directly once within
///   [`PICKUP_TOLERANCE`]), so repeated moves converge and meet at the extremes.
pub fn takeover_value(strategy: Takeover, incoming_norm: f32, current_norm: f32) -> Option<f32> {
    let incoming = incoming_norm.clamp(0.0, 1.0);
    let current = current_norm.clamp(0.0, 1.0);
    match strategy {
        Takeover::Jump => Some(incoming),
        Takeover::Pickup => {
            if (incoming - current).abs() <= PICKUP_TOLERANCE {
                Some(incoming)
            } else {
                None
            }
        }
        Takeover::Scale => {
            if (incoming - current).abs() <= PICKUP_TOLERANCE {
                Some(incoming)
            } else {
                Some(current + (incoming - current) * SCALE_RATE)
            }
        }
    }
}

// --- Controller-map preset file I/O -------------------------------------------
//
// Lives next to the installed-content registry (doc #167 §1), reusing the same
// config-dir resolution as `registry.rs`. Active *project* mappings persist
// separately in `project.json`; these presets are project-independent.

/// On-disk JSON shape of the controller-map preset file: a list of named maps,
/// mirroring [`crate::registry::InstalledRegistry`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControllerMapStore {
    #[serde(default)]
    pub maps: Vec<ControllerMap>,
}

/// Path to the preset file: `$XDG_DATA_HOME/resonance/controller_maps.json`
/// (alongside `installed.json`).
pub fn controller_maps_path() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("resonance/controller_maps.json"))
}

/// Load all controller maps. Returns an empty list if the file is missing or
/// unparseable (never block on a corrupt file — same policy as the registry).
pub fn load_controller_maps() -> Vec<ControllerMap> {
    controller_maps_path()
        .map(|p| load_controller_maps_from(&p))
        .unwrap_or_default()
}

/// Load from a specific path (useful for testing).
pub fn load_controller_maps_from(path: &Path) -> Vec<ControllerMap> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<ControllerMapStore>(&bytes)
            .map(|s| s.maps)
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Save one controller map, replacing any existing map with the same `name`
/// (presets are keyed by name) so re-saves don't accumulate duplicates.
pub fn save_controller_map(map: &ControllerMap) -> Result<(), String> {
    let path = controller_maps_path().ok_or_else(|| "no data dir".to_string())?;
    save_controller_map_to(map, &path)
}

/// Save to a specific path (useful for testing).
pub fn save_controller_map_to(map: &ControllerMap, path: &Path) -> Result<(), String> {
    let mut maps = load_controller_maps_from(path);
    maps.retain(|m| m.name != map.name);
    maps.push(map.clone());
    write_store(&ControllerMapStore { maps }, path)
}

/// Delete the controller map with the given name. Succeeds even if no such map
/// exists (the file is left listing the remaining maps).
pub fn delete_controller_map(name: &str) -> Result<(), String> {
    let path = controller_maps_path().ok_or_else(|| "no data dir".to_string())?;
    delete_controller_map_from(name, &path)
}

/// Delete from a specific path (useful for testing).
pub fn delete_controller_map_from(name: &str, path: &Path) -> Result<(), String> {
    let mut maps = load_controller_maps_from(path);
    maps.retain(|m| m.name != name);
    write_store(&ControllerMapStore { maps }, path)
}

/// Persist the whole store, pretty-printed, creating parent dirs as needed.
fn write_store(store: &ControllerMapStore, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| format!("serialize controller maps: {e}"))?;
    std::fs::write(path, json.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}
