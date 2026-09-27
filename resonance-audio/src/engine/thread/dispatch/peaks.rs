//! Peak meter polling: snapshot and emit all per-track, per-bus, and master
//! peak levels in a single `PeakSnapshot` event.

use std::sync::atomic::Ordering;

use crate::types::*;

use super::super::HandlerCtx;

/// Snapshot and clear every peak meter (per-track, per-bus, master L/R)
/// and dispatch a `PeakSnapshot` event. Runs on the engine thread and
/// reads the published render graph (tracks, busses — no lock since
/// ARCH-02 A2-6), and the GUI side reads its result via the regular event
/// queue.
pub(super) fn handle_poll_peaks(ctx: &HandlerCtx) {
    let track_peaks = ctx
        .tracks()
        .values()
        .map(|t| (t.id, t.swap_peak_l(), t.swap_peak_r()))
        .collect();
    let bus_peaks = ctx
        .shared
        .graph
        .load()
        .busses
        .values()
        .map(|b| (b.id, b.swap_peak_l(), b.swap_peak_r()))
        .collect();
    let master_peak_l =
        f32::from_bits(ctx.shared.master_peak_l_bits.swap(0, Ordering::AcqRel));
    let master_peak_r =
        f32::from_bits(ctx.shared.master_peak_r_bits.swap(0, Ordering::AcqRel));
    let _ = ctx.event_tx.send(AudioEvent::PeakSnapshot {
        track_peaks,
        bus_peaks,
        master_peak_l,
        master_peak_r,
    });
}
