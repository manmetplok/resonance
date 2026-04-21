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

use serde::Deserialize;

use crate::drum_map::{PadMapping, NUM_PADS, PAD_MAPPINGS};
use crate::kit::{decode_wav, LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};
use crate::mic_catalog::ManifestMicCatalog;
use crate::KitBridge;

// ---------------------------------------------------------------------------
// Manifest types — mirror the shape observed in drummica's drum_samples.json.
// ---------------------------------------------------------------------------

/// Top-level: drum piece name -> map of mic-setup name -> mic setup data.
pub type KitManifest = BTreeMap<String, BTreeMap<String, MicSetup>>;

#[derive(Deserialize)]
#[allow(dead_code)] // brand/channel/mic fields come from the manifest but we only use `position` + `rounds`
pub struct MicSetup {
    pub brand: String,
    pub channel: String,
    pub mic: String,
    pub position: String,
    /// RR name -> velocity name -> relative filename.
    pub rounds: BTreeMap<String, BTreeMap<String, String>>,
}

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
// Per-pad mic-choice state — kept outside the loader so the editor can
// persist it via ExtraStateSaver.
// ---------------------------------------------------------------------------

/// User-chosen setup keys per close-mic position for one pad. If an
/// entry is missing the loader picks the first available setup for that
/// position from the manifest.
#[derive(Debug, Clone, Default)]
pub struct PadMicChoices {
    pub close_setups: BTreeMap<String, String>,
}

// ---------------------------------------------------------------------------
// Status reported by the loader thread, rendered by the editor.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum KitStatus {
    Empty,
    Loading { path: PathBuf },
    Loaded { name: String, num_pads: usize },
    Error { message: String },
}

impl Default for KitStatus {
    fn default() -> Self {
        Self::Empty
    }
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
                *bridge.kit_status.lock().unwrap() = KitStatus::Loading {
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
                    *bridge.catalog.lock().unwrap() = kit.catalog;
                    // Best-effort send; if the channel is full, coalesce
                    // by dropping this load (the newer one wins anyway).
                    let _ = bridge.kit_sender.try_send(kit.pads);
                    *bridge.kit_path.lock().unwrap() = Some(manifest_path);
                    *bridge.kit_status.lock().unwrap() = KitStatus::Loaded { name, num_pads };
                }
                Ok(Err(message)) => {
                    *bridge.kit_status.lock().unwrap() = KitStatus::Error { message };
                }
                Err(_) => {
                    *bridge.kit_status.lock().unwrap() = KitStatus::Error {
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

/// Load the mic bank for `position` from `piece`, picking the user's
/// preferred setup if one is supplied and available, otherwise taking
/// the first setup whose `position` field matches.
fn load_bank_for_position(
    piece_name: &str,
    piece: &BTreeMap<String, MicSetup>,
    kit_dir: &Path,
    target_sr: f32,
    position: &str,
    preferred_setup: Option<&str>,
) -> Result<Option<LoadedMicBank>, String> {
    // Try preferred first, then fall back to position match.
    let chosen = preferred_setup
        .and_then(|key| piece.get(key).map(|setup| (key.to_string(), setup)))
        .or_else(|| {
            piece
                .iter()
                .find(|(_, setup)| setup.position == position)
                .map(|(k, v)| (k.clone(), v))
        });

    let Some((setup_key, setup)) = chosen else {
        return Ok(None);
    };
    let layers = decode_layers(piece_name, setup, kit_dir, target_sr)?;
    Ok(Some(LoadedMicBank {
        position: position.to_string(),
        setup_key,
        layers,
    }))
}

/// Load the overhead bank for a piece using the globally selected OH
/// setup key, falling back to any OH-prefixed setup the piece supplies.
fn load_overhead_bank(
    piece_name: &str,
    piece: &BTreeMap<String, MicSetup>,
    kit_dir: &Path,
    target_sr: f32,
    overhead_setup_key: &str,
) -> Result<Option<LoadedMicBank>, String> {
    let chosen = piece
        .get(overhead_setup_key)
        .map(|setup| (overhead_setup_key.to_string(), setup))
        .or_else(|| {
            piece
                .iter()
                .find(|(_, setup)| setup.position.starts_with("OH"))
                .map(|(k, v)| (k.clone(), v))
        });

    let Some((setup_key, setup)) = chosen else {
        return Ok(None);
    };
    let layers = decode_layers(piece_name, setup, kit_dir, target_sr)?;
    Ok(Some(LoadedMicBank {
        position: setup.position.clone(),
        setup_key,
        layers,
    }))
}

/// Decode all velocity layers / round robins of one mic setup into
/// `VelocityLayer`s. Common helper shared by close and overhead banks.
fn decode_layers(
    piece_name: &str,
    setup: &MicSetup,
    kit_dir: &Path,
    target_sr: f32,
) -> Result<Vec<VelocityLayer>, String> {
    // Reshape rounds: {RR -> {Vel -> filename}} into {Vel -> [RR filenames]}.
    let mut layers_by_vel: BTreeMap<u32, Vec<&String>> = BTreeMap::new();
    for (_rr_name, vel_map) in &setup.rounds {
        for (vel_name, filename) in vel_map {
            let vel_num = parse_vel_index(vel_name).ok_or_else(|| {
                format!("piece '{piece_name}': unparseable velocity key '{vel_name}'")
            })?;
            layers_by_vel.entry(vel_num).or_default().push(filename);
        }
    }

    if layers_by_vel.is_empty() {
        return Err(format!("piece '{piece_name}' has no samples"));
    }

    let mut layers = Vec::with_capacity(layers_by_vel.len());
    for (_vel_num, filenames) in layers_by_vel {
        let mut round_robins = Vec::with_capacity(filenames.len());
        for filename in filenames {
            let path = kit_dir.join(filename);
            let bytes =
                std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
            let data = decode_wav(&bytes, target_sr)
                .map_err(|e| format!("decode {}: {e}", path.display()))?;
            round_robins.push(LoadedSample::from_data(data));
        }
        layers.push(VelocityLayer { round_robins });
    }
    Ok(layers)
}

fn build_fallback_pad(mapping: &PadMapping, target_sr: f32) -> Result<LoadedPad, String> {
    let data = decode_wav(mapping.default_sample, target_sr)
        .map_err(|e| format!("decode embedded {}: {e}", mapping.name))?;
    let sample = LoadedSample::from_data(data);
    Ok(LoadedPad {
        name: mapping.name.to_string(),
        choke_group: mapping.choke_group,
        output_group: mapping.output_group,
        close_mics: vec![LoadedMicBank {
            position: "fallback".to_string(),
            setup_key: String::new(),
            layers: vec![VelocityLayer {
                round_robins: vec![sample],
            }],
        }],
        overhead: None,
    })
}

/// Parse a "VelNN" key into its numeric suffix.
fn parse_vel_index(key: &str) -> Option<u32> {
    let digits = key.strip_prefix("Vel")?;
    digits.parse().ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Resolve the drummica manifest path for the smoke test. Honours the
    /// `RESONANCE_DRUMMICA_PATH` env var so other developers and CI can
    /// point at their own copy; falls back to the author's local path.
    fn drummica_manifest() -> PathBuf {
        std::env::var("RESONANCE_DRUMMICA_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from("/home/jorrit/Documents/Guitar/drummica/drum_samples.json")
            })
    }

    fn default_choices() -> [PadMicChoices; NUM_PADS] {
        std::array::from_fn(|_| PadMicChoices::default())
    }

    fn default_articulations() -> [bool; NUM_PADS] {
        [false; NUM_PADS]
    }

    /// Smoke test: parses the drummica manifest if present, verifying that
    /// the loader returns exactly NUM_PADS pads and that close-mic counts
    /// match the PadMapping spec. Gated on the manifest path existing so
    /// CI without the samples still passes.
    #[test]
    fn drummica_smoke() {
        let manifest = drummica_manifest();
        if !manifest.exists() {
            eprintln!(
                "drummica manifest not present at {}; skipping",
                manifest.display()
            );
            return;
        }
        let kit = load_kit_from_manifest(
            &manifest,
            48000.0,
            DEFAULT_OVERHEAD_SETUP,
            &default_choices(),
            &default_articulations(),
        )
        .expect("drummica kit should load cleanly");
        assert_eq!(kit.pads.len(), NUM_PADS);

        // Kick and snare each have two close mic positions (In+Out and
        // Top+Btm respectively) so they must load two banks.
        assert_eq!(
            kit.pads[0].close_mics.len(),
            2,
            "kick should load KickIn + KickOut"
        );
        assert_eq!(
            kit.pads[1].close_mics.len(),
            2,
            "snare should load SNTop + SNBtm"
        );
        assert_eq!(
            kit.pads[25].close_mics.len(),
            2,
            "rimshot should load SNTop + SNBtm"
        );

        // Tom pads each have a single position mic.
        for tom_idx in [9, 10, 11] {
            assert_eq!(
                kit.pads[tom_idx].close_mics.len(),
                1,
                "tom pad {} should have one close mic bank",
                tom_idx
            );
        }

        // Hi-hat pads have a single Hat close mic.
        for hat_idx in [2, 3, 4, 5, 6, 7, 8] {
            assert_eq!(
                kit.pads[hat_idx].close_mics.len(),
                1,
                "hi-hat pad {} should have one close mic bank",
                hat_idx
            );
        }

        // Cymbal pads (crashes, rides, chinas) have no close mics in Drummica.
        for cymbal_idx in [12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23] {
            assert_eq!(
                kit.pads[cymbal_idx].close_mics.len(),
                0,
                "cymbal pad {} has no close mic in Drummica",
                cymbal_idx
            );
        }

        // Every pad that maps to a Drummica piece has an overhead bank.
        for (i, pad) in kit.pads.iter().enumerate() {
            assert!(
                pad.overhead.is_some(),
                "pad {} ({}) should have an overhead bank",
                i,
                pad.name
            );
        }

        // Every loaded bank must have at least one velocity layer and RR.
        for pad in &kit.pads {
            for bank in pad.close_mics.iter().chain(pad.overhead.iter()) {
                assert!(
                    !bank.layers.is_empty(),
                    "bank {} has no layers",
                    bank.setup_key
                );
                for layer in &bank.layers {
                    assert!(
                        !layer.round_robins.is_empty(),
                        "bank {} layer has no round robins",
                        bank.setup_key
                    );
                }
            }
        }

        // Catalog should mention every Drummica position we expect.
        for position in [
            "KickIn", "KickOut", "SNTop", "SNBtm", "Hat", "Tom01", "Tom02", "TomFloor", "OHsAB",
            "OHsXY",
        ] {
            assert!(
                kit.catalog.positions.contains_key(position),
                "catalog missing position {}",
                position
            );
        }
    }

    /// Smoke test for articulation: load with "ohne Teppich" for kick.
    #[test]
    fn drummica_articulation_smoke() {
        let manifest = drummica_manifest();
        if !manifest.exists() {
            eprintln!(
                "drummica manifest not present at {}; skipping",
                manifest.display()
            );
            return;
        }
        let mut arts = default_articulations();
        arts[0] = true; // Kick -> ohne Teppich
        let kit = load_kit_from_manifest(
            &manifest,
            48000.0,
            DEFAULT_OVERHEAD_SETUP,
            &default_choices(),
            &arts,
        )
        .expect("drummica kit with articulation toggle should load cleanly");
        assert_eq!(kit.pads.len(), NUM_PADS);
        // Kick should still have 2 close mics even with ohne Teppich.
        assert_eq!(
            kit.pads[0].close_mics.len(),
            2,
            "kick ohne Teppich should still load KickIn + KickOut"
        );
    }

    #[test]
    fn parse_vel() {
        assert_eq!(parse_vel_index("Vel01"), Some(1));
        assert_eq!(parse_vel_index("Vel28"), Some(28));
        assert_eq!(parse_vel_index("Velocity"), None);
        assert_eq!(parse_vel_index("foo"), None);
    }

    /// Every embedded default sample decodes cleanly via the fallback path.
    /// This runs without any external assets — it only exercises the bytes
    /// baked into the binary via `include_bytes!`.
    #[test]
    fn fallback_pads_all_decode() {
        for mapping in &PAD_MAPPINGS {
            let pad = build_fallback_pad(mapping, 48000.0)
                .unwrap_or_else(|e| panic!("fallback for {} failed: {e}", mapping.name));
            assert_eq!(
                pad.close_mics.len(),
                1,
                "{} fallback should hold exactly one close bank",
                mapping.name
            );
            assert_eq!(pad.close_mics[0].layers.len(), 1);
            assert_eq!(pad.close_mics[0].layers[0].round_robins.len(), 1);
            assert!(
                pad.close_mics[0].layers[0].round_robins[0].frames > 0,
                "{} sample is empty",
                mapping.name
            );
            assert!(pad.overhead.is_none(), "fallback pads have no overhead");
        }
    }
}
