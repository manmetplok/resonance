//! No parameter the plugin advertises may be inert.
//!
//! ba todo #1279 marked the per-pad articulation params " (editor only)"
//! because nothing read them — the loader took its articulations from a
//! separate array that only the editor chips wrote. ba todo #1325 made
//! the parameter the source of truth (see `articulation_param.rs`), so
//! the marker is gone and must not come back for anything else either:
//! the fix for a control that does nothing is to make it work or remove
//! it, not to relabel it.

use resonance_drums::drum_map::NUM_PADS;
use resonance_drums::params::{DrumParams, GLOBAL_PARAMS, PARAMS_PER_PAD};
use resonance_drums::ResonanceDrums;
use resonance_plugin::param::Param;
use resonance_plugin::ResonancePlugin;

/// The retired marker. Kept as a literal here (rather than imported)
/// precisely so this test still fails if someone reintroduces it.
const RETIRED_MARKER: &str = "(editor only)";

#[test]
fn no_parameter_is_marked_editor_only() {
    let plugin = ResonanceDrums::new();
    for index in 0..plugin.param_count() {
        let param = plugin.param(index);
        assert!(
            !param.name().contains(RETIRED_MARKER),
            "'{}' is advertised to the host but marked as doing nothing",
            param.id()
        );
    }
}

/// The marker only ever lived in the display name — the string ids are
/// what saved projects and automation lanes key on, and #1325 changed
/// none of them.
#[test]
fn articulation_param_ids_are_unchanged() {
    let params = DrumParams::default();
    for (index, pad) in params.pads.iter().enumerate() {
        assert_eq!(pad.articulation.id(), format!("pad_{index}_articulation"));
        assert_eq!(pad.articulation.name(), format!("Pad {index} Articulation"));
    }
}

/// The pad block still runs in the same order behind the globals, so
/// `param(index)` and `PARAMS_PER_PAD` stay in step. (The flat index is
/// enumeration order only — hosts key on the string id, which the CLAP
/// bridge hashes to a stable numeric id.)
#[test]
fn param_layout_is_consistent() {
    let plugin = ResonanceDrums::new();
    assert_eq!(
        plugin.param_count(),
        GLOBAL_PARAMS + NUM_PADS * PARAMS_PER_PAD
    );
    for pad in 0..NUM_PADS {
        let base = GLOBAL_PARAMS + pad * PARAMS_PER_PAD;
        assert_eq!(plugin.param(base).id(), format!("pad_{pad}_level"));
        assert_eq!(
            plugin.param(base + 3).id(),
            format!("pad_{pad}_articulation")
        );
    }
}

/// Every parameter has a unique id — the CLAP bridge refuses to load a
/// plugin whose ids collide, and the globals added by ba todo #1326 sit
/// in the same namespace as the pad params.
#[test]
fn every_param_id_is_unique() {
    let plugin = ResonanceDrums::new();
    let mut ids: Vec<&str> = (0..plugin.param_count())
        .map(|i| plugin.param(i).id())
        .collect();
    let total = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), total, "duplicate parameter id");
}
