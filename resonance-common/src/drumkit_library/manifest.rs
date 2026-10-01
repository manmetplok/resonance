//! What a kit's `drum_samples.json` says about the kit, read without
//! opening a single WAV (drums-plugin-rework.md §3.3).
//!
//! The shape is the drums loader's (`kit_loader/manifest.rs`):
//!
//! ```text
//! { "<piece>": { "<setup key>": { "brand", "channel", "mic", "position",
//!                                 "rounds": { "RRnn": { "VelNN": "<file>" } } } },
//!   "_meta": { "name"?, "pieces": { "<piece>": { "name" } },
//!              "articulations": [ { "primary", "alt", "label" } ] } }
//! ```
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
    /// In manifest key order.
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

#[derive(Deserialize, Default)]
struct RawMeta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    pieces: BTreeMap<String, RawPieceMeta>,
    #[serde(default)]
    articulations: Vec<Articulation>,
}

#[derive(Deserialize, Default)]
struct RawPieceMeta {
    #[serde(default)]
    name: Option<String>,
}

fn split_meta(bytes: &[u8]) -> Result<(RawPieces, RawMeta), ManifestError> {
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| ManifestError(format!("not JSON: {e}")))?;
    let obj = value
        .as_object_mut()
        .ok_or_else(|| ManifestError("not a JSON object".into()))?;
    let meta = obj
        .remove(META_KEY)
        .and_then(|m| serde_json::from_value::<RawMeta>(m).ok())
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
        meta_name: meta.name.filter(|n| !n.trim().is_empty()),
        articulations: meta.articulations,
        ..ManifestSummary::default()
    };
    for (key, setups) in &pieces {
        let name = meta
            .pieces
            .get(key)
            .and_then(|p| p.name.clone())
            .filter(|n| !n.trim().is_empty())
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
/// manifest's directory), deduplicated, in manifest order.
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
