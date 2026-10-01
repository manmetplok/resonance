//! Kit reload helper.
//!
//! Reloads the kit in place with the current mic / articulation /
//! pad-choice state. Triggered whenever something changes that needs the
//! sample banks re-decoded: a close-mic pick, the overhead pick, or an
//! articulation parameter (see [`crate::articulation`]). No-op if there
//! is no kit path yet or the host hasn't activated the plugin.
//!
//! Lives outside the `editor` module because the articulation watcher
//! calls it in headless builds too — a host automation lane or a
//! `set_plugin_param` call must switch samples with no GUI open.

use std::sync::atomic::Ordering;

use crate::{kit_loader, KitBridge};

/// Spawn a loader for the current setup. Returns false when there is
/// nothing to reload — no kit chosen, or no sample rate known yet.
pub fn reload_kit(bridge: &KitBridge) -> bool {
    // The wanted kit, not `kit_path`: a mic change while a pick is still
    // decoding must reload the pick, not revert to the kit before it.
    let path = match bridge.wanted_kit_path() {
        Some(p) => p,
        None => return false,
    };
    let sr_bits = bridge.sample_rate.load(Ordering::Acquire);
    if sr_bits == 0 {
        return false;
    }
    let target_sr = f32::from_bits(sr_bits);
    let overhead_key = bridge.overhead_setup_key.lock().clone();
    let choices = bridge.pad_choices.lock().clone();
    let articulations = bridge.articulations();
    kit_loader::spawn_loader(
        path,
        target_sr,
        bridge,
        overhead_key,
        choices,
        articulations,
    );
    true
}
