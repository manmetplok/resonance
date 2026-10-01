//! What a kit's `drum_samples.json` says about the kit, read without
//! opening a single WAV (drums-plugin-rework.md §3.3).
//!
//! The shape is the drums loader's (`kit_loader/manifest.rs`):
//!
//! ```text
//! { "<piece>": { "<setup key>": { "brand", "channel", "mic", "position",
//!                                 "rounds": { "RRnn": { "VelNN": "<file>" } } } },
//!   "_meta": { "name"?, "pieces": { "<piece>": { "name" } },
//!              "articulations": [ { "primary", "alt", "label" } ],
//!              "pads": { "<piece>": { "note"?, "port"?, "choke"? } } } }
//! ```
//!
//! Every part of `_meta` is optional and read on its own ([`KitMeta`]): a
//! malformed `pads` block costs the kit its pad hints, not its piece
//! names.
//!
//! Sample paths are relative to the manifest's directory. A manifest the
//! loader would refuse (a piece that is not that shape) is a
//! [`ManifestError`]; an unknown or malformed `_meta` is ignored, as the
//! loader ignores it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The manifest's file name.
pub const MANIFEST_FILE: &str = "drum_samples.json";

/// The key of the optional metadata block.
pub const META_KEY: &str = "_meta";

/// Why a manifest could not be read as a kit.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ManifestError(pub String);

/// One mic setup (a column of the kit's mic matrix), as every piece that
/// has it names it. Keyed by its setup key, e.g. `01_KickIn_e901`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicSetupInfo {
    pub position: String,
    pub brand: String,
    pub mic: String,
    pub channel: String,
}

impl MicSetupInfo {
    /// "Shure Beta 91 · KickIn": brand and mic, then the position.
    pub fn label(&self) -> String {
        let gear = [self.brand.as_str(), self.mic.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        match (gear.is_empty(), self.position.is_empty()) {
            (true, _) => self.position.clone(),
            (false, true) => gear,
            (false, false) => format!("{gear} · {}", self.position),
        }
    }
}

/// An articulation pair from `_meta.articulations`: two pieces one pad
/// toggles between, and the toggle's label ("punch/deep").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Articulation {
    pub primary: String,
    pub alt: String,
    #[serde(default)]
    pub label: String,
}

/// One piece: its manifest key and its display name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Piece {
    /// The manifest key (`SD Kick mit Teppich`).
    pub key: String,
    /// `_meta.pieces.<key>.name`, else the key.
    pub name: String,
}

/// Everything the library keeps from a manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestSummary {
    /// `_meta.name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_name: Option<String>,
    /// Sorted by piece key: the manifest is read into a `BTreeMap`, so the
    /// file's own key order is not kept.
    pub pieces: Vec<Piece>,
    /// Setup key → what it is, over every piece.
    pub mic_setups: BTreeMap<String, MicSetupInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub articulations: Vec<Articulation>,
    /// Most velocity layers any (piece, setup, round) has.
    pub layers_max: u32,
    /// Most round robins any (piece, setup) has.
    pub rr_max: u32,
    /// Sample file references in the manifest (not distinct files).
    pub sample_count: u64,
}

#[derive(Deserialize)]
struct RawSetup {
    brand: String,
    channel: String,
    mic: String,
    position: String,
    rounds: BTreeMap<String, BTreeMap<String, String>>,
}

type RawPieces = BTreeMap<String, BTreeMap<String, RawSetup>>;

/// Which output port a kit suggests for a piece (`_meta.pads.<piece>.port`):
/// a port index (0 = Main … 6 = Overhead) or a port's name ("Kick").
///
/// An index may be written as any whole JSON number — `1`, `1.0` — since
/// a manifest written by a tool that only has floats is still the kit's
/// intent. Anything else (a fraction, a negative number, a bool) is not a
/// port hint; [`KitMeta`] then drops the port alone, not the pad's note
/// and choke with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum PortHint {
    Index(u8),
    Name(String),
}

impl PortHint {
    /// The hint a JSON value states, if it states one.
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        match value {
            serde_json::Value::String(name) => Some(PortHint::Name(name.clone())),
            serde_json::Value::Number(_) => whole_u8(value).map(PortHint::Index),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for PortHint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        PortHint::from_value(&value)
            .ok_or_else(|| serde::de::Error::custom("a port index (0-255) or a port name"))
    }
}

/// A JSON number that is a whole number in `0..=255` (`36`, `36.0`).
fn whole_u8(value: &serde_json::Value) -> Option<u8> {
    if let Some(n) = value.as_u64() {
        return u8::try_from(n).ok();
    }
    let f = value.as_f64()?;
    (f.fract() == 0.0 && (0.0..=255.0).contains(&f)).then_some(f as u8)
}

/// `_meta.pads.<piece>`: where a kit wants one of its pieces played. Every
/// field is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadHint {
    /// The MIDI note (so the pad slot) that plays the piece.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<u8>,
    /// The output port the piece's pad defaults to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<PortHint>,
    /// The choke group the piece's pad defaults to; 0 = none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choke: Option<u8>,
}

impl PadHint {
    /// Read one `_meta.pads.<piece>` value field by field: a field of the
    /// wrong shape is left out on its own (a port of `1.5` costs the pad
    /// its port hint, never its note or choke group). Whole numbers may
    /// be written as floats. `None` when the value is not an object, or
    /// states no field this can read.
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        let obj = value.as_object()?;
        let hint = PadHint {
            note: obj.get("note").and_then(whole_u8),
            port: obj.get("port").and_then(PortHint::from_value),
            choke: obj.get("choke").and_then(whole_u8),
        };
        (hint != PadHint::default()).then_some(hint)
    }
}

/// Everything a manifest's `_meta` block says, each part read on its own
/// and leniently: an entry of the wrong shape is skipped, never the block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KitMeta {
    /// `_meta.name`, when it is not blank.
    pub name: Option<String>,
    /// `_meta.pieces.<piece>.name`, piece key → display name (blank names
    /// left out).
    pub piece_names: BTreeMap<String, String>,
    /// `_meta.articulations`, in the file's order.
    pub articulations: Vec<Articulation>,
    /// `_meta.pads`, piece key → hint.
    pub pads: BTreeMap<String, PadHint>,
}

impl KitMeta {
    /// Read a `_meta` value. Anything that is not an object reads as no
    /// metadata at all.
    pub fn from_value(meta: &serde_json::Value) -> Self {
        let mut out = KitMeta::default();
        let Some(obj) = meta.as_object() else {
            return out;
        };
        out.name = obj
            .get("name")
            .and_then(|n| n.as_str())
            .filter(|n| !n.trim().is_empty())
            .map(str::to_string);
        if let Some(pieces) = obj.get("pieces").and_then(|p| p.as_object()) {
            for (key, piece) in pieces {
                if let Some(name) = piece
                    .get("name")
                    .and_then(|n| n.as_str())
                    .filter(|n| !n.trim().is_empty())
                {
                    out.piece_names.insert(key.clone(), name.to_string());
                }
            }
        }
        if let Some(list) = obj.get("articulations").and_then(|a| a.as_array()) {
            out.articulations = list
                .iter()
                .filter_map(|a| serde_json::from_value::<Articulation>(a.clone()).ok())
                .collect();
        }
        if let Some(pads) = obj.get("pads").and_then(|p| p.as_object()) {
            for (key, hint) in pads {
                if let Some(hint) = PadHint::from_value(hint) {
                    out.pads.insert(key.clone(), hint);
                }
            }
        }
        out
    }

    /// The `_meta` of a manifest's bytes; no metadata when the bytes are
    /// not a JSON object or have no `_meta`.
    pub fn from_manifest_bytes(bytes: &[u8]) -> Self {
        serde_json::from_slice::<serde_json::Value>(bytes)
            .ok()
            .and_then(|v| v.get(META_KEY).map(Self::from_value))
            .unwrap_or_default()
    }

    /// The display name of piece `key`, if the kit names it.
    pub fn piece_name(&self, key: &str) -> Option<&str> {
        self.piece_names.get(key).map(String::as_str)
    }
}

fn split_meta(bytes: &[u8]) -> Result<(RawPieces, KitMeta), ManifestError> {
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| ManifestError(format!("not JSON: {e}")))?;
    let obj = value
        .as_object_mut()
        .ok_or_else(|| ManifestError("not a JSON object".into()))?;
    let meta = obj
        .remove(META_KEY)
        .map(|m| KitMeta::from_value(&m))
        .unwrap_or_default();
    let pieces: RawPieces =
        serde_json::from_value(value).map_err(|e| ManifestError(format!("bad piece: {e}")))?;
    Ok((pieces, meta))
}

/// Summarise a manifest's bytes. Opens nothing else.
pub fn summarize(bytes: &[u8]) -> Result<ManifestSummary, ManifestError> {
    let (pieces, meta) = split_meta(bytes)?;
    if pieces.is_empty() {
        return Err(ManifestError("no drum pieces".into()));
    }
    let mut out = ManifestSummary {
        meta_name: meta.name.clone(),
        articulations: meta.articulations.clone(),
        ..ManifestSummary::default()
    };
    for (key, setups) in &pieces {
        let name = meta
            .piece_name(key)
            .map(str::to_string)
            .unwrap_or_else(|| key.clone());
        out.pieces.push(Piece {
            key: key.clone(),
            name,
        });
        for (setup_key, s) in setups {
            out.mic_setups
                .entry(setup_key.clone())
                .or_insert_with(|| MicSetupInfo {
                    position: s.position.clone(),
                    brand: s.brand.clone(),
                    mic: s.mic.clone(),
                    channel: s.channel.clone(),
                });
            out.rr_max = out.rr_max.max(s.rounds.len() as u32);
            for vels in s.rounds.values() {
                out.layers_max = out.layers_max.max(vels.len() as u32);
                out.sample_count += vels.len() as u64;
            }
        }
    }
    Ok(out)
}

/// Every sample path the manifest names, resolved against `kit_dir` (the
/// manifest's directory), deduplicated, in key order (piece, setup, round,
/// velocity — not the file's order).
pub fn sample_paths(bytes: &[u8], kit_dir: &Path) -> Result<Vec<PathBuf>, ManifestError> {
    let (pieces, _) = split_meta(bytes)?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for setups in pieces.values() {
        for s in setups.values() {
            for vels in s.rounds.values() {
                for file in vels.values() {
                    if seen.insert(file.as_str()) {
                        out.push(kit_dir.join(file));
                    }
                }
            }
        }
    }
    Ok(out)
}
