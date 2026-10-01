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
//! What counts as what, from the manifest's `position` field ([`MicKinds`];
//! case, spaces, `_` and `-` do not count):
//!
//! - **overhead**: a position starting `OH` (`OHsAB`, `OHsXY`) or naming
//!   an `Overhead`;
//! - **room**: a position naming a `Room` (`Room`, `RoomFar`, `Mono Room`)
//!   or starting `Amb` (`Ambient`, `Ambience`);
//! - **bleed**: a close position of **another** pad
//!   (`PAD_MAPPINGS[other].close_mic_positions`) — a close mic that
//!   belongs to another piece but was recording when this one was hit.
//!   In Drummica that is SN Btm on the kick and the toms (the snare wires'
//!   buzz), recorded on the "mit Teppich" pieces only. A position no pad
//!   lists (`SnareTop` for `SNTop`, a cymbal's own spot mic) is never
//!   bleed: guessing wrong would play a piece's own close mic as bleed.
//!
//! A kit can say what a position is outright, which beats every guess:
//! `_meta.mic_kinds: {"<position>": "close"|"overhead"|"room"|"bleed"}`.

use std::collections::BTreeMap;

pub use resonance_common::drumkit_library::MicKind;
use resonance_common::drumkit_library::KitMeta;

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

/// How one kit's mic positions classify (see the module docs): the
/// kit's own word (`_meta.mic_kinds`) where it gives one, else a guess
/// from the name. The default is the guess alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MicKinds {
    /// `_meta.mic_kinds`, keyed by [`normalize`]d position.
    overrides: BTreeMap<String, MicKind>,
}

/// A position as compared: lowercase letters and digits only, so
/// `"Mono Room"`, `"mono_room"` and `"MonoRoom"` are one position.
fn normalize(position: &str) -> String {
    position
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Whether pad `pad`'s mapping lists `position` as one of its close mics.
fn is_own_close(pad: usize, normalized: &str) -> bool {
    PAD_MAPPINGS.get(pad).is_some_and(|m| {
        m.close_mic_positions
            .iter()
            .any(|own| normalize(own) == normalized)
    })
}

/// The canonical spelling of `position` if some pad lists it as a close
/// mic (`"snbtm"` → `"SNBtm"`), and the first pad that does.
fn close_owner(position: &str) -> Option<(usize, &'static str)> {
    let normalized = normalize(position);
    PAD_MAPPINGS.iter().enumerate().find_map(|(pad, m)| {
        m.close_mic_positions
            .iter()
            .find(|own| normalize(own) == normalized)
            .map(|own| (pad, *own))
    })
}

impl MicKinds {
    /// The kit's `_meta.mic_kinds`.
    pub fn from_meta(meta: &KitMeta) -> Self {
        Self {
            overrides: meta
                .mic_kinds
                .iter()
                .map(|(position, kind)| (normalize(position), *kind))
                .collect(),
        }
    }

    fn stated(&self, normalized: &str) -> Option<MicKind> {
        self.overrides.get(normalized).copied()
    }

    /// A position that names an overhead setup.
    pub fn is_overhead(&self, position: &str) -> bool {
        let n = normalize(position);
        match self.stated(&n) {
            Some(kind) => kind == MicKind::Overhead,
            None => n.starts_with("oh") || n.contains("overhead"),
        }
    }

    /// A position that names a room setup. Conservative: a far or mid
    /// mic that does not say "room" (or "amb…") is left alone — the kit
    /// can name it in `_meta.mic_kinds`.
    pub fn is_room(&self, position: &str) -> bool {
        let n = normalize(position);
        match self.stated(&n) {
            Some(kind) => kind == MicKind::Room,
            None => !self.is_overhead(position) && (n.contains("room") || n.starts_with("amb")),
        }
    }

    /// Whether `position` is a bleed position on pad `pad`: never one of
    /// the pad's own close mics; else one the kit calls bleed, or the
    /// close position of another pad.
    pub fn is_bleed(&self, pad: usize, position: &str) -> bool {
        let n = normalize(position);
        if is_own_close(pad, &n) {
            return false;
        }
        match self.stated(&n) {
            Some(MicKind::Bleed) => true,
            Some(MicKind::Overhead | MicKind::Room) => false,
            Some(MicKind::Close) | None => {
                !self.is_overhead(position)
                    && !self.is_room(position)
                    && close_owner(position).is_some()
            }
        }
    }
}

/// A position that names an overhead setup, by name alone
/// ([`MicKinds::is_overhead`] for a kit with no `_meta.mic_kinds`).
pub fn is_overhead_position(position: &str) -> bool {
    MicKinds::default().is_overhead(position)
}

/// A position that names a room setup, by name alone.
pub fn is_room_position(position: &str) -> bool {
    MicKinds::default().is_room(position)
}

/// Whether `position` is a bleed position on pad `pad`, by name alone.
pub fn is_bleed_position(pad: usize, position: &str) -> bool {
    MicKinds::default().is_bleed(pad, position)
}

/// The setup overhead slot 1 plays on `piece`: `key` if the piece has it
/// as an overhead setup, else its first overhead setup. Like slots 2 and
/// 3, slot 1 never plays a setup of another kind: a close-mic or room key
/// in `overhead_setup_key` falls back as a missing one does.
pub fn resolve_overhead_slot1<'a>(
    piece: &'a BTreeMap<String, MicSetup>,
    key: &str,
    kinds: &MicKinds,
) -> Option<&'a str> {
    piece
        .get_key_value(key)
        .filter(|(_, setup)| kinds.is_overhead(&setup.position))
        .map(|(k, _)| k.as_str())
        .or_else(|| {
            piece
                .iter()
                .find(|(_, setup)| kinds.is_overhead(&setup.position))
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
    kinds: &MicKinds,
) -> Vec<(BankKind, String)> {
    let mut out = Vec::new();
    let slot1 = resolve_overhead_slot1(piece, overhead_setup_key, kinds);
    for (i, key) in banks.setups.extra_overheads.iter().enumerate() {
        let usable = !key.is_empty()
            && Some(key.as_str()) != slot1
            && piece
                .get(key)
                .is_some_and(|setup| kinds.is_overhead(&setup.position))
            && !out.iter().any(|(_, k): &(BankKind, String)| k == key);
        if usable {
            out.push((BankKind::Overhead { slot: i as u8 + 1 }, key.clone()));
        }
    }
    if banks.bleed {
        let mut positions: Vec<&str> = piece
            .values()
            .map(|setup| setup.position.as_str())
            .filter(|position| kinds.is_bleed(pad, position))
            .collect();
        positions.sort_unstable();
        positions.dedup();
        for position in positions.into_iter().take(MAX_BLEED_BANKS) {
            // The owner's picks are keyed by the canonical spelling.
            let owner_pick = close_owner(position)
                .and_then(|(owner, canonical)| {
                    pad_choices.get(owner)?.close_setups.get(canonical)
                })
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
            .filter(|(_, setup)| kinds.is_room(&setup.position))
            .map(|(k, _)| k.clone());
        let key = chosen.or_else(|| {
            piece
                .iter()
                .find(|(_, setup)| kinds.is_room(&setup.position))
                .map(|(k, _)| k.clone())
        });
        if let Some(key) = key {
            out.push((BankKind::Room, key));
        }
    }
    out
}
