//! Kit reload helper used by the editor sub-modules.
//!
//! Reloads the kit in place with the current mic / articulation / pad-choice
//! state. Triggered whenever the user changes a close-mic, overhead-mic,
//! articulation, or any other setup that needs the sample banks to be
//! re-decoded. No-op if there's no kit path yet or the host hasn't activated
//! the plugin.

use std::sync::atomic::Ordering;

use crate::{kit_loader, KitBridge};

pub(crate) fn reload_kit(bridge: &KitBridge) {
    let path = match bridge.kit_path.lock().clone() {
        Some(p) => p,
        None => return,
    };
    let sr_bits = bridge.sample_rate.load(Ordering::Acquire);
    if sr_bits == 0 {
        return;
    }
    let target_sr = f32::from_bits(sr_bits);
    let overhead_key = bridge.overhead_setup_key.lock().clone();
    let choices = bridge.pad_choices.lock().clone();
    let articulations = *bridge.articulations.lock();
    kit_loader::spawn_loader(
        path,
        target_sr,
        bridge,
        overhead_key,
        choices,
        articulations,
    );
}
