//! Decode WAV samples referenced by a drumkit manifest into the in-memory
//! `LoadedMicBank` / `VelocityLayer` shape consumed by the sampler.

use std::collections::BTreeMap;
use std::path::Path;

use crate::kit::{decode_wav, LoadedMicBank, LoadedSample, VelocityLayer};

use super::manifest::{parse_vel_index, MicSetup};

/// Load the mic bank for `position` from `piece`, picking the user's
/// preferred setup if one is supplied and available, otherwise taking
/// the first setup whose `position` field matches.
pub(super) fn load_bank_for_position(
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
pub(super) fn load_overhead_bank(
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
    for vel_map in setup.rounds.values() {
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
