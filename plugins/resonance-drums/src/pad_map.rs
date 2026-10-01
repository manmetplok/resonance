//! Which piece of a kit each pad plays, under what name, with which
//! articulation — the kit-driven pads of drums-plugin-rework.md §7 E10.
//!
//! # Where a pad's piece comes from
//!
//! The plugin has [`NUM_PADS`] fixed pad slots, one per note of
//! `resonance_common::drum_map::GM_PADS`. A kit fills them from its
//! manifest ([`KitPads::resolve`]):
//!
//! 1. **`_meta.pads.<piece>.note`** places that piece on the pad whose
//!    note it names. A piece placed this way plays only there, not also
//!    on the slot the Drummica table gives it. When two pieces name one
//!    note, the first in key order wins it. A note no pad plays is
//!    ignored (the piece falls back to the table).
//! 2. **The Drummica table** ([`DRUMMICA_MAPPING`]) fills every slot no
//!    override took, with the table's piece — if the kit has it.
//! 3. Anything else is **absent** (D7): the pad is silent, the editor dims
//!    it and the inspector says "Not in this kit". Nothing is filled in
//!    from the built-in samples; the built-in kit plays only when no kit
//!    is selected (or the selected kit is missing altogether).
//!
//! # Names
//!
//! A present pad shows `_meta.pieces.<piece>.name`. Without one it shows
//! the GM pad name when it plays the table's piece for its slot ("Kick"
//! rather than "SD Kick mit Teppich"), else the piece key without the
//! `"SD "` prefix the plok.org kits share. An absent pad shows the GM pad
//! name, dimmed.
//!
//! # Articulations
//!
//! The pad's `pad_N_articulation` parameter stays generic: 0 is the
//! primary piece, 1 the alternate ([`crate::articulation`]). Which piece
//! is the alternate, and what the two are called, comes from the kit:
//!
//! - A kit with `_meta.articulations` pairs exactly the pieces it lists
//!   (`{primary, alt, label}`), and only those.
//! - A kit without it gets the Drummica pairs ([`DRUMMICA_ARTICULATION_ALT`])
//!   labelled [`DRUMMICA_ARTICULATION_LABEL`].
//!
//! Either way the alternate piece must be in the kit, or the pad has no
//! articulation and the parameter selects the primary piece whatever its
//! value. The label is split into the two chip labels at its `/`
//! ([`split_label`]): "punch/deep" → "punch" / "deep", and "mit/ohne
//! Teppich" → "mit Teppich" / "ohne Teppich".
//!
//! # Port and choke hints
//!
//! `_meta.pads.<piece>.port` and `.choke` are the kit's suggested output
//! port and choke group for the pad that plays the piece. They are kept on
//! [`KitPad::port`] / [`KitPad::choke`] for whoever applies defaults on a
//! kit load, and the loader also builds the pad with them
//! ([`KitPad::output_group`], [`KitPad::choke_group`]).

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use resonance_common::drumkit_library::{KitMeta, PortHint};

use crate::drum_map::{pad_index_for_note, NUM_PADS, PAD_MAPPINGS};
use crate::kit::{OutputGroup, NUM_OUTPUT_PORTS, OUTPUT_PORT_NAMES};

/// The Drummica piece each pad slot plays when the kit says nothing else.
pub const DRUMMICA_MAPPING: [&str; NUM_PADS] = [
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

/// The Drummica alternate ("ohne Teppich") piece per slot, for kits
/// without `_meta.articulations`. Empty: the slot has none.
pub const DRUMMICA_ARTICULATION_ALT: [&str; NUM_PADS] = [
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

/// The label of a Drummica-table articulation (a kit without
/// `_meta.articulations`).
pub const DRUMMICA_ARTICULATION_LABEL: &str = "mit/ohne Teppich";

/// Highest choke group a kit can suggest (E12: groups 1–8, 0 = none).
pub const MAX_CHOKE_GROUP: u8 = 8;

/// One pad's two pieces and what they are called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadArticulation {
    /// The piece parameter value 0 plays.
    pub primary: String,
    /// The piece parameter value 1 plays.
    pub alt: String,
    /// The kit's label for the pair ("punch/deep").
    pub label: String,
    /// The chip label of value 0 ("punch").
    pub primary_label: String,
    /// The chip label of value 1 ("deep").
    pub alt_label: String,
}

impl PadArticulation {
    fn new(primary: &str, alt: &str, label: &str) -> Self {
        let (primary_label, alt_label) = split_label(label);
        Self {
            primary: primary.to_string(),
            alt: alt.to_string(),
            label: label.to_string(),
            primary_label,
            alt_label,
        }
    }

    /// The chip label of parameter value `value` (0 or 1).
    pub fn label_of(&self, value: i32) -> Option<&str> {
        match value {
            0 => Some(&self.primary_label),
            1 => Some(&self.alt_label),
            _ => None,
        }
    }
}

/// What one pad slot plays in the current kit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitPad {
    /// The name the editor shows.
    pub name: String,
    /// Whether the pad sounds: the kit has its piece, or no kit is
    /// selected (the built-in kit plays every pad).
    pub present: bool,
    /// The manifest piece the pad plays (its primary piece). `None` for an
    /// absent pad and for every pad of the built-in kit.
    pub piece: Option<String>,
    /// The pad's articulation pair, when the kit has one for it.
    pub articulation: Option<PadArticulation>,
    /// The kit's suggested output port (`_meta.pads.<piece>.port`), as an
    /// index into [`OUTPUT_PORT_NAMES`].
    pub port: Option<usize>,
    /// The kit's suggested choke group (`_meta.pads.<piece>.choke`):
    /// 0 = none, 1–[`MAX_CHOKE_GROUP`].
    pub choke: Option<u8>,
}

impl KitPad {
    /// The close-mic port the pad's banks go to: the kit's suggestion when
    /// it names a close-mic port, else the slot's own.
    pub fn output_group(&self, slot: usize) -> OutputGroup {
        match self.port {
            Some(0) => OutputGroup::Main,
            Some(1) => OutputGroup::Kick,
            Some(2) => OutputGroup::Snare,
            Some(3) => OutputGroup::Toms,
            Some(4) => OutputGroup::Hats,
            Some(5) => OutputGroup::Cymbals,
            // The Overhead port is not a close-mic group.
            _ => PAD_MAPPINGS[slot].output_group,
        }
    }

    /// The choke group the pad is built with: the kit's suggestion when it
    /// has one, else the slot's own.
    pub fn choke_group(&self, slot: usize) -> Option<u8> {
        match self.choke {
            Some(0) => None,
            Some(group) => Some(group),
            None => PAD_MAPPINGS[slot].choke_group,
        }
    }
}

/// Every pad slot of one kit, in pad order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitPads {
    pub pads: Vec<KitPad>,
    /// False for the built-in kit.
    pub from_kit: bool,
}

impl KitPads {
    /// The built-in kit: every pad present, under its GM name, with no
    /// articulation (the built-in samples have one recording per pad).
    pub fn builtin() -> Self {
        Self {
            pads: PAD_MAPPINGS
                .iter()
                .map(|mapping| KitPad {
                    name: mapping.name.to_string(),
                    present: true,
                    piece: None,
                    articulation: None,
                    port: None,
                    choke: None,
                })
                .collect(),
            from_kit: false,
        }
    }

    /// The pads a kit with pieces `has_piece` and metadata `meta` gives
    /// (see the module docs for the rules).
    pub fn resolve(has_piece: impl Fn(&str) -> bool, meta: &KitMeta) -> Self {
        // 1. `_meta.pads` notes.
        let mut slots: Vec<Option<String>> = vec![None; NUM_PADS];
        let mut placed: std::collections::HashSet<&str> = Default::default();
        for (piece, hint) in &meta.pads {
            if !has_piece(piece) {
                continue;
            }
            let Some(slot) = hint.note.and_then(pad_index_for_note) else {
                continue;
            };
            placed.insert(piece.as_str());
            if slots[slot].is_none() {
                slots[slot] = Some(piece.clone());
            }
        }
        // 2. The Drummica table for the rest.
        for (slot, entry) in slots.iter_mut().enumerate() {
            let piece = DRUMMICA_MAPPING[slot];
            if entry.is_none() && has_piece(piece) && !placed.contains(piece) {
                *entry = Some(piece.to_string());
            }
        }
        // 3. Names, articulations and hints.
        let pads = slots
            .into_iter()
            .enumerate()
            .map(|(slot, piece)| {
                let Some(piece) = piece else {
                    return KitPad {
                        name: PAD_MAPPINGS[slot].name.to_string(),
                        present: false,
                        piece: None,
                        articulation: None,
                        port: None,
                        choke: None,
                    };
                };
                let name = match meta.piece_name(&piece) {
                    Some(name) => name.to_string(),
                    None if piece == DRUMMICA_MAPPING[slot] => PAD_MAPPINGS[slot].name.to_string(),
                    None => fallback_name(&piece),
                };
                let articulation = if meta.articulations.is_empty() {
                    let alt = DRUMMICA_ARTICULATION_ALT[slot];
                    (piece == DRUMMICA_MAPPING[slot] && !alt.is_empty() && has_piece(alt))
                        .then(|| PadArticulation::new(&piece, alt, DRUMMICA_ARTICULATION_LABEL))
                } else {
                    meta.articulations
                        .iter()
                        .find(|a| a.primary == piece && a.alt != piece && has_piece(&a.alt))
                        .map(|a| PadArticulation::new(&a.primary, &a.alt, &a.label))
                };
                let hint = meta.pads.get(&piece);
                KitPad {
                    name,
                    present: true,
                    articulation,
                    port: hint.and_then(|h| h.port.as_ref()).and_then(port_index),
                    choke: hint
                        .and_then(|h| h.choke)
                        .filter(|&group| group <= MAX_CHOKE_GROUP),
                    piece: Some(piece),
                }
            })
            .collect();
        Self {
            pads,
            from_kit: true,
        }
    }

    /// The manifest piece pad `slot` plays with articulation `alt` (the
    /// parameter's value is not primary), or `None` when the kit lacks the
    /// pad. A pad without an articulation plays its primary piece either
    /// way.
    pub fn piece_for(&self, slot: usize, alt: bool) -> Option<&str> {
        let pad = self.pads.get(slot)?;
        match (&pad.articulation, alt) {
            (Some(articulation), true) => Some(&articulation.alt),
            _ => pad.piece.as_deref(),
        }
    }

    /// Whether pad `slot` sounds in this kit.
    pub fn is_present(&self, slot: usize) -> bool {
        self.pads.get(slot).is_some_and(|pad| pad.present)
    }
}

/// A piece's name when the kit gives none: the key without the `"SD "`
/// prefix every plok.org kit's pieces carry.
pub fn fallback_name(piece: &str) -> String {
    piece.strip_prefix("SD ").unwrap_or(piece).trim().to_string()
}

/// Split an articulation label into its two chip labels at the first `/`.
///
/// When the left side is one word and the right side several, the words
/// after the right side's first are shared: "mit/ohne Teppich" is
/// "mit Teppich" / "ohne Teppich". A label without a `/` names the pair as
/// a whole, so its chips are the label and the label with "(alt)".
pub fn split_label(label: &str) -> (String, String) {
    let label = label.trim();
    let Some((left, right)) = label.split_once('/') else {
        return if label.is_empty() {
            (
                crate::articulation::ARTICULATION_LABELS[0].to_string(),
                crate::articulation::ARTICULATION_LABELS[1].to_string(),
            )
        } else {
            (label.to_string(), format!("{label} (alt)"))
        };
    };
    let (left, right) = (left.trim(), right.trim());
    let left = match right.split_once(char::is_whitespace) {
        Some((_, shared)) if !left.contains(char::is_whitespace) && !left.is_empty() => {
            format!("{left} {}", shared.trim())
        }
        _ => left.to_string(),
    };
    (left, right.to_string())
}

/// The port index a kit's port hint names, if it names one of
/// [`OUTPUT_PORT_NAMES`].
fn port_index(hint: &PortHint) -> Option<usize> {
    match hint {
        PortHint::Index(i) => Some(*i as usize).filter(|&i| i < NUM_OUTPUT_PORTS),
        PortHint::Name(name) => OUTPUT_PORT_NAMES
            .iter()
            .position(|port| port.eq_ignore_ascii_case(name.trim())),
    }
}

/// A loaded kit's manifest path and its pads.
type LoadedKitPads = (PathBuf, Arc<KitPads>);

/// The pads of the kit an instance plays, shared between the loader (which
/// sets them), the editor and the parameter text (which read them).
///
/// It holds the bridge's `kit_path` handle, not the bridge: the parameter
/// text closures hold one of these, and the bridge holds the parameters,
/// so holding the bridge would be a reference cycle.
#[derive(Clone)]
pub struct KitPadsHandle {
    kit_path: Arc<Mutex<Option<PathBuf>>>,
    loaded: Arc<Mutex<Option<LoadedKitPads>>>,
}

impl KitPadsHandle {
    /// A handle that reads `kit_path` (the bridge's) to tell which kit is
    /// current.
    pub fn new(kit_path: Arc<Mutex<Option<PathBuf>>>) -> Self {
        Self {
            kit_path,
            loaded: Arc::new(Mutex::new(None)),
        }
    }

    /// Record the pads of the kit at `path`, as a load built them.
    pub fn set(&self, path: PathBuf, pads: Arc<KitPads>) {
        *self.loaded.lock() = Some((path, pads));
    }

    /// The pads of the kit the instance plays: the last loaded kit's while
    /// it is still the current kit, else the built-in kit's.
    pub fn current(&self) -> Arc<KitPads> {
        let path = self.kit_path.lock().clone();
        let loaded = self.loaded.lock();
        match (&*loaded, path) {
            (Some((loaded_path, pads)), Some(path)) if *loaded_path == path => pads.clone(),
            _ => builtin(),
        }
    }

    /// The chip label of articulation value `value` on pad `slot`, when the
    /// current kit has an articulation for the pad.
    pub fn articulation_text(&self, slot: usize, value: i32) -> Option<String> {
        self.current()
            .pads
            .get(slot)?
            .articulation
            .as_ref()?
            .label_of(value)
            .map(str::to_string)
    }
}

/// The built-in kit's pads, built once.
fn builtin() -> Arc<KitPads> {
    static BUILTIN: std::sync::OnceLock<Arc<KitPads>> = std::sync::OnceLock::new();
    BUILTIN.get_or_init(|| Arc::new(KitPads::builtin())).clone()
}
