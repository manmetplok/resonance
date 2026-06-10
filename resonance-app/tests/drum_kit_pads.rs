//! Pins the drumroll kit pad library against the shared GM drum-note
//! contract in `resonance_common::drum_map`.
//!
//! The kit library (`default_kit_pads`) is the picker source for the
//! Drum Groups Manager, and the notes it hands out are what the drum
//! lane renders into MIDI for the `resonance-drums` plugin. These tests
//! lock in the reconciliation: every plugin pad appears exactly once
//! under its canonical name, the only non-plugin pads are the documented
//! external-GM percussion extras, and the default drum groups never
//! reference a note outside the library or duplicate a note internally.

use std::collections::HashSet;

use resonance_app::compose::drumroll::{default_drum_groups, default_kit_pads};
use resonance_common::drum_map::{pad_index_for_note, GM_PADS};

#[test]
fn kit_pads_have_unique_notes() {
    let pads = default_kit_pads();
    let mut seen = HashSet::new();
    for pad in &pads {
        assert!(
            seen.insert(pad.note),
            "kit pad library has duplicate note {} ({})",
            pad.note,
            pad.name
        );
    }
}

#[test]
fn every_plugin_pad_is_in_the_kit_library_with_its_canonical_name() {
    let pads = default_kit_pads();
    for gm_pad in GM_PADS.iter() {
        let kit = pads
            .iter()
            .find(|p| p.note == gm_pad.note)
            .unwrap_or_else(|| {
                panic!(
                    "plugin pad {} (note {}) missing from kit library",
                    gm_pad.name, gm_pad.note
                )
            });
        assert_eq!(
            kit.name, gm_pad.name,
            "kit pad at note {} disagrees with the canonical GM_PADS name",
            gm_pad.note
        );
    }
}

#[test]
fn non_plugin_kit_pads_are_external_gm_perc_only() {
    // Pads without a resonance-drums pad index are the documented
    // external-GM extras: cowbell, shaker, conga, tambourine. They must
    // all sit in "Perc" so the picker groups them away from the
    // plugin-backed categories.
    for pad in default_kit_pads() {
        if pad_index_for_note(pad.note).is_none() {
            assert_eq!(
                pad.category, "Perc",
                "pad {} (note {}) has no plugin pad but isn't external perc",
                pad.name, pad.note
            );
            assert!(
                matches!(pad.name.as_str(), "Cowbell" | "Shaker" | "Conga" | "Tambourine"),
                "unexpected non-plugin pad {} (note {}) in kit library",
                pad.name,
                pad.note
            );
        }
    }
}

#[test]
fn default_group_pads_resolve_to_kit_pads_without_internal_duplicates() {
    let kit: HashSet<u8> = default_kit_pads().iter().map(|p| p.note).collect();
    let mut next_id = 0u64;
    for group in default_drum_groups(&mut next_id) {
        let mut seen = HashSet::new();
        for pad in &group.pads {
            assert!(
                kit.contains(&pad.note),
                "group {} pad {} uses note {} that isn't in the kit library",
                group.name,
                pad.name,
                pad.note
            );
            assert!(
                seen.insert(pad.note),
                "group {} has two pads on note {} — they'd trigger the same sample",
                group.name,
                pad.note
            );
        }
    }
}
