//! Index of the mic setups available in a loaded kit manifest.
//!
//! Scanned once at kit-load time and shared with the GUI (the bridge's
//! `catalog`) so the mic pickers — the inspector's close-mic and overhead
//! picks, and the Setup tab's mic banks (E15) — render without re-reading
//! the JSON on every frame. Keyed by canonical position (`"KickIn"`,
//! `"SNTop"`, `"OHsAB"`, …) so the editor can enumerate exactly the
//! setups the library provides.
//!
//! **What the Setup tab (K5) reads here** for the E15 banks:
//!
//! - [`overhead_setups`](ManifestMicCatalog::overhead_setups): the setups
//!   an overhead slot can play (`KitBridge::set_overhead_slot`; the slots
//!   as they stand are `KitBridge::overhead_slots`);
//! - [`room_setups`](ManifestMicCatalog::room_setups): the setups the
//!   room bank can play (`KitBridge::set_room_setup`; on/off is the
//!   `room_on` param) — empty for a kit with no room mics (Drummica);
//! - [`bleed`](ManifestMicCatalog::bleed): what turning bleed on
//!   (`bleed_on`) adds, per bleed position: its setups and the pads it is
//!   heard on — empty for a kit with no bleed recordings;
//! - [`label`](ManifestMicCatalog::label): a friendly name for any setup
//!   key (`"OHsAB · Sennheiser e914"`).

use std::collections::BTreeMap;

use crate::drum_map::NUM_PADS;
use crate::kit_loader::banks::MicKinds;
use crate::kit_loader::KitManifest;
use crate::pad_map::KitPads;

/// What the manifest says about one setup, for display.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SetupInfo {
    pub position: String,
    pub brand: String,
    pub mic: String,
}

/// One bleed position of the kit (E15): a close mic recorded on pieces it
/// does not belong to (SN Btm on the kick and the toms).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BleedSource {
    /// The position (`"SNBtm"`).
    pub position: String,
    /// Its setups, in manifest order.
    pub setups: Vec<String>,
    /// The pads it is heard on with bleed on (either articulation), in
    /// pad order.
    pub pads: Vec<usize>,
}

/// The mic setups of a kit, by position, and its E15 bank sources.
#[derive(Debug, Default, Clone)]
pub struct ManifestMicCatalog {
    /// Map from position key (e.g. `"KickIn"`) → setup keys in the order
    /// they first appear in the manifest.
    pub positions: BTreeMap<String, Vec<String>>,
    /// Every setup key's position, brand and mic.
    pub setups: BTreeMap<String, SetupInfo>,
    /// The kit's bleed positions (see [`BleedSource`]); filled by
    /// [`with_bleed`](Self::with_bleed), which knows the pads.
    pub bleed: Vec<BleedSource>,
    /// What each position is (the kit's `_meta.mic_kinds`, else a guess
    /// from the name).
    pub kinds: MicKinds,
}

impl ManifestMicCatalog {
    /// Build a catalog by walking every piece in the manifest and
    /// collecting the unique (position, setup_key) pairs encountered.
    pub fn from_manifest(manifest: &KitManifest) -> Self {
        let mut positions: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut setups = BTreeMap::new();
        for piece in manifest.values() {
            for (setup_key, setup) in piece {
                let entry = positions.entry(setup.position.clone()).or_default();
                if !entry.contains(setup_key) {
                    entry.push(setup_key.clone());
                }
                setups
                    .entry(setup_key.clone())
                    .or_insert_with(|| SetupInfo {
                        position: setup.position.clone(),
                        brand: setup.brand.clone(),
                        mic: setup.mic.clone(),
                    });
            }
        }
        Self {
            positions,
            setups,
            bleed: Vec::new(),
            kinds: MicKinds::default(),
        }
    }

    /// [`from_manifest`](Self::from_manifest) plus the kit's bleed sources,
    /// given which piece each pad plays (`pads`) and what each mic
    /// position is (`kinds`).
    pub fn with_bleed(manifest: &KitManifest, pads: &KitPads, kinds: &MicKinds) -> Self {
        let mut catalog = Self::from_manifest(manifest);
        catalog.kinds = kinds.clone();
        let mut bleed: BTreeMap<String, BleedSource> = BTreeMap::new();
        for pad in 0..NUM_PADS {
            for alt in [false, true] {
                let Some(piece) = pads.piece_for(pad, alt).and_then(|p| manifest.get(p)) else {
                    continue;
                };
                for (key, setup) in piece {
                    if !kinds.is_bleed(pad, &setup.position) {
                        continue;
                    }
                    let source =
                        bleed
                            .entry(setup.position.clone())
                            .or_insert_with(|| BleedSource {
                                position: setup.position.clone(),
                                ..BleedSource::default()
                            });
                    if !source.setups.contains(key) {
                        source.setups.push(key.clone());
                    }
                    if !source.pads.contains(&pad) {
                        source.pads.push(pad);
                    }
                }
            }
        }
        catalog.bleed = bleed.into_values().collect();
        catalog
    }

    /// All overhead setup keys, in position then manifest order: what an
    /// overhead slot (E15: up to three at once) can play.
    pub fn overhead_setups(&self) -> Vec<String> {
        self.setups_where(|position| self.kinds.is_overhead(position))
    }

    /// All room setup keys (see [`MicKinds::is_room`]): what the room bank can
    /// play. Empty for a kit without room mics.
    pub fn room_setups(&self) -> Vec<String> {
        self.setups_where(|position| self.kinds.is_room(position))
    }

    /// All setup keys for a specific close-mic position (e.g. `"KickIn"`).
    pub fn close_setups(&self, position: &str) -> Vec<String> {
        self.positions.get(position).cloned().unwrap_or_default()
    }

    /// Whether turning bleed on adds anything on this kit.
    pub fn has_bleed(&self) -> bool {
        !self.bleed.is_empty()
    }

    /// A setup key as the user reads it: `"OHsAB · Sennheiser e914"`, or
    /// the key itself for one the catalog does not know.
    pub fn label(&self, setup_key: &str) -> String {
        match self.setups.get(setup_key) {
            Some(info) => {
                let mic = [info.brand.as_str(), info.mic.as_str()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join(" ");
                if mic.is_empty() {
                    info.position.clone()
                } else {
                    format!("{} · {mic}", info.position)
                }
            }
            None => setup_key.to_string(),
        }
    }

    fn setups_where(&self, wanted: impl Fn(&str) -> bool) -> Vec<String> {
        self.positions
            .iter()
            .filter(|(pos, _)| wanted(pos))
            .flat_map(|(_, keys)| keys.iter().cloned())
            .collect()
    }
}
