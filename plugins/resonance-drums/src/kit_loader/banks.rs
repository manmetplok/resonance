//! Which E15 mic banks a pad plays beyond its close mics and overhead
//! slot 1: more overhead setups, bleed and room (drums-plugin-rework.md
//! §7 E15).
//!
//! The choices are kit-wide. The setups for overhead slots 2 and 3 and
//! the room setup are plugin state ([`MicBankSetups`], state key
//! `mic_banks`); whether bleed and room play at all are the `bleed_on` /
//! `room_on` params. A load resolves them per pad against the pad's piece
//! ([`resolve_extra_banks`]) into setup keys, which go into the pad's
//! [`super::PadRequest`] — so turning a bank on rebuilds only the pads
//! that gain a bank, and decodes only that bank's files (E4; the pad's
//! other banks come back from the shared cache).
//!
//! What counts as what, from the manifest's `position` field:
//!
//! - **overhead**: a position starting `OH` (`OHsAB`, `OHsXY`);
//! - **room**: a position starting `Room`;
//! - **bleed**: any other position the pad does not list as its own close
//!   mic (`PAD_MAPPINGS[pad].close_mic_positions`) — a close mic that
//!   belongs to another piece but was recording when this one was hit.
//!   In Drummica that is SN Btm on the kick and the toms (the snare wires'
//!   buzz), recorded on the "mit Teppich" pieces only.

use std::collections::BTreeMap;

use crate::drum_map::PAD_MAPPINGS;
use crate::kit::{BankKind, MAX_BLEED_BANKS, MAX_OVERHEAD_SLOTS};

use super::manifest::{MicSetup, PadMicChoices};

/// The state key the bank setups are saved under.
pub const MIC_BANKS_STATE_KEY: &str = "mic_banks";

/// The kit-wide setup choices for the E15 banks, kept as plugin state.
/// Overhead slot 1 is the long-standing `overhead_setup_key`, kept apart
/// so a state from before E15 means what it always did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MicBankSetups {
    /// The setups of overhead slots 2 and 3; `""` is an empty (off) slot,
    /// the default. A slot naming a setup a piece lacks plays nothing on
    /// that piece (no fallback: a layered overhead is a deliberate pick).
    pub extra_overheads: [String; MAX_OVERHEAD_SLOTS - 1],
    /// The room setup; `""` (the default) plays the piece's first `Room*`
    /// setup. Only heard with `room_on`.
    pub room: String,
}

impl MicBankSetups {
    /// As saved: `{"overheads": [slot 2, slot 3], "room": key}`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "overheads": self.extra_overheads,
            "room": self.room,
        })
    }

    /// Read [`to_json`](Self::to_json)'s shape; missing or malformed
    /// parts read as their defaults.
    pub fn from_json(value: &serde_json::Value) -> Self {
        let mut setups = Self::default();
        if let Some(arr) = value.get("overheads").and_then(|v| v.as_array()) {
            for (slot, key) in setups.extra_overheads.iter_mut().zip(arr) {
                if let Some(k) = key.as_str() {
                    *slot = k.to_string();
                }
            }
        }
        if let Some(room) = value.get("room").and_then(|v| v.as_str()) {
            setups.room = room.to_string();
        }
        setups
    }
}

/// Everything about the E15 banks a load decodes from: the setups and
/// whether bleed and room are on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BankRequest {
    pub setups: MicBankSetups,
    /// `bleed_on`.
    pub bleed: bool,
    /// `room_on`.
    pub room: bool,
}

/// A position that names an overhead setup.
pub fn is_overhead_position(position: &str) -> bool {
    position.starts_with("OH")
}

/// A position that names a room setup.
pub fn is_room_position(position: &str) -> bool {
    position.starts_with("Room")
}

/// Whether `position` is a bleed position on pad `pad`: a close mic that
/// is not one of the pad's own.
pub fn is_bleed_position(pad: usize, position: &str) -> bool {
    !is_overhead_position(position)
        && !is_room_position(position)
        && !PAD_MAPPINGS
            .get(pad)
            .is_some_and(|m| m.close_mic_positions.contains(&position))
}

/// The setup overhead slot 1 plays on `piece`: `key` if the piece has it,
/// else its first overhead setup (`decode::plan_overhead_bank`'s rule).
pub fn resolve_overhead_slot1<'a>(
    piece: &'a BTreeMap<String, MicSetup>,
    key: &str,
) -> Option<&'a str> {
    piece
        .get_key_value(key)
        .map(|(k, _)| k.as_str())
        .or_else(|| {
            piece
                .iter()
                .find(|(_, setup)| is_overhead_position(&setup.position))
                .map(|(k, _)| k.as_str())
        })
}

/// The E15 banks pad `pad` plays from `piece`, as (kind, setup key), in
/// [`crate::kit::LoadedPad::extra_banks`] order: overhead slots 2 and 3,
/// the bleed banks, the room bank. Empty with every bank off (the
/// default), whatever the kit.
///
/// - An overhead slot plays its setup if the piece has it, and it is not
///   the setup slot 1 already plays here.
/// - Bleed plays, per bleed position (at most [`MAX_BLEED_BANKS`], in
///   position order), the setup the pad that owns the position picked for
///   it (the snare's SN Btm pick, on the kick) — the same mic on every
///   pad — else the piece's first setup there.
/// - Room plays the chosen room setup if the piece has it, else its first
///   `Room*` setup.
pub fn resolve_extra_banks(
    pad: usize,
    piece: &BTreeMap<String, MicSetup>,
    overhead_setup_key: &str,
    pad_choices: &[PadMicChoices],
    banks: &BankRequest,
) -> Vec<(BankKind, String)> {
    let mut out = Vec::new();
    let slot1 = resolve_overhead_slot1(piece, overhead_setup_key);
    for (i, key) in banks.setups.extra_overheads.iter().enumerate() {
        let usable = !key.is_empty()
            && Some(key.as_str()) != slot1
            && piece
                .get(key)
                .is_some_and(|setup| is_overhead_position(&setup.position))
            && !out.iter().any(|(_, k): &(BankKind, String)| k == key);
        if usable {
            out.push((BankKind::Overhead { slot: i as u8 + 1 }, key.clone()));
        }
    }
    if banks.bleed {
        let mut positions: Vec<&str> = piece
            .values()
            .map(|setup| setup.position.as_str())
            .filter(|position| is_bleed_position(pad, position))
            .collect();
        positions.sort_unstable();
        positions.dedup();
        for position in positions.into_iter().take(MAX_BLEED_BANKS) {
            let owner_pick = PAD_MAPPINGS
                .iter()
                .position(|m| m.close_mic_positions.contains(&position))
                .and_then(|owner| pad_choices.get(owner))
                .and_then(|choices| choices.close_setups.get(position))
                .filter(|key| piece.get(*key).is_some_and(|s| s.position == position));
            let key = owner_pick.cloned().or_else(|| {
                piece
                    .iter()
                    .find(|(_, setup)| setup.position == position)
                    .map(|(k, _)| k.clone())
            });
            if let Some(key) = key {
                out.push((BankKind::Bleed, key));
            }
        }
    }
    if banks.room {
        let chosen = Some(banks.setups.room.as_str())
            .filter(|key| !key.is_empty())
            .and_then(|key| piece.get_key_value(key))
            .filter(|(_, setup)| is_room_position(&setup.position))
            .map(|(k, _)| k.clone());
        let key = chosen.or_else(|| {
            piece
                .iter()
                .find(|(_, setup)| is_room_position(&setup.position))
                .map(|(k, _)| k.clone())
        });
        if let Some(key) = key {
            out.push((BankKind::Room, key));
        }
    }
    out
}
