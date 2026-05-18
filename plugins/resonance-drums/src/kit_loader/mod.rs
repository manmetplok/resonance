//! Drum-kit loader: parses a `drum_samples.json` manifest, decodes the
//! referenced WAV files into `LoadedPad`s, and hands the result back to the
//! audio thread via a `crossbeam_channel`.
//!
//! The loader runs on a dedicated background thread — never on the audio
//! thread and never on the editor/UI thread. It only touches:
//!   * the filesystem (read JSON + WAVs),
//!   * the shared `kit_path` / `kit_status` arcs (for reporting),
//!   * the SPSC channel (for publishing the new pad set).

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::drum_map::{PadMapping, NUM_PADS, PAD_MAPPINGS};
use crate::kit::LoadedPad;
use crate::mic_catalog::ManifestMicCatalog;
use crate::KitBridge;

pub mod decode;
pub mod fallback;
pub mod manifest;

pub use fallback::build_fallback_pad;
pub use manifest::{parse_vel_index, KitManifest, MicSetup, PadMicChoices};

use decode::{load_bank_for_position, load_overhead_bank};

// ---------------------------------------------------------------------------
// Drum-piece -> pad slot mapping.
//
// Fixed mapping from the 30 hardcoded pad slots to drummica drum-piece names.
// Kits that don't ship every name (or use different names) fall back to the
// embedded default sample for that slot.
//
// Pads with `has_articulation == true` in PAD_MAPPINGS have an alternate
// piece name in DRUMMICA_ARTICULATION_ALT. When the user toggles the
// articulation, the loader uses the alt name instead of the primary.
// ---------------------------------------------------------------------------

const DRUMMICA_MAPPING: [&str; NUM_PADS] = [
    "SD Kick mit Teppich",      // 0  Kick
    "SD Snare Normal",          // 1  Snare
    "SD Hat Closed",            // 2  Hi-Hat Closed
    "SD Hat Open",              // 3  Hi-Hat Open
    "SD Hat Half Open",         // 4  Hi-Hat Half Open
    "SD Hat Loose",             // 5  Hi-Hat Loose
    "SD Hat Pedal",             // 6  Hi-Hat Pedal
    "SD Hat Pressed",           // 7  Hi-Hat Pressed
    "SD Hat Trash Open",        // 8  Hi-Hat Trash Open
    "SD Tom01 mit Teppich",     // 9  Tom High
    "SD Tom02 mit Teppich",     // 10 Tom Mid
    "SD Tom Floor mit Teppich", // 11 Tom Low
    "SD Crash 16 Edge",         // 12 Crash 16 Edge
    "SD Crash 16 Bell",         // 13 Crash 16 Bell
    "SD Crash 16 Tip",          // 14 Crash 16 Tip
    "SD Crash 18 Edge",         // 15 Crash 18 Edge
    "SD Crash 18 Bell",         // 16 Crash 18 Bell
    "SD Crash 18 Tip",          // 17 Crash 18 Tip
    "SD Ride Edge",             // 18 Ride Edge
    "SD Ride Bell",             // 19 Ride Bell
    "SD Ride Tip",              // 20 Ride Tip
    "SD China 16 Edge",         // 21 China Edge
    "SD China 16 Bell",         // 22 China Bell
    "SD China 16 Tip",          // 23 China Tip
    "SD Snare Sidestick",       // 24 Sidestick
    "SD Snare Rimshots",        // 25 Rimshot
    "SD Snare Flam",            // 26 Snare Flam
    "SD Snare Roll",            // 27 Snare Roll
    "SD Snare Handtuch",        // 28 Snare Handtuch
    "SD Count Stick",           // 29 Count Stick
];

/// Alternate drummica piece names for the articulation toggle (ohne Teppich).
/// Empty string means the pad has no articulation variant.
const DRUMMICA_ARTICULATION_ALT: [&str; NUM_PADS] = [
    "SD Kick ohne Teppich",      // 0  Kick
    "SD Snare ohne Teppich",     // 1  Snare
    "",                          // 2  Hi-Hat Closed
    "",                          // 3  Hi-Hat Open
    "",                          // 4  Hi-Hat Half Open
    "",                          // 5  Hi-Hat Loose
    "",                          // 6  Hi-Hat Pedal
    "",                          // 7  Hi-Hat Pressed
    "",                          // 8  Hi-Hat Trash Open
    "SD Tom01 ohne Teppich",     // 9  Tom High
    "SD Tom02 ohne Teppich",     // 10 Tom Mid
    "SD Tom Floor ohne Teppich", // 11 Tom Low
    "",                          // 12 Crash 16 Edge
    "",                          // 13 Crash 16 Bell
    "",                          // 14 Crash 16 Tip
    "",                          // 15 Crash 18 Edge
    "",                          // 16 Crash 18 Bell
    "",                          // 17 Crash 18 Tip
    "",                          // 18 Ride Edge
    "",                          // 19 Ride Bell
    "",                          // 20 Ride Tip
    "",                          // 21 China Edge
    "",                          // 22 China Bell
    "",                          // 23 China Tip
    "",                          // 24 Sidestick
    "",                          // 25 Rimshot
    "",                          // 26 Snare Flam
    "",                          // 27 Snare Roll
    "",                          // 28 Snare Handtuch
    "",                          // 29 Count Stick
];

/// Default overhead setup key. Matches the pre-multi-output loader so
/// existing projects load with no audible change.
pub const DEFAULT_OVERHEAD_SETUP: &str = "23_OHsAB_e914";

// ---------------------------------------------------------------------------
// Status reported by the loader thread, rendered by the editor.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub enum KitStatus {
    #[default]
    Empty,
    Loading { path: PathBuf },
    Loaded { name: String, num_pads: usize },
    Error { message: String },
}


/// Output of a successful kit load — the decoded pads + a snapshot of
/// the manifest's mic catalog for the GUI.
pub struct LoadedKit {
    pub pads: Vec<LoadedPad>,
    pub catalog: ManifestMicCatalog,
}

// ---------------------------------------------------------------------------
// Public loader entrypoint.
// ---------------------------------------------------------------------------

/// Parse the manifest at `manifest_path`, decode every referenced sample at
/// `target_sr`, and return the assembled pad list + catalog of available
/// mic setups for the GUI.
///
/// `articulations` is a per-pad boolean: when true, the loader uses the
/// alternate (ohne Teppich) piece name for that pad instead of the primary.
pub fn load_kit_from_manifest(
    manifest_path: &Path,
    target_sr: f32,
    overhead_setup_key: &str,
    pad_choices: &[PadMicChoices; NUM_PADS],
    articulations: &[bool; NUM_PADS],
) -> Result<LoadedKit, String> {
    let bytes = std::fs::read(manifest_path).map_err(|e| format!("read manifest: {e}"))?;

    // Two-phase parse: first as raw JSON so we can strip the optional
    // `_meta` key (which has a different shape than a drum piece), then
    // deserialize the remaining entries as the usual KitManifest.
    let mut raw: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("parse manifest JSON: {e}"))?;
    // Remove _meta if present so it doesn't trip up the KitManifest deserializer.
    if let Some(obj) = raw.as_object_mut() {
        obj.remove("_meta");
    }
    let manifest: KitManifest =
        serde_json::from_value(raw).map_err(|e| format!("parse manifest pieces: {e}"))?;

    let kit_dir = manifest_path
        .parent()
        .ok_or_else(|| "manifest path has no parent directory".to_string())?;

    let catalog = ManifestMicCatalog::from_manifest(&manifest);

    let mut pads = Vec::with_capacity(NUM_PADS);
    for (pad_idx, mapping) in PAD_MAPPINGS.iter().enumerate() {
        let piece_name = if articulations[pad_idx] && !DRUMMICA_ARTICULATION_ALT[pad_idx].is_empty()
        {
            DRUMMICA_ARTICULATION_ALT[pad_idx]
        } else {
            DRUMMICA_MAPPING[pad_idx]
        };
        let pad = match manifest.get(piece_name) {
            Some(piece) => build_pad_from_piece(
                mapping,
                piece_name,
                piece,
                kit_dir,
                target_sr,
                overhead_setup_key,
                &pad_choices[pad_idx],
            )?,
            None => build_fallback_pad(mapping, target_sr)?,
        };
        pads.push(pad);
    }

    Ok(LoadedKit { pads, catalog })
}

/// Spawn a background loader thread. Writes status updates and the kit path
/// to `bridge`, and publishes the finished pad vec through `bridge.kit_sender`.
///
/// Each call bumps `bridge.load_generation`; in-flight older loads check
/// the stamp before writing state and become no-ops if a newer load has
/// started, so last-click-wins status is preserved even under spam.
/// Loader panics are caught and converted to `KitStatus::Error`.
pub fn spawn_loader(
    manifest_path: PathBuf,
    target_sr: f32,
    bridge: &KitBridge,
    overhead_setup_key: String,
    pad_choices: [PadMicChoices; NUM_PADS],
    articulations: [bool; NUM_PADS],
) {
    let bridge = bridge.clone();
    let stamp = bridge.load_generation.fetch_add(1, Ordering::AcqRel) + 1;

    std::thread::Builder::new()
        .name("resonance-drums-loader".to_string())
        .spawn(move || {
            // Publish "Loading" only if we're still the latest load.
            if bridge.load_generation.load(Ordering::Acquire) == stamp {
                *bridge.kit_status.lock() = KitStatus::Loading {
                    path: manifest_path.clone(),
                };
            }

            let outcome = catch_unwind(AssertUnwindSafe(|| {
                load_kit_from_manifest(
                    &manifest_path,
                    target_sr,
                    &overhead_setup_key,
                    &pad_choices,
                    &articulations,
                )
            }));

            // Only the newest load is allowed to write final state.
            if bridge.load_generation.load(Ordering::Acquire) != stamp {
                return;
            }

            match outcome {
                Ok(Ok(kit)) => {
                    let num_pads = kit.pads.len();
                    let name = manifest_path
                        .parent()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "kit".to_string());
                    *bridge.catalog.lock() = kit.catalog;
                    // Best-effort send; if the channel is full, coalesce
                    // by dropping this load (the newer one wins anyway).
                    let _ = bridge.kit_sender.try_send(kit.pads);
                    *bridge.kit_path.lock() = Some(manifest_path);
                    *bridge.kit_status.lock() = KitStatus::Loaded { name, num_pads };
                }
                Ok(Err(message)) => {
                    *bridge.kit_status.lock() = KitStatus::Error { message };
                }
                Err(_) => {
                    *bridge.kit_status.lock() = KitStatus::Error {
                        message: "loader panicked".to_string(),
                    };
                }
            }
        })
        .expect("spawn drums kit loader thread");
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn build_pad_from_piece(
    mapping: &PadMapping,
    piece_name: &str,
    piece: &BTreeMap<String, MicSetup>,
    kit_dir: &Path,
    target_sr: f32,
    overhead_setup_key: &str,
    choices: &PadMicChoices,
) -> Result<LoadedPad, String> {
    // Close-mic banks: one per position in PadMapping::close_mic_positions
    // (already empty for cymbals). For kick/snare that's two banks.
    let mut close_mics = Vec::with_capacity(mapping.close_mic_positions.len());
    for position in mapping.close_mic_positions {
        if let Some(bank) = load_bank_for_position(
            piece_name,
            piece,
            kit_dir,
            target_sr,
            position,
            choices.close_setups.get(*position).map(String::as_str),
        )? {
            close_mics.push(bank);
        }
    }

    // Overhead bank: look up the global overhead setup key directly. If
    // the piece doesn't have that specific setup, fall back to any
    // OH-prefixed setup the piece does have so the pad still makes sound.
    let overhead = load_overhead_bank(piece_name, piece, kit_dir, target_sr, overhead_setup_key)?;

    Ok(LoadedPad {
        name: mapping.name.to_string(),
        choke_group: mapping.choke_group,
        output_group: mapping.output_group,
        close_mics,
        overhead,
    })
}
