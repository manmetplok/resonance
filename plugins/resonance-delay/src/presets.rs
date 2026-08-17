//! Factory presets.
//!
//! Every entry is a **full snapshot**: all [`PARAM_COUNT`] ids are
//! present. The shared loader ([`resonance_plugin::presets::load`])
//! only writes ids it finds in the map, so a partial preset silently
//! inherits whatever the previous patch left behind — loading
//! "Slapback" over a trance-gated patch used to leave the gate
//! running. `tests/presets.rs` enforces the full-coverage rule so a
//! newly added parameter cannot quietly reintroduce a partial recall.

use crate::params::{DelayParams, PARAM_COUNT};

/// The factory-preset entry type is the shared one, so this crate's
/// `PRESETS` can be handed straight to `presets::PresetBank` (ba todo
/// #1358). The alias keeps the crate-local name every call site uses.
pub use resonance_plugin::presets::FactoryPreset as PresetEntry;

/// Apply a preset JSON blob to `params`. Returns `false` when the blob
/// is not a `{"params": {...}}` object.
pub fn load_preset(params: &DelayParams, json: &str) -> bool {
    resonance_plugin::presets::load(json, PARAM_COUNT, |i| params.param_at(i))
}

pub const PRESETS: &[PresetEntry] = &[
    PresetEntry {
        name: "Quarter Note",
        json: r#"{"params":{"sync":1.0,"division":4.0,"time_ms":375.0,"feedback":0.35,"mix":0.35,"character":0.0,"routing":0.0,"stereo_offset":0.0,"hi_cut":8000.0,"lo_cut":120.0,"drive":0.1,"mod_rate":0.4,"mod_depth":0.05,"freeze":0.0,"gate_on":0.0,"gate_rate":7.0,"gate_width":0.5,"gate_shape":0.05,"gate_depth":1.0,"duck_amount":0.0,"duck_threshold":-24.0,"duck_release":200.0}}"#,
    },
    PresetEntry {
        name: "Dotted Eighth",
        json: r#"{"params":{"sync":1.0,"division":8.0,"time_ms":375.0,"feedback":0.40,"mix":0.35,"character":0.0,"routing":0.0,"stereo_offset":0.0,"hi_cut":8000.0,"lo_cut":120.0,"drive":0.1,"mod_rate":0.4,"mod_depth":0.05,"freeze":0.0,"gate_on":0.0,"gate_rate":7.0,"gate_width":0.5,"gate_shape":0.05,"gate_depth":1.0,"duck_amount":0.0,"duck_threshold":-24.0,"duck_release":200.0}}"#,
    },
    PresetEntry {
        name: "Slapback",
        json: r#"{"params":{"sync":0.0,"division":4.0,"time_ms":80.0,"feedback":0.15,"mix":0.50,"character":0.0,"routing":0.0,"stereo_offset":0.0,"hi_cut":10000.0,"lo_cut":80.0,"drive":0.05,"mod_rate":0.3,"mod_depth":0.02,"freeze":0.0,"gate_on":0.0,"gate_rate":7.0,"gate_width":0.5,"gate_shape":0.05,"gate_depth":1.0,"duck_amount":0.0,"duck_threshold":-24.0,"duck_release":200.0}}"#,
    },
    PresetEntry {
        name: "Dub",
        json: r#"{"params":{"sync":1.0,"division":4.0,"time_ms":375.0,"feedback":0.65,"mix":0.40,"character":1.0,"routing":0.0,"stereo_offset":0.0,"hi_cut":3000.0,"lo_cut":200.0,"drive":0.35,"mod_rate":0.5,"mod_depth":0.15,"freeze":0.0,"gate_on":0.0,"gate_rate":7.0,"gate_width":0.5,"gate_shape":0.05,"gate_depth":1.0,"duck_amount":0.0,"duck_threshold":-24.0,"duck_release":200.0}}"#,
    },
    PresetEntry {
        name: "Ping-Pong Eighth",
        json: r#"{"params":{"sync":1.0,"division":7.0,"time_ms":375.0,"feedback":0.45,"mix":0.40,"character":0.0,"routing":1.0,"stereo_offset":0.0,"hi_cut":8000.0,"lo_cut":120.0,"drive":0.1,"mod_rate":0.4,"mod_depth":0.05,"freeze":0.0,"gate_on":0.0,"gate_rate":7.0,"gate_width":0.5,"gate_shape":0.05,"gate_depth":1.0,"duck_amount":0.0,"duck_threshold":-24.0,"duck_release":200.0}}"#,
    },
    PresetEntry {
        name: "Lo-Fi Tape",
        json: r#"{"params":{"sync":0.0,"division":4.0,"time_ms":350.0,"feedback":0.55,"mix":0.40,"character":1.0,"routing":0.0,"stereo_offset":0.1,"hi_cut":2500.0,"lo_cut":180.0,"drive":0.25,"mod_rate":0.6,"mod_depth":0.30,"freeze":0.0,"gate_on":0.0,"gate_rate":7.0,"gate_width":0.5,"gate_shape":0.05,"gate_depth":1.0,"duck_amount":0.0,"duck_threshold":-24.0,"duck_release":200.0}}"#,
    },
];
