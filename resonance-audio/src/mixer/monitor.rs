//! Monitor input: how live capture reaches the mix.
//!
//! Two halves, both owned here:
//!
//! - **Ring pacing** — [`monitor_catchup_skip`], [`monitor_read_len`] and
//!   [`MonitorDrain`] decide how much of the capture ring this callback
//!   skips and reads, in whole frames only. Pure functions plus one small
//!   piece of per-session state; the callback's
//!   [`read_monitor_input`](super::callback::monitor_input::read_monitor_input)
//!   drives them.
//! - **Pass-through** — [`mix_monitor_passthrough`] de-interleaves the
//!   input stream, routes each track's chosen channel(s) into its stereo
//!   L/R pair, runs the track's plugin chain and sums the result into the
//!   output. Used by the no-playback / count-in branches of the callback,
//!   which keep every audible monitored track flowing through to the
//!   master so the performer can hear themselves; the playing-back
//!   timeline path mixes monitor input inside `render_core` instead.


use crate::bypass::{run_faded, FadeStage, FxDryScratch};
use crate::clap_host::PluginMap;
use crate::types::*;

use super::common::{
    latch_transport, ramped_stereo_peaks, sum_to_output, track_stereo_gains, TransportSnap,
};
use super::midi_stash::MidiStash;

// ---------------------------------------------------------------------------
// Ring pacing
// ---------------------------------------------------------------------------

/// Whole-frame catch-up skip for the monitor ring: when `available`
/// exceeds `needed` plus one quantum of jitter margin, skip down to
/// that margin (never to exactly `needed`, which would re-overflow on
/// the next push) in whole frames only.
#[inline]
pub fn monitor_catchup_skip(
    available: usize,
    needed: usize,
    quantum: usize,
    frame_stride: usize,
) -> usize {
    let target = needed + quantum * frame_stride;
    if available > target {
        (available - target) / frame_stride * frame_stride
    } else {
        0
    }
}

/// Whole-frame read length for the monitor ring.
#[inline]
pub fn monitor_read_len(needed: usize, occupied: usize, frame_stride: usize) -> usize {
    needed.min(occupied / frame_stride * frame_stride)
}

/// Adaptive monitor-ring backlog drain for the native PipeWire backend
/// (doc #260 finding #12). With input and output streams in the same
/// graph on the same clock, pushes and reads are strictly 1:1 — the
/// only backlog the ring *needs* is the intra-cycle ordering bound
/// ([`monitor_catchup_skip`]'s one-quantum margin covers a read that
/// runs before that cycle's push). A startup burst can still leave one
/// sticky extra quantum that the margin skip never reclaims. This
/// tracker watches for backlog that stays above `needed` for
/// [`MONITOR_DRAIN_STREAK`] consecutive callbacks — only a stable
/// scheduling order produces that — and then drains the excess down to
/// `needed`, converging the ring to its true minimum (0 or 1 cycle
/// depending on ordering). Any low cycle resets the streak, so jittery
/// ordering keeps the full margin. Inactive on the cpal fallback,
/// whose independent clock genuinely needs the standing margin.
///
/// Zero margin is a gamble on that ordering staying stable: the input
/// and output streams run on independent RT data loops with no
/// ordering edge between them, so a cycle where the output callback
/// runs before that cycle's input push finds the ring empty and drops
/// a full quantum of monitored input — an audible click with no graph
/// xrun anywhere. On a graph where that happens the drain would
/// restore the vulnerability ~43 ms later, clicking on every
/// subsequent flip. So the first shortfall *after* a drain
/// ([`Self::note_shortfall`]) locks the drain out for the rest of the
/// session: the standing one-quantum margin absorbs all further flips,
/// trading +1 quantum of monitor latency for silence-free monitoring.
/// Shortfalls before any drain (the ring filling at startup) don't
/// lock out — they're not evidence about ordering stability, and
/// locking on them would forfeit the latency win on every session.
pub struct MonitorDrain {
    native: bool,
    high_streak: u32,
    /// True once this session has drained to zero margin at least once.
    drained: bool,
    /// True once a post-drain shortfall proved the scheduling order
    /// unstable; no further drains this session.
    locked_out: bool,
}

/// Consecutive high-backlog callbacks before the excess is drained:
/// ~43 ms at 48 kHz / q128 — long enough to prove a stable scheduling
/// order, short enough to reclaim the latency promptly after startup.
pub const MONITOR_DRAIN_STREAK: u32 = 16;

impl MonitorDrain {
    pub fn new(native: bool) -> Self {
        Self {
            native,
            high_streak: 0,
            drained: false,
            locked_out: false,
        }
    }

    /// Whole-frame sample count to drain beyond the margin skip, given
    /// the ring occupancy right before this callback's read. Non-zero
    /// only on the native backend after a full high streak, and never
    /// again after a post-drain shortfall locked the drain out.
    pub fn excess_drain(&mut self, available: usize, needed: usize, frame_stride: usize) -> usize {
        if !self.native || self.locked_out {
            return 0;
        }
        if available > needed {
            self.high_streak += 1;
        } else {
            self.high_streak = 0;
            return 0;
        }
        if self.high_streak < MONITOR_DRAIN_STREAK {
            return 0;
        }
        self.high_streak = 0;
        let drain = (available - needed) / frame_stride.max(1) * frame_stride.max(1);
        if drain > 0 {
            self.drained = true;
        }
        drain
    }

    /// The mixer read came up short while monitoring. After at least
    /// one drain that means the zero-margin gamble lost on this graph:
    /// lock the drain out so the standing margin absorbs further
    /// ordering flips.
    pub fn note_shortfall(&mut self) {
        if self.drained {
            self.locked_out = true;
        }
    }
}

// ---------------------------------------------------------------------------
// Pass-through
// ---------------------------------------------------------------------------

/// De-interleave monitor input into track buffers and process through plugins.
/// Returns the number of frames written. `monitor_temp` is interleaved
/// multi-channel input audio (the raw stream straight from the device);
/// `input_channels` tells us how many channels are in each frame, and the
/// track's own `input_port` picks which channel(s) to route into its
/// stereo L/R pair.
#[allow(clippy::too_many_arguments)]
fn process_monitor_track(
    track: &Track,
    monitor_temp: &[f32],
    monitor_frames: usize,
    max_frames: usize,
    input_channels: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    plugins_guard: &PluginMap,
    fx_dry: &mut FxDryScratch,
    transport_snap: Option<TransportSnap>,
    sample_rate: u32,
) -> usize {
    let is_mono = track.mono();
    let mix_frames = max_frames.min(monitor_frames);

    track_buf_l[..mix_frames].fill(0.0);
    track_buf_r[..mix_frames].fill(0.0);

    if input_channels == 0 {
        return mix_frames;
    }

    let port = (track.input_port() as usize).min(input_channels - 1);
    let right_port = if is_mono {
        port
    } else {
        (port + 1).min(input_channels - 1)
    };

    for f in 0..mix_frames {
        let base = f * input_channels;
        track_buf_l[f] = monitor_temp[base + port];
        track_buf_r[f] = monitor_temp[base + right_port];
    }

    run_track_chain(
        track,
        &mut track_buf_l[..mix_frames],
        &mut track_buf_r[..mix_frames],
        plugins_guard,
        fx_dry,
        transport_snap,
        sample_rate,
    );

    mix_frames
}

/// Run a track's whole plugin chain in place over `buf_l` / `buf_r`, off
/// the timeline (monitor pass-through, stopped-transport instruments).
/// Chain- and slot-level bypass go through the same click-free crossfade
/// as the arrangement render (`crate::bypass`), so toggling a bypass while
/// monitoring — which is exactly when a guitarist A/Bs an amp sim —
/// cannot click either.
fn run_track_chain(
    track: &Track,
    buf_l: &mut [f32],
    buf_r: &mut [f32],
    plugins_guard: &PluginMap,
    fx_dry: &mut FxDryScratch,
    transport_snap: Option<TransportSnap>,
    sample_rate: u32,
) {
    let frames = buf_l.len().min(buf_r.len());
    let chain_stage = track.fx_bypass().stage(sample_rate, frames, true);
    let (chain_dry, slot_dry) = fx_dry.split();
    run_faded(
        chain_stage,
        frames,
        (&mut buf_l[..frames], &mut buf_r[..frames]),
        chain_dry,
        |buf_l, buf_r| {
            let mut ran = false;
            let plugins = track.plugins();
            for &plugin_id in plugins.iter() {
                let Some(slot) = plugins_guard.get(&plugin_id) else {
                    continue;
                };
                let slot_stage = slot.stage(sample_rate, frames, true);
                if slot_stage == FadeStage::Dry {
                    continue;
                }
                let Some(mut inst) = slot.try_lock() else {
                    continue;
                };
                latch_transport(&mut inst, transport_snap);
                slot.sync_own_bypass(&mut inst.0);
                ran |= run_faded(
                    slot_stage,
                    frames,
                    (&mut *buf_l, &mut *buf_r),
                    (&mut *slot_dry.0, &mut *slot_dry.1),
                    |l, r| {
                        inst.0.process(l, r, frames);
                        true
                    },
                );
            }
            ran
        },
    );
}

/// Monitor pass-through for the count-in and stopped branches of
/// `mix_audio`: route every audible monitored track through its plugin
/// chain and sum it straight into the output with ramped gains and VU
/// peaks. Returns whether any track was mixed.
#[allow(clippy::too_many_arguments)]
pub(super) fn mix_monitor_passthrough(
    data: &mut [f32],
    channels: usize,
    tracks_guard: &TrackMap,
    plugins_guard: &PluginMap,
    monitor_temp: &[f32],
    monitor_frames: usize,
    input_channels: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    fx_dry: &mut FxDryScratch,
    transport_snap: Option<TransportSnap>,
    sample_rate: u32,
) -> bool {
    // Snapshot solo once (FU-B3a): `any_solo` and each track's own flag
    // below must come from the same instant, or a solo toggle mid-scan can
    // make every track look silenced.
    let any_solo = snapshot_top_level_solo(tracks_guard.values().map(|t| &**t));
    let is_audible = |t: &&Track| -> bool {
        t.monitor_enabled() && !t.muted() && (!any_solo || t.block_soloed())
    };
    let mut mixed_any = false;
    for track in tracks_guard.values().map(|t| &**t).filter(|t| is_audible(t)) {
        mixed_any = true;
        let processed_frames = process_monitor_track(
            track,
            monitor_temp,
            monitor_frames,
            monitor_frames,
            input_channels,
            track_buf_l,
            track_buf_r,
            plugins_guard,
            fx_dry,
            transport_snap,
            sample_rate,
        );
        let (target_l, target_r) = track_stereo_gains(track);
        let (last_l, last_r) = track.last_gains();
        let gain_l = (last_l, target_l);
        let gain_r = (last_r, target_r);
        // Post-fader peak levels for VU meters, with the same ramp the
        // sum applies.
        let (peak_l, peak_r) =
            ramped_stereo_peaks(track_buf_l, track_buf_r, processed_frames, gain_l, gain_r);
        track.update_peak_l(peak_l);
        track.update_peak_r(peak_r);
        sum_to_output(
            data,
            channels,
            processed_frames,
            track_buf_l,
            track_buf_r,
            gain_l,
            gain_r,
        );
        track.set_last_gains(target_l, target_r);
    }
    mixed_any
}

/// Stopped-transport instrument pass (code review MIX-08): process every
/// instrument that has live notes waiting or still releasing (see
/// `ClapInstance::wants_idle_process`), from silence, through its track's
/// chain and fader into the output. Piano-roll preview notes and a MIDI
/// controller played while stopped used to be queued into instruments
/// that were never processed — silent, piling up to the queue cap, then
/// bursting on the next Play. An idle instrument is not processed at all,
/// which bounds the cost.
///
/// `monitored` says whether [`mix_monitor_passthrough`] ran this block;
/// a track it already processed is skipped here. Muted / solo-suppressed
/// tracks still run (so their queue drains and voices release) but are
/// not heard. Returns whether any track was mixed audibly.
#[allow(clippy::too_many_arguments)]
pub(super) fn mix_idle_instruments(
    data: &mut [f32],
    channels: usize,
    frames: usize,
    tracks_guard: &TrackMap,
    plugins_guard: &PluginMap,
    midi_stash: &mut MidiStash,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    fx_dry: &mut FxDryScratch,
    transport_snap: Option<TransportSnap>,
    sample_rate: u32,
    monitored: bool,
) -> bool {
    // Snapshot solo once (FU-B3a): see `mix_monitor_passthrough` above —
    // `any_solo` and `block_soloed()` must agree, which two independent
    // `soloed()` reads can't guarantee across a mid-block toggle.
    let any_solo = snapshot_top_level_solo(tracks_guard.values().map(|t| &**t));
    let frames = frames.min(track_buf_l.len()).min(track_buf_r.len());
    let mut mixed_any = false;
    for track in tracks_guard.values() {
        if track.sub_track_of.is_some() || !track.runs_internal_instrument() {
            continue;
        }
        let audible = !track.muted() && (!any_solo || track.block_soloed());
        if monitored && audible && track.monitor_enabled() {
            continue;
        }
        let Some(&inst_id) = track.plugins().first() else {
            continue;
        };
        let Some(slot) = plugins_guard.get(&inst_id) else {
            continue;
        };
        {
            let Some(mut inst) = slot.try_lock() else {
                continue;
            };
            // Events parked under contention go first, as on the
            // arrangement path.
            midi_stash.deliver(inst_id, &mut *inst);
            if !inst.0.wants_idle_process() {
                continue;
            }
        }
        let (buf_l, buf_r) = (&mut track_buf_l[..frames], &mut track_buf_r[..frames]);
        buf_l.fill(0.0);
        buf_r.fill(0.0);
        run_track_chain(
            track,
            buf_l,
            buf_r,
            plugins_guard,
            fx_dry,
            transport_snap,
            sample_rate,
        );
        let (target_l, target_r) = if audible {
            track_stereo_gains(track)
        } else {
            (0.0, 0.0)
        };
        let (last_l, last_r) = track.last_gains();
        let gain_l = (last_l, target_l);
        let gain_r = (last_r, target_r);
        let (peak_l, peak_r) = ramped_stereo_peaks(buf_l, buf_r, frames, gain_l, gain_r);
        track.update_peak_l(peak_l);
        track.update_peak_r(peak_r);
        sum_to_output(data, channels, frames, buf_l, buf_r, gain_l, gain_r);
        track.set_last_gains(target_l, target_r);
        mixed_any |= audible;
    }
    mixed_any
}
