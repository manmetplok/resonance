//! Per-pad articulation: the choice labels, and the watcher that turns a
//! parameter change into a kit reload.
//!
//! # One source of truth
//!
//! The articulation of a pad is the value of its `pad_N_articulation`
//! parameter — nothing else. The editor chips write that parameter, host
//! automation writes that parameter, and `set_plugin_param` over the
//! control API writes that parameter; all three then take the identical
//! path described below. (Before ba todo #1325 the editor wrote a
//! separate `KitBridge::articulations` array that the loader read, so
//! only the chips did anything.)
//!
//! # How a parameter change becomes sound
//!
//! Switching articulation means decoding a different piece of the kit
//! from disk, which can take seconds — so it cannot happen on the audio
//! thread, and a parameter write has no callback to hang it off: host
//! automation lands in the params inside `process()`, and the editor
//! writes them from the UI thread.
//!
//! So one watcher thread per plugin instance compares the parameters
//! against [`KitBridge::loaded_articulations`] — the set the kit in
//! memory was actually built with — and spawns a loader when they
//! differ on a pad the kit pairs ([`masked`]): the articulation of a pad
//! with no alternate in this kit moves nothing. It sleeps on a channel between checks: the editor pings that
//! channel so a chip click reloads immediately, and the poll timeout
//! covers writers that cannot ping it (automation, the control API).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError};

use crate::drum_map::NUM_PADS;
use crate::pad_map::KitPads;
use crate::reload::reload_kit;
use crate::KitBridge;

/// The generic labels of the per-pad articulation choice, indexed by
/// parameter value: 0 plays the pad's primary piece, 1 its alternate.
///
/// A CLAP parameter declares its range once, so the values stay generic;
/// the words come from the kit. Which piece is the alternate, and what
/// the two are called ("punch" / "deep" in IT Techno, "mit Teppich" /
/// "ohne Teppich" in Drummica), is [`crate::pad_map`]'s: the editor chips
/// show the kit's labels, and so does the parameter's text once the
/// plugin has attached the kit to it ([`attach_kit_labels`]). These two
/// are what it reads when the kit has no articulation for the pad.
pub const ARTICULATION_LABELS: &[&str] = &["Primary", "Alternate"];

/// Have every pad's articulation parameter display (and parse) the
/// current kit's chip labels, falling back to [`ARTICULATION_LABELS`].
pub fn attach_kit_labels(params: &crate::params::DrumParams, kit_pads: &crate::pad_map::KitPadsHandle) {
    for (slot, pad) in params.pads.iter().enumerate() {
        let kit_pads = kit_pads.clone();
        pad.articulation
            .set_text(Arc::new(move |value| kit_pads.articulation_text(slot, value)));
    }
}

/// Parameter value for the primary articulation (the default).
pub const ARTICULATION_PRIMARY: i32 = 0;
/// Parameter value for the alternate articulation.
pub const ARTICULATION_ALT: i32 = 1;

/// How long the watcher sleeps between checks when nobody pings it.
/// Only writers that cannot reach the wake channel (host automation,
/// the control API) wait this long, and they wait it out in front of a
/// kit decode that is orders of magnitude longer.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Keeps the articulation watcher thread alive. Dropping it (i.e.
/// dropping the plugin) tells the thread to exit at its next wake.
///
/// The thread holds a `KitBridge` clone, and the bridge holds the wake
/// channel's sender, so channel disconnection alone can never end it —
/// hence the explicit flag.
pub struct ArticulationWatcher {
    alive: Arc<AtomicBool>,
}

impl Drop for ArticulationWatcher {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

/// Spawn the watcher for `bridge`. `wake` is the receiving end of
/// [`KitBridge::articulation_wake`].
pub fn spawn_watcher(bridge: &KitBridge, wake: Receiver<()>) -> ArticulationWatcher {
    let alive = Arc::new(AtomicBool::new(true));
    let thread_alive = alive.clone();
    let bridge = bridge.clone();

    let spawned = std::thread::Builder::new()
        .name("resonance-drums-articulation".to_string())
        .spawn(move || loop {
            match wake.recv_timeout(POLL_INTERVAL) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if !thread_alive.load(Ordering::Acquire) {
                break;
            }
            // One look at everything a host or the control API may have
            // moved — `stream_preload`, `kit_select` (drums-plugin-rework.md
            // §5.1), the articulations — starting at most one load.
            crate::selection::watch(&bridge);
        });

    if spawned.is_err() {
        // Without the watcher the editor chips still reload directly, so
        // degrade rather than refuse to instantiate the plugin.
        eprintln!("resonance-drums: could not spawn the articulation watcher thread");
    }

    ArticulationWatcher { alive }
}

/// `articulations` with every pad `pads` does not pair cleared: on such a
/// pad both values play the same piece, so its parameter is no part of
/// what a load decodes (the loader masks it the same way per pad).
pub fn masked(articulations: [bool; NUM_PADS], pads: &KitPads) -> [bool; NUM_PADS] {
    std::array::from_fn(|i| {
        articulations[i] && pads.pads.get(i).is_some_and(|pad| pad.articulation.is_some())
    })
}

/// Reload the kit if the articulation parameters no longer match what
/// the loaded kit was built from. Returns true when a load was started.
///
/// Public so the editor can apply a chip click without waiting for the
/// watcher's next wake, and so tests can drive the same step the watcher
/// drives. Takes [`crate::selection::KitSelection::acting`], so it never
/// interleaves with a `kit_select` act or a state load.
pub fn apply_pending(bridge: &KitBridge) -> bool {
    let _acting = bridge.params.selection.acting();
    apply_pending_locked(bridge, false)
}

/// [`apply_pending`] for a caller holding
/// [`crate::selection::KitSelection::acting`]. `also_reload`: something
/// else the kit is built from moved too (the streaming preload), so reload
/// even when the articulations did not — one load for both.
///
/// The comparison is over the pads the kit in place pairs
/// ([`KitPadsHandle::current`](crate::pad_map::KitPadsHandle::current)):
/// moving the articulation of a pad the kit has no alternate for reloads
/// nothing (it would fade every voice out, restart the round robins and
/// drop `kit_load_progress` to 0, to build the same kit).
pub fn apply_pending_locked(bridge: &KitBridge, also_reload: bool) -> bool {
    let wanted = bridge.articulations();
    let pads = bridge.kit_pads.current();
    // Take the comparison out of the `if` condition: the temporary guard
    // would otherwise live until the end of the statement, and
    // `spawn_loader` locks the same mutex.
    let loaded = *bridge.loaded_articulations.lock();
    let unchanged = masked(loaded, &pads) == masked(wanted, &pads);
    if unchanged && !also_reload {
        // Not adopted: once another kit pairs the pad, the raw
        // difference is a change to load.
        return false;
    }
    if reload_kit(bridge) {
        // `spawn_loader` records what it was handed, so the next check
        // compares against this load rather than re-triggering.
        return true;
    }
    // Nothing to reload (no kit chosen yet, or the host has not
    // activated the plugin). Whatever loads later reads the parameters
    // as they are then, so adopt the value now instead of retrying
    // every poll.
    *bridge.loaded_articulations.lock() = wanted;
    false
}
