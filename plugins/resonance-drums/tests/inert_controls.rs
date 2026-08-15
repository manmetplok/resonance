//! Controls the plugin draws but does not act on must say so (ba todo
//! #1279). Today that is the per-pad articulation parameter: the DSP and
//! the loader read `KitBridge::articulations`, so a host automation lane
//! on `pad_N_articulation` does nothing. Until ba todo #1325 makes the
//! param the source of truth, every one of them is marked in its name.

use resonance_drums::drum_map::NUM_PADS;
use resonance_drums::params::{DrumParams, EDITOR_ONLY_SUFFIX, PARAMS_PER_PAD};
use resonance_drums::ResonanceDrums;
use resonance_plugin::param::Param;
use resonance_plugin::ResonancePlugin;

/// Every articulation param is marked editor-only in its display name —
/// the name the host's parameter list and MCP `plugin_params` show.
#[test]
fn articulation_params_are_marked_editor_only() {
    let params = DrumParams::default();
    for (index, pad) in params.pads.iter().enumerate() {
        assert_eq!(
            pad.articulation.name(),
            format!("Pad {index} Articulation{EDITOR_ONLY_SUFFIX}"),
            "pad {index} articulation must be marked as doing nothing"
        );
    }
}

/// The marker lives in the name only. The string id is what saved
/// projects and automation lanes key on, so #1325 can make the param real
/// without breaking them.
#[test]
fn articulation_param_ids_are_unchanged() {
    let params = DrumParams::default();
    for (index, pad) in params.pads.iter().enumerate() {
        assert_eq!(pad.articulation.id(), format!("pad_{index}_articulation"));
    }
}

/// No *other* parameter carries the marker — the ones that are marked are
/// exactly the ones nothing reads.
#[test]
fn only_articulation_params_are_marked() {
    let plugin = ResonanceDrums::new();
    let mut marked = 0;
    for index in 0..plugin.param_count() {
        let param = plugin.param(index);
        if param.name().ends_with(EDITOR_ONLY_SUFFIX) {
            marked += 1;
            assert!(
                param.id().ends_with("_articulation"),
                "unexpected editor-only marker on '{}'",
                param.id()
            );
        }
    }
    assert_eq!(marked, NUM_PADS, "one marked param per pad");
    assert_eq!(plugin.param_count(), 1 + NUM_PADS * PARAMS_PER_PAD);
}
