//! `com.resonance.kit-info` for Resonance Drums (drums-plugin-rework.md §8,
//! slice K9): the pads of the kit the instance plays — note, name in the
//! kit, present or not — so the host's drum-group kit picker lists the
//! real kit instead of a fixed table. The ABI is
//! `resonance_common::kit_info`; the bridge serves it from the source this
//! module builds.
//!
//! The answer is read from [`crate::pad_map::KitPadsHandle`], which is
//! published with every kit hand-off. The host re-reads it after each
//! params rescan the plugin requests, and every hand-off moves
//! `kit_load_progress`, which requests one.

use std::sync::Arc;

use resonance_common::kit_info::{KitInfo, KitInfoPad};
use resonance_plugin::KitInfoSource;

use crate::drum_map::PAD_MAPPINGS;
use crate::pad_map::KitPadsHandle;

/// The source the drums hand the bridge.
struct DrumsKitInfo {
    pads: KitPadsHandle,
}

impl KitInfoSource for DrumsKitInfo {
    fn kit_info_json(&self) -> Option<String> {
        Some(kit_info(&self.pads).to_json())
    }
}

/// The pads `handle` holds, as the extension reports them: one per pad
/// slot, in slot order, on the slot's note.
pub fn kit_info(handle: &KitPadsHandle) -> KitInfo {
    let current = handle.current();
    KitInfo {
        from_kit: current.from_kit,
        pads: current
            .pads
            .iter()
            .zip(PAD_MAPPINGS.iter())
            .map(|(pad, mapping)| KitInfoPad {
                note: mapping.note,
                name: pad.name.clone(),
                present: pad.present,
            })
            .collect(),
    }
}

/// The [`resonance_plugin::ResonancePlugin::kit_info_source`] of an
/// instance whose kit pads are `bridge.kit_pads`.
pub fn source(bridge: &crate::KitBridge) -> Option<Arc<dyn KitInfoSource>> {
    Some(Arc::new(DrumsKitInfo {
        pads: bridge.kit_pads.clone(),
    }))
}
