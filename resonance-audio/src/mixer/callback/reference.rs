//! Reference A/B monitor branch.

use std::sync::atomic::Ordering;

use crate::mixer::common::advance_playhead_silent;

use super::context::{CallbackInputs, CallbackScratch};

/// When the user has switched the monitored source to a loaded reference,
/// replace the entire output with the reference PCM, bypassing the mix +
/// master/mastering chain (this is the post-master monitor tap), and
/// report `true` so the callback stops there.
///
/// Suppressed while recording or counting in so a realtime bounce /
/// punch-in monitors the live signal, not the reference. The
/// offline/realtime bounce render paths never consult `shared.reference`,
/// so exports stay the processed mix regardless of this selection.
pub(super) fn monitor_reference(
    inputs: &CallbackInputs<'_>,
    scratch: &mut CallbackScratch<'_>,
    frames: usize,
) -> bool {
    let shared = inputs.shared;
    if shared.recording.load(Ordering::Relaxed) || shared.count_in_active.load(Ordering::Relaxed) {
        return false;
    }
    let playhead = shared.playhead.load(Ordering::Relaxed);
    // Latency-match the reference against the mix (doc #260 finding
    // #19): the processed mix at this output position is the timeline of
    // `max comp latency + master-chain latency` ago, so in loop-to-mix
    // mode the reference reads from that delayed position — toggling A/B
    // then produces no timing jump. Free-run mode ignores the playhead
    // entirely.
    let ab_delay = inputs.latency_comp.load().max_latency()
        + shared.master_latency_samples.load(Ordering::Relaxed);
    let channels = inputs.channels;
    if !shared.reference.render(
        &mut scratch.data[..frames * channels],
        channels,
        frames,
        playhead.saturating_sub(ab_delay),
    ) {
        return false;
    }
    // Meter the reference exactly as monitored — post loudness-match /
    // trim gain — so the panel's Delta against the mix is honest.
    scratch
        .ab_meters
        .reference
        .feed_interleaved(&scratch.data[..frames * channels], channels, frames);
    shared.ref_meter.store(&scratch.ab_meters.reference.snapshot());
    // Keep the transport rolling while playing so loop-to-mix tracking
    // and a later switch back to the mix resume at the right spot; when
    // stopped the reference free-runs (audition) and the playhead stays
    // put.
    if shared.playing.load(Ordering::Relaxed) {
        let new_playhead = advance_playhead_silent(shared, playhead, frames as u64);
        shared.playhead.store(new_playhead, Ordering::Relaxed);
    }
    true
}
