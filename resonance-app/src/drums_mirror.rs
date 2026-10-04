//! What the app reads about a Resonance Drums instance from its mirrored
//! parameters (drums-plugin-rework.md §8, slice K9): which kit it has
//! selected (`kit_select`'s value and text) and whether it routes its pads
//! to separate outputs (`output_mode`).
//!
//! Every read goes through the param mirror the engine keeps current
//! (`PluginAdded`, values/text rescans, edits), so none of it touches the
//! plugin. The keys are the drums' stable param keys; their CLAP ids are
//! [`resonance_plugin::stable_hash`] of them.

use resonance_audio::types::ParamInfo;

use crate::state::{PluginSlotState, TrackState};

// The drums' id and host-read keys are the plugin's own declarations
// (`resonance_plugin::first_party`, code review ARCH2-03), which the
// drums' tests pin to real params.
pub use resonance_plugin::first_party::drums::{KIT_LOAD_PROGRESS, KIT_SELECT, OUTPUT_MODE};
/// Resonance Drums' CLAP id.
pub use resonance_plugin::first_party::DRUMS as DRUMS_PLUGIN_ID;
/// [`OUTPUT_MODE`]'s Multi step, as the mirrored param value.
pub const OUTPUT_MODE_MULTI: f64 =
    resonance_plugin::first_party::drums::OUTPUT_MODE_MULTI as f64;

/// One of a slot's params by its stable key.
pub fn param<'a>(slot: &'a PluginSlotState, key: &str) -> Option<&'a ParamInfo> {
    let id = resonance_plugin::stable_hash(key);
    slot.params.iter().find(|p| p.id == id)
}

/// Mirror a write of `value` to `slot`'s `kit_select`. When it names
/// another kit, the mirrored `kit_load_progress` drops to 0 at once: the
/// plugin reports 0 until its watcher acts on the write, but that reaches
/// the mirror only with its next rescan, and a read in between would see
/// the previous kit's 1.0 and take the new kit for loaded. Returns
/// whether the value changed. The caller has checked `param_id` is
/// `kit_select` on a drums slot.
pub fn mirror_kit_select(slot: &mut PluginSlotState, value: f64) -> bool {
    let id = resonance_plugin::stable_hash(KIT_SELECT);
    let Some(param) = slot.params.iter_mut().find(|p| p.id == id) else {
        return false;
    };
    let changed = param.current_value.round() != value.round();
    param.current_value = value;
    if changed {
        let progress = resonance_plugin::stable_hash(KIT_LOAD_PROGRESS);
        if let Some(p) = slot.params.iter_mut().find(|p| p.id == progress) {
            p.current_value = 0.0;
            p.text = "0%".to_owned();
        }
    }
    changed
}

/// Whether `slot` is a Resonance Drums instance.
pub fn is_drums(slot: &PluginSlotState) -> bool {
    slot.clap_plugin_id == DRUMS_PLUGIN_ID
}

/// The track's Resonance Drums instance, if any (its instrument, or the
/// first one in its chain).
pub fn drums_slot(track: &TrackState) -> Option<&PluginSlotState> {
    track.plugins.iter().find(|s| is_drums(s))
}

/// The library slot the instance has selected; `None` for the built-in kit,
/// a parked (missing/external) kit, or a non-drums slot.
pub fn kit_slot(slot: &PluginSlotState) -> Option<u32> {
    if !is_drums(slot) {
        return None;
    }
    let v = param(slot, KIT_SELECT)?.current_value.round();
    (v >= 0.0).then_some(v as u32)
}

/// The name of the kit the instance has selected, as `kit_select`'s text
/// reads ("Drummica", "(missing)", "(external)", the built-in kit's name).
/// `None` for a non-drums slot or before the plugin reported any text.
pub fn kit_name(slot: &PluginSlotState) -> Option<&str> {
    if !is_drums(slot) {
        return None;
    }
    let text = param(slot, KIT_SELECT)?.text.trim();
    (!text.is_empty()).then_some(text)
}

/// Whether the instance is in Multi output mode. A drums instance whose
/// mirror has no `output_mode` (a build before E11 had only multi-out) and
/// any other plugin answer `true`: their extra ports always carry audio.
pub fn routes_to_ports(slot: &PluginSlotState) -> bool {
    if !is_drums(slot) {
        return true;
    }
    match param(slot, OUTPUT_MODE) {
        Some(p) => p.current_value.round() >= OUTPUT_MODE_MULTI,
        None => true,
    }
}
