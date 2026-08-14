//! Shared per-track render core used by both the live audio callback
//! (`track_block::render_timeline_block`) and the offline bounce
//! renderer (`engine::bounce::render::render_chunk`).
//!
//! The block structure — per-track clip/MIDI/plugin processing, the
//! multi-output instrument fan-out, sub-track routing, latency
//! compensation, and the per-bus pass — is identical on both paths.
//! What differs is captured by [`RenderStrategy`]:
//!
//! - **Live**: non-blocking `try_lock` on plugins (dropping out for one
//!   block on contention, with MIDI parked in the [`MidiStash`]),
//!   transport latching, monitor-input mixing, per-sample gain ramps
//!   from the last-gain atomics (with mute fade-out blocks and the
//!   silenced-instrument path that keeps NoteOffs flowing), and VU peak
//!   metering.
//! - **Bounce**: deterministic blocking locks (spin + back-off so the
//!   audio thread isn't starved), `in_filter` / `respect_mute_solo`
//!   gating, constant gains (a ramp with equal endpoints degenerates to
//!   the constant — bit-identical), and no meter or last-gain atomic
//!   writes, since a bounce may run concurrently with live playback.

use indexmap::IndexMap;
use parking_lot::{Mutex, MutexGuard};

use resonance_common::AutomationTarget;

use crate::clap_host::{StereoBufMut, SyncClapInstance};
use crate::engine::AutomationSnapshot;
use crate::latency::LatencyComp;
use crate::limits::MAX_PLUGIN_OUTPUT_PORTS;
use crate::types::*;

use super::automation_apply::{apply_plugin_params, auto_gain_ramp, auto_muted, auto_volume_ramp};
use super::common::{
    bus_stereo_gains, latch_transport, ramped_stereo_peaks, sum_to_output, sum_to_stereo,
    track_stereo_gains, TransportSnap,
};

/// Automated stereo-gain ramp endpoints `((l_start, l_end), (r_start,
/// r_end))` for one block, or `None` when no gain/pan lane targets the
/// track/bus.
type AutoGain = Option<((f32, f32), (f32, f32))>;

/// Bounce gain ramp endpoints: the automated `start..end` ramp when a
/// lane is present, else the static gains collapsed to a constant
/// (`from == to`, which the ramp helpers reduce to a plain multiply).
#[inline]
fn bounce_gain_endpoints(static_gains: (f32, f32), auto: AutoGain) -> ((f32, f32), (f32, f32)) {
    match auto {
        Some(endpoints) => endpoints,
        None => {
            let (l, r) = static_gains;
            ((l, l), (r, r))
        }
    }
}
use super::midi_events::collect_midi_events;
use super::midi_stash::MidiStash;

/// Per-call policy for the bits of the render block that differ between
/// the live callback and the offline bounce. See the module docs.
pub(crate) enum RenderStrategy<'a> {
    Live {
        midi_stash: &'a mut MidiStash,
        transport_snap: Option<TransportSnap>,
        monitor_temp: &'a [f32],
        monitor_frames: usize,
        input_channels: usize,
    },
    Bounce {
        in_filter: &'a dyn Fn(TrackId) -> bool,
        /// Tracks that are in the filter ONLY to drive their sub-tracks'
        /// port fan-out (ba todo #1242). Their instrument runs — a
        /// sub-track has no other source of audio — but their own main
        /// output (port 0) is discarded before the track's FX chain,
        /// fader, aux sends and routing, so a sub-track stem carries that
        /// tap and nothing else. Always `false` outside stem rendering.
        fan_out_only: &'a dyn Fn(TrackId) -> bool,
        /// Tracks that are in the filter ONLY to be captured as a
        /// sidechain key (ba doc #277). They render through their whole
        /// chain — otherwise there is no audio to capture — and are then
        /// dropped before PDC, fader, aux sends and routing, so they key
        /// the stem without joining it. Always `false` outside stem
        /// rendering, where every track renders anyway.
        key_only: &'a dyn Fn(TrackId) -> bool,
        /// The bus twin of `key_only`.
        key_only_bus: &'a dyn Fn(BusId) -> bool,
        respect_mute_solo: bool,
        /// Freeze-cache capture mode. When `true`, every in-filter track
        /// renders its **raw post-instrument / post-FX** signal — unity
        /// gain, no pan, forced straight to master (its own fader / pan /
        /// bus routing skipped). This keeps the cached WAV fader- and
        /// route-independent so the frozen playback substitution can
        /// re-apply volume / pan / routing / sends live on playback and
        /// stay sample-identical to the unfrozen track (doc #187).
        freeze_raw: bool,
    },
}

/// How a top-level track participates in this block, as decided by the
/// strategy's gating rules.
struct TrackDisposition {
    /// `(previous, target)` gain ramp endpoints per channel. Bounce uses
    /// equal endpoints, which the ramp helpers reduce to a constant.
    gain_l: (f32, f32),
    gain_r: (f32, f32),
    /// Live: muted or solo-suppressed. Inherited by sub-tracks so a
    /// silenced parent fades its fan-out in the same block.
    silenced: bool,
    /// Live: the instrument still runs (NoteOffs keep flowing, voices
    /// don't stick on unmute) but its output is discarded once the mute
    /// ramp has fully faded the previous gain to zero.
    discard_after_instrument: bool,
    /// Bounce/stem: the instrument runs and its extra ports still fan out
    /// to sub-tracks, but this track's OWN main output (port 0) is
    /// discarded — it is only here to drive somebody else's fan-out (ba
    /// todo #1242). Unlike `discard_after_instrument` this must NOT skip
    /// the rest of the iteration, or the fan-out never happens.
    discard_own_output: bool,
}

impl RenderStrategy<'_> {
    /// Live-only side effects: VU peak meters and the last-gain atomics
    /// that seed the next block's ramp. Bounce must not touch either —
    /// it can run while live playback owns them.
    #[inline]
    fn is_live(&self) -> bool {
        matches!(self, Self::Live { .. })
    }

    /// True when this track is rendered only so it can be captured as a
    /// key — see `RenderStrategy::Bounce::key_only`.
    #[inline]
    fn is_key_only(&self, id: TrackId) -> bool {
        match self {
            Self::Live { .. } => false,
            Self::Bounce { key_only, .. } => key_only(id),
        }
    }

    /// The bus twin of [`RenderStrategy::is_key_only`].
    #[inline]
    fn is_key_only_bus(&self, id: BusId) -> bool {
        match self {
            Self::Live { .. } => false,
            Self::Bounce { key_only_bus, .. } => key_only_bus(id),
        }
    }

    /// Freeze-cache capture: bypass per-track / sub-track fader, pan and
    /// bus routing, summing the raw post-FX buffer straight to master.
    /// Only the `Bounce { freeze_raw: true }` strategy does this; every
    /// other path honours the track's real output routing.
    #[inline]
    fn force_master_route(&self) -> bool {
        matches!(self, Self::Bounce { freeze_raw: true, .. })
    }

    /// Acquire an effect plugin's lock. Live: non-blocking, skipping the
    /// plugin for this block on contention (and latching the transport
    /// snapshot on success). Bounce: blocking with spin + back-off.
    #[inline]
    fn lock_fx<'p>(
        &self,
        mutex: &'p Mutex<SyncClapInstance>,
    ) -> Option<MutexGuard<'p, SyncClapInstance>> {
        match self {
            Self::Live { transport_snap, .. } => {
                let mut inst = mutex.try_lock()?;
                latch_transport(&mut inst, *transport_snap);
                Some(inst)
            }
            Self::Bounce { .. } => Some(crate::engine::try_lock_with_backoff(mutex)),
        }
    }

    /// Acquire an instrument plugin's lock. Live additionally replays
    /// events parked during earlier lock contention before the caller
    /// queues this block's events.
    #[inline]
    fn lock_instrument<'p>(
        &mut self,
        mutex: &'p Mutex<SyncClapInstance>,
        id: PluginInstanceId,
    ) -> Option<MutexGuard<'p, SyncClapInstance>> {
        match self {
            Self::Live {
                midi_stash,
                transport_snap,
                ..
            } => {
                let mut inst = mutex.try_lock()?;
                latch_transport(&mut inst, *transport_snap);
                midi_stash.deliver(id, &mut *inst);
                Some(inst)
            }
            Self::Bounce { .. } => Some(crate::engine::try_lock_with_backoff(mutex)),
        }
    }

    /// Live: the UI thread holds the plugin lock (param drag / autosave /
    /// reload) — park this block's events so they replay on the next
    /// successful lock instead of dropping them. The one-block audio
    /// dropout is accepted for now (future work: crossfade). Bounce
    /// locks never fail, so this is unreachable there.
    #[inline]
    fn instrument_lock_failed(&mut self, id: PluginInstanceId, events: &[PendingNoteEvent]) {
        if let Self::Live { midi_stash, .. } = self {
            midi_stash.stash(id, events);
        }
    }

    /// Decide whether and how a top-level track renders this block.
    /// `auto_gain` / `auto_mute` carry this block's automation overrides
    /// (already resolved against the lane snapshot); both are `None` when
    /// nothing automates the track, in which case the static fader / pan
    /// / mute apply exactly as before.
    fn track_disposition(
        &self,
        track: &Track,
        any_solo: bool,
        auto_gain: AutoGain,
        auto_mute: Option<bool>,
    ) -> Option<TrackDisposition> {
        // Automation mute is OR-ed into the static mute so a muted track
        // stays muted regardless, and an unmute lane can't override a
        // hard mute. Solo handling is unchanged.
        let muted = track.muted() || auto_mute.unwrap_or(false);
        match self {
            Self::Live { .. } => {
                // Muted / solo-suppressed instrument tracks still run
                // their instrument plugin (audio discarded) so NoteOffs
                // keep flowing; other tracks are skipped outright —
                // except for one extra block after silencing, which
                // renders normally with a target gain of 0.0 so the
                // mute ramps out instead of hard-cutting.
                let silenced = muted || (any_solo && !track.soloed());
                let (last_gain_l, last_gain_r) = track.last_gains();
                let faded_out = last_gain_l == 0.0 && last_gain_r == 0.0;
                if silenced && faded_out && track.track_type != TrackType::Instrument {
                    return None;
                }
                // Live ramps from the previous block's gain to this
                // block's target; the automated end value becomes the
                // target, so consecutive blocks chain into one smooth
                // sweep (last block's end == this block's start).
                let (target_gain_l, target_gain_r) = if silenced {
                    (0.0, 0.0)
                } else if let Some(((_, gl_end), (_, gr_end))) = auto_gain {
                    (gl_end, gr_end)
                } else {
                    track_stereo_gains(track)
                };
                Some(TrackDisposition {
                    gain_l: (last_gain_l, target_gain_l),
                    gain_r: (last_gain_r, target_gain_r),
                    silenced,
                    discard_after_instrument: silenced && faded_out,
                    discard_own_output: false,
                })
            }
            Self::Bounce {
                in_filter,
                fan_out_only,
                respect_mute_solo,
                freeze_raw,
                ..
            } => {
                // For `to_wav` we honour the user's mix (muted /
                // non-soloed tracks drop out). For bounce-in-place
                // `in_filter` already gates to the source + sub-tracks
                // — and the source is explicitly muted by
                // `finalize_bounce` after every successful bounce, so
                // respecting `muted` would silence every re-bounce.
                if *respect_mute_solo && (muted || (any_solo && !track.soloed())) {
                    return None;
                }
                if !in_filter(track.id) {
                    return None;
                }
                // Freeze captures the raw post-FX signal at unity / no
                // pan; the live mixer re-applies volume / pan (and its
                // automation) on playback, so nothing is baked into the
                // cache. A regular bounce has no previous-block gain to
                // ramp from, so the automated start..end endpoints drive
                // the per-chunk ramp directly (a static track collapses
                // to a constant).
                let (gain_l, gain_r) = if *freeze_raw {
                    ((1.0, 1.0), (1.0, 1.0))
                } else {
                    bounce_gain_endpoints(track_stereo_gains(track), auto_gain)
                };
                Some(TrackDisposition {
                    gain_l,
                    gain_r,
                    silenced: false,
                    discard_after_instrument: false,
                    discard_own_output: fan_out_only(track.id),
                })
            }
        }
    }

    /// Decide whether and how a sub-track renders its parent's port.
    /// Returns the `(gain_l, gain_r)` ramp endpoints.
    ///
    /// `parent_volume` is the parent track's fader (start, end) for this
    /// block, applied as a group trim over the tap — a multi-output
    /// instrument leaves through its taps, so without this the parent's
    /// fader moved a value nobody read and the kit stayed put (ba doc
    /// #275 P1.1). It is volume only, never pan; see `auto_volume_ramp`.
    fn sub_track_disposition(
        &self,
        sub_track: &Track,
        any_solo: bool,
        parent_silenced: bool,
        auto_gain: AutoGain,
        auto_mute: Option<bool>,
        parent_volume: (f32, f32),
    ) -> Option<((f32, f32), (f32, f32))> {
        let muted = sub_track.muted() || auto_mute.unwrap_or(false);
        match self {
            Self::Live { .. } => {
                // A silenced parent fades its sub-tracks out in the same
                // block; once fully faded the fan-out stops running and
                // the subs stay at zero.
                let sub_silenced = muted || parent_silenced;
                let (sub_last_l, sub_last_r) = sub_track.last_gains();
                if sub_silenced && sub_last_l == 0.0 && sub_last_r == 0.0 {
                    return None;
                }
                let (sub_target_l, sub_target_r) = if sub_silenced {
                    (0.0, 0.0)
                } else if let Some(((_, gl_end), (_, gr_end))) = auto_gain {
                    (gl_end, gr_end)
                } else {
                    track_stereo_gains(sub_track)
                };
                // Only the TARGET is trimmed: the ramp's start endpoint is
                // the gain this tap actually ended the previous block on,
                // which already carried whatever the parent fader was then.
                // Trimming it again would square the parent's gain.
                let (sub_target_l, sub_target_r) =
                    (sub_target_l * parent_volume.1, sub_target_r * parent_volume.1);
                Some(((sub_last_l, sub_target_l), (sub_last_r, sub_target_r)))
            }
            Self::Bounce {
                in_filter,
                respect_mute_solo,
                freeze_raw,
                ..
            } => {
                if *respect_mute_solo && (muted || (any_solo && !sub_track.soloed())) {
                    return None;
                }
                if !in_filter(sub_track.id) {
                    return None;
                }
                // Under `freeze_raw` the sub-track keeps its own constant
                // fader / pan (its internal balance is baked into the
                // parent's cache; automation stays live on playback) and
                // `force_master_route` sums it into master so the whole
                // fan-out lands in one cache file. A regular bounce ramps
                // between the automated endpoints.
                if *freeze_raw {
                    // No parent trim here: freeze captures the parent at
                    // unity (see `track_disposition`) and the live mixer
                    // re-applies its fader when the cache plays back.
                    // Baking it in would apply it twice.
                    let (gain_l, gain_r) = track_stereo_gains(sub_track);
                    Some(((gain_l, gain_l), (gain_r, gain_r)))
                } else {
                    let ((l0, l1), (r0, r1)) =
                        bounce_gain_endpoints(track_stereo_gains(sub_track), auto_gain);
                    let (pv0, pv1) = parent_volume;
                    Some(((l0 * pv0, l1 * pv1), (r0 * pv0, r1 * pv1)))
                }
            }
        }
    }

    /// Decide whether and how a bus renders. Live fades a muted bus out
    /// (its FX keep running until the ramp lands on zero); bounce skips
    /// muted busses outright.
    fn bus_disposition(
        &self,
        bus: &Bus,
        auto_gain: AutoGain,
        auto_mute: Option<bool>,
    ) -> Option<((f32, f32), (f32, f32))> {
        let muted = bus.muted() || auto_mute.unwrap_or(false);
        match self {
            Self::Live { .. } => {
                let (bus_last_l, bus_last_r) = bus.last_gains();
                if muted && bus_last_l == 0.0 && bus_last_r == 0.0 {
                    return None;
                }
                let (bus_target_l, bus_target_r) = if muted {
                    (0.0, 0.0)
                } else if let Some(((_, gl_end), (_, gr_end))) = auto_gain {
                    (gl_end, gr_end)
                } else {
                    bus_stereo_gains(bus)
                };
                Some(((bus_last_l, bus_target_l), (bus_last_r, bus_target_r)))
            }
            Self::Bounce { .. } => {
                if muted {
                    return None;
                }
                Some(bounce_gain_endpoints(bus_stereo_gains(bus), auto_gain))
            }
        }
    }

    /// Live-only: mix the track's live input channel(s) from the
    /// interleaved multi-channel monitor buffer. Returns whether any
    /// monitor audio was added.
    fn mix_monitor(
        &self,
        track: &Track,
        track_buf_l: &mut [f32],
        track_buf_r: &mut [f32],
        frames: usize,
    ) -> bool {
        let Self::Live {
            monitor_temp,
            monitor_frames,
            input_channels,
            ..
        } = self
        else {
            return false;
        };
        if !track.monitor_enabled() || *monitor_frames == 0 || *input_channels == 0 {
            return false;
        }
        let is_mono = track.mono();
        let mix_frames = frames.min(*monitor_frames);
        let port = (track.input_port() as usize).min(input_channels - 1);
        let right_port = if is_mono {
            port
        } else {
            (port + 1).min(input_channels - 1)
        };
        for f in 0..mix_frames {
            let base = f * input_channels;
            track_buf_l[f] += monitor_temp[base + port];
            track_buf_r[f] += monitor_temp[base + right_port];
        }
        true
    }
}

/// Multi-output instrument fan-out: zero the first `port_count` port
/// scratch pairs, build a contiguous `StereoBufMut` slice over them,
/// and run `process_multi`.
fn process_multi_port(
    inst: &mut SyncClapInstance,
    port_scratch: &mut [(Vec<f32>, Vec<f32>)],
    port_count: usize,
    frames: usize,
) {
    let mut views: [Option<StereoBufMut<'_>>; MAX_PLUGIN_OUTPUT_PORTS] = Default::default();
    for (i, (pl, pr)) in port_scratch.iter_mut().take(port_count).enumerate() {
        pl[..frames].fill(0.0);
        pr[..frames].fill(0.0);
        views[i] = Some(StereoBufMut {
            left: &mut pl[..frames],
            right: &mut pr[..frames],
        });
    }
    // Build a contiguous slice of StereoBufMut for the CLAP call. We
    // know ports 0..port_count are Some.
    let mut slots: [std::mem::MaybeUninit<StereoBufMut<'_>>; MAX_PLUGIN_OUTPUT_PORTS] =
        [const { std::mem::MaybeUninit::uninit() }; MAX_PLUGIN_OUTPUT_PORTS];
    for i in 0..port_count {
        slots[i].write(views[i].take().unwrap());
    }
    // SAFETY: the first `port_count` slots are initialized above; the
    // slice only refers to those.
    let slice: &mut [StereoBufMut<'_>] = unsafe {
        std::slice::from_raw_parts_mut(slots.as_mut_ptr() as *mut StereoBufMut<'_>, port_count)
    };
    inst.0.process_multi(slice, frames);
    // Drop the initialized entries before the MaybeUninit array goes
    // out of scope.
    for slot in slots.iter_mut().take(port_count) {
        unsafe { slot.assume_init_drop() };
    }
}

/// Recorded-playback monitor gate (doc #257): `true` when the track's
/// live input monitor must be *skipped* for the block `[playhead,
/// playhead + frames)` because the track's playback source is
/// `Recorded` and a recorded take (an audio clip on the track) covers
/// the block — the take plays through the normal clip mix and the
/// hardware return must not be layered on top of it.
///
/// A record-armed track is never gated, so a punch-in still monitors
/// the hardware while re-recording over an existing take. The stopped
/// transport never reaches this path at all (`monitor.rs`'s
/// passthrough handles stopped monitoring), so monitoring while
/// preparing a take is untouched. Block granularity matches the
/// monitor stream itself (~a few ms). Cheap: two relaxed atomic loads,
/// and the `O(clips)` span scan only runs for `Recorded` tracks.
pub fn recorded_monitor_gate(
    track: &Track,
    clips: &[AudioClip],
    playhead: u64,
    frames: usize,
) -> bool {
    track.playback_source() == resonance_common::PlaybackSource::Recorded
        && !track.record_armed()
        && audio_clip_covers(clips, track.id, playhead, playhead + frames as u64)
}

/// Fill the de-interleaved track buffers from a track's [`FrozenSource`]
/// cache for the timeline window `[playhead, playhead + frames)`,
/// replacing the live instrument + insert-FX render (doc #187, todo
/// #573). The cache is interleaved stereo L/R, rendered from sample 0 so
/// timeline frame `t` maps directly to cache frame `t`; frames past the
/// end of the cache stay silent (the caller zeroed the buffers).
///
/// Sample-rate mismatch between the cache and the engine is handled by
/// linear interpolation: with matching rates (the normal case — the
/// cache is rendered at the project rate) the read is frame-for-frame and
/// therefore bit-exact, which is what makes a frozen bounce sample-
/// identical to the unfrozen one. Returns whether any non-zero sample was
/// written. Allocation-free and `O(frames)`.
fn fill_from_frozen_source(
    source: &FrozenSource,
    engine_sample_rate: u32,
    playhead: u64,
    frames: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
) -> bool {
    let samples = source.samples.as_slice();
    let cache_frames = source.frame_count;
    let mut has_audio = false;

    if source.sample_rate == engine_sample_rate || engine_sample_rate == 0 {
        // Frame-for-frame copy — bit-exact, the parity-critical path.
        for f in 0..frames {
            let tl = playhead + f as u64;
            if tl >= cache_frames {
                break;
            }
            let idx = tl as usize * 2;
            if idx + 1 >= samples.len() {
                break;
            }
            let (l, r) = (samples[idx], samples[idx + 1]);
            track_buf_l[f] = l;
            track_buf_r[f] = r;
            has_audio |= l != 0.0 || r != 0.0;
        }
    } else {
        // Rate mismatch: resample the cache on the fly by linear
        // interpolation. Timeline frame `tl` maps to cache position
        // `tl * cache_rate / engine_rate`.
        let ratio = source.sample_rate as f64 / engine_sample_rate as f64;
        for f in 0..frames {
            let tl = playhead + f as u64;
            let src_pos = tl as f64 * ratio;
            let i0 = src_pos.floor() as u64;
            if i0 >= cache_frames {
                break;
            }
            let frac = (src_pos - i0 as f64) as f32;
            let idx0 = i0 as usize * 2;
            if idx0 + 1 >= samples.len() {
                break;
            }
            let (l0, r0) = (samples[idx0], samples[idx0 + 1]);
            let (l1, r1) = if i0 + 1 < cache_frames && idx0 + 3 < samples.len() {
                (samples[idx0 + 2], samples[idx0 + 3])
            } else {
                (l0, r0)
            };
            let l = l0 + (l1 - l0) * frac;
            let r = r0 + (r1 - r0) * frac;
            track_buf_l[f] = l;
            track_buf_r[f] = r;
            has_audio |= l != 0.0 || r != 0.0;
        }
    }

    has_audio
}

/// Mix every audio clip on `track_id` into the de-interleaved track
/// buffers for the timeline window `[playhead, playhead + frames)`,
/// applying per-frame the single coefficient
/// `fade_in_envelope × fade_out_envelope × dB→linear(gain_db)`. Returns
/// whether any clip contributed audio.
///
/// Where two clips on the same track overlap, the overlap region is an
/// automatic crossfade: the earlier clip fades out and the later clip
/// fades in across the shared span. With the default equal-power curves
/// the two contributions sum to constant power, so the seam is
/// click-free. An explicit fade that is longer than the overlap reshapes
/// the crossfade (the longer of the two lengths wins).
///
/// Edges that no fade and no overlap cover still get the short
/// [`CLIP_DECLICK_FRAMES`] ramp, so a trimmed, split or butt-joined clip
/// cannot step the signal on its first and last frame.
///
/// Shared verbatim by the live mixer and the offline bounce/export (both
/// reach it through [`render_block`]), so playback and bounced WAV render
/// identically. Allocation-free and `O(1)` per output frame (the
/// per-clip crossfade scan is `O(clips)`, run once per clip per block).
pub fn mix_track_clips(
    clips: &[AudioClip],
    track_id: TrackId,
    playhead: u64,
    frames: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
) -> bool {
    let buf_start = playhead;
    let buf_end = playhead + frames as u64;
    let mut has_audio = false;

    for clip in clips.iter() {
        if clip.track_id != track_id {
            continue;
        }

        let clip_frames = clip.duration_frames();
        let clip_start = clip.start_sample;
        let clip_end = clip_start + clip_frames;

        if buf_end <= clip_start || buf_start >= clip_end {
            continue;
        }

        let overlap_start = buf_start.max(clip_start);
        let overlap_end = buf_end.min(clip_end);

        // Fold the automatic same-track crossfade into the fade lengths:
        // an overlap at the clip's head/tail behaves like a fade of that
        // length, and the explicit fade wins only when it is longer.
        let (head_xfade, tail_xfade) = clip_crossfade_lengths(clip, clips, clip_frames);
        // Anti-click ramp on both audible edges — see `CLIP_DECLICK_FRAMES`.
        // Whichever of the three is longest shapes the edge, so an explicit
        // fade or a crossfade always subsumes the declick.
        let declick = declick_frames(clip_frames);
        let fade_in_len = clip.fade_in_frames.max(head_xfade).max(declick);
        let fade_out_len = clip.fade_out_frames.max(tail_xfade).max(declick);
        let gain_lin = if clip.gain_db == 0.0 {
            1.0
        } else {
            10f32.powf(clip.gain_db / 20.0)
        };

        let clip_data = clip.source.as_frames();
        for timeline_frame in overlap_start..overlap_end {
            let frame_offset = (timeline_frame - buf_start) as usize;
            let clip_frame =
                (timeline_frame - clip_start) as usize + clip.trim_start_frames as usize;
            let clip_idx = clip_frame * 2;
            if clip_idx + 1 < clip_data.len() {
                let coef = clip_fade_gain_coef(
                    timeline_frame,
                    clip_start,
                    clip_end,
                    fade_in_len,
                    fade_out_len,
                    clip.fade_in_curve,
                    clip.fade_out_curve,
                    gain_lin,
                );
                track_buf_l[frame_offset] += clip_data[clip_idx] * coef;
                track_buf_r[frame_offset] += clip_data[clip_idx + 1] * coef;
                has_audio = true;
            }
        }
    }

    has_audio
}

/// Length of the automatic anti-click ("declick") ramp applied to both
/// audible edges of every audio clip: 2 ms at 48 kHz.
///
/// A clip edge is a splice. Trimming a take, splitting it, or butting two
/// takes together almost never lands on a zero crossing, so playing the
/// raw samples steps the signal from silence to whatever the waveform
/// happened to be doing — an audible click, and one that a downstream amp
/// sim or delay then amplifies and repeats. Every clip therefore ramps in
/// and out over this many frames unless a longer explicit fade or an
/// overlap crossfade already shapes that edge.
///
/// 2 ms is long enough to remove the step for the lowest musical
/// fundamentals and short enough to leave a transient sliced at its attack
/// sounding like a transient (Ardour declicks over ~64 frames, Reaper over
/// 10 ms; this sits deliberately between them). Expressed in frames rather
/// than seconds because the clip mix is rate-agnostic — at 44.1 kHz it is
/// 2.2 ms, which is the same thing musically.
pub const CLIP_DECLICK_FRAMES: u64 = 96;

/// The declick ramp length for a clip of `clip_frames` audible frames,
/// capped at half the clip so the head and tail ramps of a very short clip
/// (a sliced grain, a drum hit) meet at its midpoint instead of overlapping
/// into a double attenuation.
#[inline]
fn declick_frames(clip_frames: u64) -> u64 {
    CLIP_DECLICK_FRAMES.min(clip_frames / 2)
}

/// Linear gain coefficient applied to `clip` at absolute timeline frame
/// `timeline_frame`, combining the fade-in ramp, the fade-out ramp, and
/// the clip's (already linearised) gain. `clip_end` is exclusive.
#[inline]
#[allow(clippy::too_many_arguments)]
fn clip_fade_gain_coef(
    timeline_frame: u64,
    clip_start: u64,
    clip_end: u64,
    fade_in_len: u64,
    fade_out_len: u64,
    fade_in_curve: FadeCurve,
    fade_out_curve: FadeCurve,
    gain_lin: f32,
) -> f32 {
    let mut coef = gain_lin;
    if fade_in_len > 0 {
        let pos = timeline_frame - clip_start;
        if pos < fade_in_len {
            coef *= fade_in_curve.coefficient(pos as f32 / fade_in_len as f32);
        }
    }
    if fade_out_len > 0 {
        // Frames remaining before the clip's last visible frame; the
        // curve runs the complementary direction (`coefficient(0)` at the
        // final frame), which equal-power turns into the constant-power
        // crossfade complement.
        let pos_from_end = (clip_end - 1).saturating_sub(timeline_frame);
        if pos_from_end < fade_out_len {
            coef *= fade_out_curve.coefficient(pos_from_end as f32 / fade_out_len as f32);
        }
    }
    coef
}

/// Lengths (in frames) of the automatic crossfades at `clip`'s head and
/// tail, derived from where other clips on the same track overlap it. The
/// head length is the span an earlier-starting clip covers from `clip`'s
/// start; the tail length is the span a later-starting clip covers up to
/// `clip`'s end. Each is capped at the clip's visible duration so a clip
/// overlapped on both sides cannot fade past its own length.
fn clip_crossfade_lengths(clip: &AudioClip, clips: &[AudioClip], clip_frames: u64) -> (u64, u64) {
    let clip_start = clip.start_sample;
    let clip_end = clip_start + clip_frames;
    let mut head = 0u64;
    let mut tail = 0u64;
    for other in clips.iter() {
        if other.id == clip.id || other.track_id != clip.track_id {
            continue;
        }
        let o_start = other.start_sample;
        let o_end = o_start + other.duration_frames();
        // An earlier-or-equal-starting clip covering this clip's start →
        // crossfade in over the covered span.
        if o_start <= clip_start && o_end > clip_start {
            head = head.max(o_end.min(clip_end) - clip_start);
        }
        // A later-starting clip overlapping this clip's tail → crossfade
        // out over the span from where it starts to this clip's end.
        if o_start > clip_start && o_start < clip_end {
            tail = tail.max(clip_end - o_start);
        }
    }
    (head.min(clip_frames), tail.min(clip_frames))
}

/// Linear gain for an aux-send level in dB. `0 dB` short-circuits to
/// unity (the common case for a freshly-created send) so the per-block
/// tap stays cheap.
#[inline]
fn db_to_linear(db: f32) -> f32 {
    if db == 0.0 {
        1.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Sum bus `src_idx`'s summing buffer into bus `dst_idx`'s, scaled by the
/// (possibly ramped) `gain_l`/`gain_r`. The two indices are required to
/// differ — a bus can never aux-send to itself (cyclic-route validation
/// rejects it) — so a disjoint `split_at_mut` lets both buffers be
/// borrowed at once without allocating a temporary.
#[inline]
fn sum_bus_to_bus(
    bus_bufs: &mut [(Vec<f32>, Vec<f32>)],
    src_idx: usize,
    dst_idx: usize,
    frames: usize,
    gain_l: (f32, f32),
    gain_r: (f32, f32),
) {
    if src_idx == dst_idx {
        return;
    }
    let (src, dst) = if src_idx < dst_idx {
        let (left, right) = bus_bufs.split_at_mut(dst_idx);
        (&left[src_idx], &mut right[0])
    } else {
        let (left, right) = bus_bufs.split_at_mut(src_idx);
        (&right[0], &mut left[dst_idx])
    };
    sum_to_stereo(&mut dst.0, &mut dst.1, frames, &src.0, &src.1, gain_l, gain_r);
}

/// Render one contiguous timeline block into the interleaved output:
/// walks every active track + bus, mixes audio clips, dispatches MIDI
/// events to instrument plugins, routes per-port multi-output
/// instruments through their sub-tracks, and sums into the output (or
/// per-bus summing buffer). Allocation-free.
///
/// `aux_sends` is the engine's current aux-send table (a lock-free
/// snapshot loaded once per block by the caller). For every enabled send
/// the source track/bus's signal is tapped — pre-fader (raw, send level
/// only) or post-fader (after the source's fader/pan ramp) — scaled by
/// the send level and summed into the destination return bus's summing
/// buffer, in addition to the source's normal output. Empty ⇒ the block
/// renders byte-for-byte as before, so projects without sends are
/// unaffected.
///
/// The caller is responsible for:
/// - Passing `data` sliced to exactly `frames * channels` samples and
///   cleared before the first call.
/// - Running any master FX / metronome / master-volume passes afterwards.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_block(
    data: &mut [f32],
    channels: usize,
    tracks_guard: &IndexMap<TrackId, Track>,
    busses_guard: &IndexMap<BusId, Bus>,
    clips_guard: &[AudioClip],
    midi_clips_guard: &[MidiClip],
    plugins_guard: &IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>,
    tempo_map: &TempoMap,
    sample_rate: u32,
    any_solo: bool,
    active_busses: usize,
    aux_sends: &[AuxSend],
    sidechain_routes: &[SidechainRoute],
    sidechain: &mut SidechainTaps,
    playhead: u64,
    frames: usize,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    bus_bufs: &mut [(Vec<f32>, Vec<f32>)],
    port_scratch: &mut [(Vec<f32>, Vec<f32>)],
    note_event_buf: &mut Vec<PendingNoteEvent>,
    latency_comp: &LatencyComp,
    automation: &AutomationSnapshot,
    strategy: &mut RenderStrategy<'_>,
) {
    // Automation is evaluated at the block's first frame and at the
    // next-block-start frame (`eval_end`); the gain ramp sweeps between
    // them. Because `eval_end` of one block equals `eval_start` of the
    // next, consecutive blocks chain into one continuous sweep. Plugin
    // params and the mute gate are evaluated at the block start.
    let eval_start = playhead;
    let eval_end = playhead + frames as u64;

    // Post-PDC parameters (fader / pan / mute, applied at sum time
    // after the delay lines) act on audio whose timeline position is
    // one comp stage older than the raw playhead: track/sub gains meet
    // their audio `track_stage` late, bus gains a further `bus_stage`
    // late. Evaluating their automation at the comp-delayed position
    // makes a drawn move land on the audio it was drawn against
    // (doc #260 finding #9). Pre-chain parameters (plugin params, MIDI)
    // keep the raw positions. Zero-latency projects shift by 0 and stay
    // bit-identical.
    let track_gain_shift = latency_comp.track_stage();
    let gain_eval_start = eval_start.saturating_sub(track_gain_shift);
    let gain_eval_end = eval_end.saturating_sub(track_gain_shift);
    let bus_gain_shift = latency_comp.max_latency();
    let bus_eval_start = eval_start.saturating_sub(bus_gain_shift);
    let bus_eval_end = eval_end.saturating_sub(bus_gain_shift);

    // Zero every active bus summing buffer at the start of the block so
    // tracks can accumulate into them.
    for (buf_l, buf_r) in bus_bufs.iter_mut().take(active_busses) {
        buf_l[..frames].fill(0.0);
        buf_r[..frames].fill(0.0);
    }

    // Per-track processing: (clips + monitor input) -> plugins -> volume
    // -> master. Sub-tracks are skipped here; they're driven by their
    // parent's plugin fan-out later in the same track pass.
    for track in tracks_guard.values() {
        if track.sub_track_of.is_some() {
            continue;
        }
        let auto_gain = auto_gain_ramp(
            automation,
            AutomationTarget::TrackGain(track.id),
            AutomationTarget::TrackPan(track.id),
            track.volume(),
            track.pan(),
            gain_eval_start,
            gain_eval_end,
        );
        let auto_mute = auto_muted(
            automation,
            AutomationTarget::TrackMute(track.id),
            gain_eval_start,
        );
        let Some(TrackDisposition {
            gain_l,
            gain_r,
            silenced,
            discard_after_instrument,
            discard_own_output,
        }) = strategy.track_disposition(track, any_solo, auto_gain, auto_mute)
        else {
            continue;
        };

        // Zero per-track buffers
        track_buf_l[..frames].fill(0.0);
        track_buf_r[..frames].fill(0.0);

        let mut has_audio = false;
        // Sub-track fan-out book-keeping: how many extra output ports the
        // instrument plugin filled on this block, so the post-plugin loop
        // knows how many `port_scratch` entries to route to sub-tracks.
        let mut extra_ports_filled: usize = 0;

        // Frozen playback substitution (doc #187, todo #573): when the
        // track carries an active frozen source, play its cached post-FX
        // samples in place of running the timeline synth + insert FX. The
        // cache is timeline-aligned and captured pre-fader (see
        // `freeze_raw`), so the post-source mixer stage below — PDC,
        // volume, pan, mute / solo, routing and aux sends — still applies
        // live. This path is shared by the live callback and the offline
        // bounce / stem renderer, so a frozen track is transparently
        // identical in playback and in export, with no separate code path.
        // (The instrument's fan-out is skipped, so `extra_ports_filled`
        // stays 0; a frozen multi-output track's sub-mix is already baked
        // into its single cache file.)
        let frozen_source = track.frozen_source.load_full();
        if let Some(source) = frozen_source.as_deref() {
            if fill_from_frozen_source(
                source,
                sample_rate,
                playhead,
                frames,
                &mut track_buf_l[..frames],
                &mut track_buf_r[..frames],
            ) {
                has_audio = true;
            }
        } else if track.track_type == TrackType::Instrument && !track.is_external() {
            // -- Instrument track: collect MIDI events, send to instrument plugin --
            collect_midi_events(
                midi_clips_guard,
                track.id,
                playhead,
                frames,
                tempo_map,
                sample_rate,
                note_event_buf,
            );

            // Process: first plugin is the instrument (receives note events),
            // remaining plugins are effects (audio-only).
            let track_plugins = track.plugins();
            let mut plugin_iter = track_plugins.iter();
            if let Some(&instrument_id) = plugin_iter.next() {
                if let Some(mutex) = plugins_guard.get(&instrument_id) {
                    if let Some(mut inst) = strategy.lock_instrument(mutex, instrument_id) {
                        apply_plugin_params(&mut inst, automation, instrument_id, eval_start);
                        for event in note_event_buf.iter() {
                            if event.is_note_on {
                                inst.0.queue_note_on(
                                    event.note,
                                    event.velocity,
                                    event.sample_offset,
                                );
                            } else {
                                inst.0.queue_note_off(event.note, event.sample_offset);
                            }
                        }

                        let port_count = inst.0.output_port_count().min(port_scratch.len());
                        if port_count > 1 {
                            // Multi-output instrument: fan out into the
                            // per-port scratch pool, then copy port 0 back
                            // into the track's main buffer so the rest of
                            // the track chain (effects + fader + bus
                            // routing) runs unchanged.
                            process_multi_port(&mut inst, port_scratch, port_count, frames);
                            track_buf_l[..frames].copy_from_slice(&port_scratch[0].0[..frames]);
                            track_buf_r[..frames].copy_from_slice(&port_scratch[0].1[..frames]);
                            extra_ports_filled = port_count;
                        } else {
                            // Single-output path (legacy plugins): use the
                            // thin wrapper that re-targets onto track_buf_l/r.
                            inst.0.process(
                                &mut track_buf_l[..frames],
                                &mut track_buf_r[..frames],
                                frames,
                            );
                        }
                        has_audio = true;
                    } else {
                        strategy.instrument_lock_failed(instrument_id, note_event_buf);
                    }
                }
            }
            // Silenced track (live): the instrument ran (voice state stays
            // consistent) but its output is discarded — once the mute
            // ramp has finished fading the previous gain to zero.
            if discard_after_instrument {
                continue;
            }
            // Fan-out driver only (ba todo #1242): this track is in the
            // stem's filter so its instrument would RUN and fill the port
            // scratch, not because its own main output belongs here.
            // Drop port 0 — and with it the parent chain, fader, aux
            // sends and routing that would otherwise carry it into the
            // stem — while falling through to the sub-track fan-out
            // below, which is the whole reason this track is rendering.
            if discard_own_output {
                track_buf_l[..frames].fill(0.0);
                track_buf_r[..frames].fill(0.0);
            } else if !track.fx_bypassed() {
                for &plugin_id in plugin_iter {
                    if let Some(mutex) = plugins_guard.get(&plugin_id) {
                        if let Some(mut inst) = strategy.lock_fx(mutex) {
                            apply_plugin_params(&mut inst, automation, plugin_id, eval_start);
                            // Same external key as the audio-track branch
                            // below: a ducker on a synth track is the most
                            // common sidechain there is, and routing one
                            // here used to store the route and then key
                            // off the track's own input (ba doc #275 P0).
                            let key = sidechain.key_for(sidechain_routes, plugin_id);
                            let mut outs = [StereoBufMut {
                                left: &mut track_buf_l[..frames],
                                right: &mut track_buf_r[..frames],
                            }];
                            inst.0.process_multi_with_key(&mut outs, key, frames);
                            has_audio = true;
                        }
                    }
                }
            }
        } else {
            // -- Audio track: mix clips + monitor input + plugin chain --
            //
            // External-instrument tracks land here too (doc #169): their
            // synth is outboard, so there is no instrument plugin to run
            // and their audio arrives on the return input — live through
            // the monitor mix, or as a recorded take through the clip
            // mix. Every plugin on such a track is an insert effect.

            // Mix monitor input for all tracks with monitoring enabled
            // (live path only) — unless the Recorded playback source
            // gates it because a recorded take covers this block
            // (doc #257); the take itself arrives via the clip mix
            // just below.
            if !recorded_monitor_gate(track, clips_guard, playhead, frames)
                && strategy.mix_monitor(track, track_buf_l, track_buf_r, frames)
            {
                has_audio = true;
            }

            // Accumulate all clips for this track into de-interleaved
            // track buffers, applying each clip's fade-in/out envelope,
            // clip gain, and the automatic same-track crossfade.
            if mix_track_clips(clips_guard, track.id, playhead, frames, track_buf_l, track_buf_r) {
                has_audio = true;
            }

            // Process through plugin chain (skipped when FX are bypassed).
            let track_plugins = track.plugins();
            if !track_plugins.is_empty() && !track.fx_bypassed() {
                for &plugin_id in track_plugins.iter() {
                    if let Some(mutex) = plugins_guard.get(&plugin_id) {
                        if let Some(mut inst) = strategy.lock_fx(mutex) {
                            apply_plugin_params(&mut inst, automation, plugin_id, eval_start);
                            // An external key, when this instance is
                            // routed one and actually declares a key
                            // port. `sidechain` is borrowed immutably
                            // here and mutably at the capture below, so
                            // the two never overlap.
                            let key = sidechain.key_for(sidechain_routes, plugin_id);
                            let mut outs = [StereoBufMut {
                                left: &mut track_buf_l[..frames],
                                right: &mut track_buf_r[..frames],
                            }];
                            inst.0.process_multi_with_key(&mut outs, key, frames);
                            has_audio = true;
                        }
                    }
                }
            }
        }

        // Capture this track post-FX and pre-fader for anything keying
        // off it. Costs a `copy_from_slice` only for tracks that are
        // actually routed somewhere as a key.
        let tap_source = SendSource::Track(track.id);
        if sidechain.is_tapped(tap_source) {
            sidechain.capture(
                tap_source,
                &track_buf_l[..frames],
                &track_buf_r[..frames],
                frames,
            );
        }

        // Present only as a key source: its audio has just been captured,
        // and it belongs to a different stem (ba doc #277). Everything
        // below — PDC, fader, aux sends, routing — would put it in this
        // one, so stop here.
        if strategy.is_key_only(track.id) {
            continue;
        }

        // Plugin-delay compensation: delay the post-chain signal so
        // every track reaches master with the same total latency (see
        // `crate::latency`). Runs even when the track produced no audio
        // this block so delayed tails keep flushing.
        if latency_comp.apply(
            track.id,
            &mut track_buf_l[..frames],
            &mut track_buf_r[..frames],
            playhead,
        ) {
            has_audio = true;
        }

        if !has_audio {
            // Nothing to ramp over: snap the remembered gain to the
            // target so changes made during silence don't ramp later.
            if strategy.is_live() {
                track.set_last_gains(gain_l.1, gain_r.1);
            }
            continue;
        }

        // Compute post-fader peak levels for VU meters (live only).
        if strategy.is_live() {
            let (peak_l, peak_r) =
                ramped_stereo_peaks(track_buf_l, track_buf_r, frames, gain_l, gain_r);
            track.update_peak_l(peak_l);
            track.update_peak_r(peak_r);
        }

        // Route post-fader audio: either directly to the interleaved
        // output or into the target bus's summing buffer. If the target
        // bus no longer exists (e.g. removed mid-block), fall back to
        // master so the track isn't silenced.
        let routed_to_bus = if strategy.force_master_route() {
            // Freeze capture: sum the raw post-FX buffer straight to
            // master, never through the track's bus (bus FX would
            // otherwise bake into the cache and double on playback).
            false
        } else {
            match track.output() {
                TrackOutput::Bus(bus_id) => busses_guard
                    .get_index_of(&bus_id)
                    .filter(|idx| *idx < active_busses)
                    .map(|idx| {
                        let (bl, br) = &mut bus_bufs[idx];
                        sum_to_stereo(bl, br, frames, track_buf_l, track_buf_r, gain_l, gain_r);
                    })
                    .is_some(),
                TrackOutput::Master => false,
            }
        };
        if !routed_to_bus {
            sum_to_output(
                data,
                channels,
                frames,
                track_buf_l,
                track_buf_r,
                gain_l,
                gain_r,
            );
        }
        if strategy.is_live() {
            track.set_last_gains(gain_l.1, gain_r.1);
        }

        // Aux sends: tap this track's signal into each destination return
        // bus, on top of the main output routed above. Post-fader follows
        // the fader/pan/mute ramp (`gain_l`/`gain_r`, which ramps to zero
        // on a muted track); pre-fader takes the raw post-plugin signal
        // with the send level only. The destination's summing buffer is
        // always filled before the bus pass runs it, so a track→return
        // send is sample-correct regardless of bus ordering.
        for send in aux_sends {
            if !send.enabled || send.source != SendSource::Track(track.id) {
                continue;
            }
            let Some(dst_idx) = busses_guard
                .get_index_of(&send.dest)
                .filter(|idx| *idx < active_busses)
            else {
                continue;
            };
            let send_lin = db_to_linear(send.level_db);
            let (send_gain_l, send_gain_r) = if send.pre_fader {
                ((send_lin, send_lin), (send_lin, send_lin))
            } else {
                (
                    (gain_l.0 * send_lin, gain_l.1 * send_lin),
                    (gain_r.0 * send_lin, gain_r.1 * send_lin),
                )
            };
            let (dst_l, dst_r) = &mut bus_bufs[dst_idx];
            sum_to_stereo(
                dst_l,
                dst_r,
                frames,
                track_buf_l,
                track_buf_r,
                send_gain_l,
                send_gain_r,
            );
        }

        // Sub-track fan-out: for every non-main plugin output port that
        // was filled by the instrument above, look up the matching
        // sub-track (if any) and route its scratch buffer through the
        // sub-track's fader / pan / bus.
        if extra_ports_filled > 1 {
            // The parent's fader is the kit's group trim (ba doc #275
            // P1.1). Evaluated once per block, outside the tap loop.
            let parent_volume = auto_volume_ramp(
                automation,
                AutomationTarget::TrackGain(track.id),
                track.volume(),
                gain_eval_start,
                gain_eval_end,
            );
            for sub_track in tracks_guard.values() {
                let Some((parent_id, port_idx)) = sub_track.sub_track_of else {
                    continue;
                };
                if parent_id != track.id {
                    continue;
                }
                let port_idx = port_idx as usize;
                if port_idx == 0 || port_idx >= extra_ports_filled {
                    continue;
                }
                let sub_auto_gain = auto_gain_ramp(
                    automation,
                    AutomationTarget::TrackGain(sub_track.id),
                    AutomationTarget::TrackPan(sub_track.id),
                    sub_track.volume(),
                    sub_track.pan(),
                    gain_eval_start,
                    gain_eval_end,
                );
                let sub_auto_mute = auto_muted(
                    automation,
                    AutomationTarget::TrackMute(sub_track.id),
                    gain_eval_start,
                );
                let Some((sub_gain_l, sub_gain_r)) = strategy.sub_track_disposition(
                    sub_track,
                    any_solo,
                    silenced,
                    sub_auto_gain,
                    sub_auto_mute,
                    parent_volume,
                ) else {
                    continue;
                };

                // Run the sub-track's own effect chain in place on its
                // port buffer, before peak metering and bus/master routing.
                // Sub-tracks never host an instrument, so every entry in
                // the plugin chain is treated as an audio effect and is
                // subject to the sub-track's own FX-bypass flag.
                if !sub_track.fx_bypassed() {
                    let (pl, pr) = &mut port_scratch[port_idx];
                    let sub_plugins = sub_track.plugins();
                    for &plugin_id in sub_plugins.iter() {
                        if let Some(mutex) = plugins_guard.get(&plugin_id) {
                            if let Some(mut inst) = strategy.lock_fx(mutex) {
                                apply_plugin_params(&mut inst, automation, plugin_id, eval_start);
                                let key = sidechain.key_for(sidechain_routes, plugin_id);
                                let mut outs = [StereoBufMut {
                                    left: &mut pl[..frames],
                                    right: &mut pr[..frames],
                                }];
                                inst.0.process_multi_with_key(&mut outs, key, frames);
                            }
                        }
                    }
                }

                // A sub-track is a first-class key source: "duck the bass
                // from the kick" on a multi-output kit means keying off
                // the kick TAP, which is the only place that piece exists
                // as its own signal. Captured post-FX, pre-fader, exactly
                // like the top-level tracks above.
                let sub_tap = SendSource::Track(sub_track.id);
                if sidechain.is_tapped(sub_tap) {
                    let (pl, pr) = &port_scratch[port_idx];
                    sidechain.capture(sub_tap, &pl[..frames], &pr[..frames], frames);
                }

                // Captured, and not a member of this stem (ba doc #277).
                // This is the drum-tap case the field report hit: keying
                // a compressor from the kick TAP while measuring the
                // ducked track, which is how the routing gets verified.
                if strategy.is_key_only(sub_track.id) {
                    continue;
                }

                // Plugin-delay compensation for the sub-track's chain.
                {
                    let (pl, pr) = &mut port_scratch[port_idx];
                    latency_comp.apply(
                        sub_track.id,
                        &mut pl[..frames],
                        &mut pr[..frames],
                        playhead,
                    );
                }

                // Peak levels for sub-track VU meter (live only).
                let (pl, pr) = &port_scratch[port_idx];
                if strategy.is_live() {
                    let (sub_peak_l, sub_peak_r) =
                        ramped_stereo_peaks(pl, pr, frames, sub_gain_l, sub_gain_r);
                    sub_track.update_peak_l(sub_peak_l);
                    sub_track.update_peak_r(sub_peak_r);
                }

                // Route post-fader audio to the sub-track's destination.
                // Freeze capture folds the fan-out into master so the
                // parent's cache carries the whole multi-output mix.
                let routed = if strategy.force_master_route() {
                    false
                } else {
                    match sub_track.output() {
                        TrackOutput::Bus(bus_id) => busses_guard
                            .get_index_of(&bus_id)
                            .filter(|idx| *idx < active_busses)
                            .map(|idx| {
                                let (bl, br) = &mut bus_bufs[idx];
                                sum_to_stereo(bl, br, frames, pl, pr, sub_gain_l, sub_gain_r);
                            })
                            .is_some(),
                        TrackOutput::Master => false,
                    }
                };
                if !routed {
                    sum_to_output(data, channels, frames, pl, pr, sub_gain_l, sub_gain_r);
                }
                if strategy.is_live() {
                    sub_track.set_last_gains(sub_gain_l.1, sub_gain_r.1);
                }
            }
        }
    }

    // Bus-stage equalization for master-direct signals: everything the
    // track pass summed straight into the output (`data` holds exactly
    // those contributions here — the bus pass below hasn't run yet) is
    // delayed by the shared dry line so it arrives together with
    // signals that traverse a bus chain (see `crate::latency`). No-op
    // when no bus carries latency.
    latency_comp.apply_dry(data, channels, frames, playhead);

    // Per-bus processing: plugin chain, volume/pan, peaks, sum to master.
    for (bus_idx, bus) in busses_guard.values().enumerate().take(active_busses) {
        let bus_auto_gain = auto_gain_ramp(
            automation,
            AutomationTarget::BusGain(bus.id),
            AutomationTarget::BusPan(bus.id),
            bus.volume(),
            bus.pan(),
            bus_eval_start,
            bus_eval_end,
        );
        let bus_auto_mute =
            auto_muted(automation, AutomationTarget::BusMute(bus.id), bus_eval_start);
        let Some((bus_gain_l, bus_gain_r)) =
            strategy.bus_disposition(bus, bus_auto_gain, bus_auto_mute)
        else {
            continue;
        };
        let (bus_buf_l, bus_buf_r) = &mut bus_bufs[bus_idx];

        // Process bus plugin chain in place over the accumulated buffer
        // (skipped when the bus's FX are bypassed).
        if !bus.fx_bypassed() {
            for &plugin_id in &bus.plugin_ids {
                if let Some(mutex) = plugins_guard.get(&plugin_id) {
                    if let Some(mut inst) = strategy.lock_fx(mutex) {
                        apply_plugin_params(&mut inst, automation, plugin_id, eval_start);
                        let key = sidechain.key_for(sidechain_routes, plugin_id);
                        let mut outs = [StereoBufMut {
                            left: &mut bus_buf_l[..frames],
                            right: &mut bus_buf_r[..frames],
                        }];
                        inst.0.process_multi_with_key(&mut outs, key, frames);
                    }
                }
            }
        }

        // Capture this bus post-FX and pre-fader for anything keying off
        // it — the bus half of the track tap above. `SendSource::Bus` has
        // been a legal route target all along (`sidechain::from_bus`), so
        // without this a bus-sourced route resolved to a slot that was
        // never written and silently keyed off the plugin's own input.
        let bus_tap = SendSource::Bus(bus.id);
        if sidechain.is_tapped(bus_tap) {
            sidechain.capture(bus_tap, &bus_buf_l[..frames], &bus_buf_r[..frames], frames);
        }

        // Captured, and not part of this stem (ba doc #277).
        if strategy.is_key_only_bus(bus.id) {
            continue;
        }

        // Bus-stage equalization: pad this bus's chain up to the
        // longest bus chain, so every path through *any* bus (main
        // output or aux send) reaches master with the same bus-stage
        // latency (see `crate::latency`). Runs before peaks / fader /
        // send taps; every active bus is processed each block, so
        // delayed tails keep flushing.
        latency_comp.apply_bus(bus.id, &mut bus_buf_l[..frames], &mut bus_buf_r[..frames], playhead);

        // Compute post-fader peaks (live only).
        if strategy.is_live() {
            let (bus_peak_l, bus_peak_r) =
                ramped_stereo_peaks(bus_buf_l, bus_buf_r, frames, bus_gain_l, bus_gain_r);
            bus.update_peak_l(bus_peak_l);
            bus.update_peak_r(bus_peak_r);
        }

        // Sum the bus output into master.
        sum_to_output(
            data, channels, frames, bus_buf_l, bus_buf_r, bus_gain_l, bus_gain_r,
        );
        if strategy.is_live() {
            bus.set_last_gains(bus_gain_l.1, bus_gain_r.1);
        }

        // Aux sends sourced from this bus, tapped after its own fader so
        // post-fader reflects the bus level (the pre-fader buffer is still
        // intact — `sum_to_output` only read it). The tapped signal lands
        // in the destination's summing buffer, which is only re-read if
        // that bus is processed later in this pass: return busses are
        // created after their feeder busses, so their index is higher and
        // the "returns after feeders" ordering holds. A send to an
        // earlier-indexed bus (already flushed this block) is skipped by
        // the natural ordering — its signal would otherwise be summed into
        // a buffer that's already gone to master.
        for send in aux_sends {
            if !send.enabled || send.source != SendSource::Bus(bus.id) {
                continue;
            }
            let Some(dst_idx) = busses_guard
                .get_index_of(&send.dest)
                .filter(|idx| *idx < active_busses && *idx != bus_idx)
            else {
                continue;
            };
            let send_lin = db_to_linear(send.level_db);
            let (send_gain_l, send_gain_r) = if send.pre_fader {
                ((send_lin, send_lin), (send_lin, send_lin))
            } else {
                (
                    (bus_gain_l.0 * send_lin, bus_gain_l.1 * send_lin),
                    (bus_gain_r.0 * send_lin, bus_gain_r.1 * send_lin),
                )
            };
            sum_bus_to_bus(bus_bufs, bus_idx, dst_idx, frames, send_gain_l, send_gain_r);
        }
    }
}
