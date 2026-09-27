//! Per-call policy for the bits of the render block that differ between
//! the live callback and the offline bounce: plugin locking, gating,
//! gain-ramp endpoints, live-only meters and the monitor-input mix.
//!
//! See the [`super::super::render_core`] module docs for what "Live" and
//! "Bounce" mean; the decisions themselves are documented per method.

use parking_lot::{Mutex, MutexGuard};

use crate::clap_host::SyncClapInstance;
use crate::mixer::common::{bus_stereo_gains, latch_transport, track_stereo_gains, TransportSnap};
use crate::types::*;

/// Automated stereo-gain ramp endpoints `((l_start, l_end), (r_start,
/// r_end))` for one block, or `None` when no gain/pan lane targets the
/// track/bus.
pub(crate) type AutoGain = Option<((f32, f32), (f32, f32))>;

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

/// Whether a top-level track is silenced by mute / solo this block. The
/// one mute/solo resolution shared by the live mixer and the bounce (code
/// review MIX-07), so an export can never disagree with playback about
/// who is heard. `muted` already folds in any mute automation.
///
/// Reads `track.block_soloed()` — the snapshot `snapshot_top_level_solo`
/// latched at the top of this block — rather than `track.soloed()`
/// directly (FU-B3a). `any_solo` and every track's own flag must come
/// from the same instant: a live re-read here could straddle a solo
/// toggle against the moment `any_solo` was computed and silence the
/// whole block (the aggregate says "someone is soloed" while this track's
/// fresh read says "not me", and every other track was decided the same
/// stale way).
#[inline]
pub(crate) fn track_silenced(track: &Track, muted: bool, any_solo: bool) -> bool {
    muted || (any_solo && !track.block_soloed())
}

/// Whether a sub-track is silenced this block. Sub-tracks follow their
/// parent's solo (their own solo flag is ignored, exactly as
/// `any_top_level_solo` ignores it), so only their own mute and a
/// silenced parent count. Shared by live and bounce (code review MIX-07).
#[inline]
pub(crate) fn sub_track_silenced(muted: bool, parent_silenced: bool) -> bool {
    muted || parent_silenced
}

/// Per-call policy for the bits of the render block that differ between
/// the live callback and the offline bounce. See the module docs.
pub(crate) enum RenderStrategy<'a> {
    Live {
        transport_snap: Option<TransportSnap>,
        monitor_temp: &'a [f32],
        monitor_frames: usize,
        input_channels: usize,
    },
    Bounce {
        in_filter: &'a (dyn Fn(TrackId) -> bool + Sync),
        /// Tracks that are in the filter ONLY to drive their sub-tracks'
        /// port fan-out (ba todo #1242). Their instrument runs — a
        /// sub-track has no other source of audio — but their own main
        /// output (port 0) is discarded before the track's FX chain,
        /// fader, aux sends and routing, so a sub-track stem carries that
        /// tap and nothing else. Always `false` outside stem rendering.
        fan_out_only: &'a (dyn Fn(TrackId) -> bool + Sync),
        /// Tracks that are in the filter ONLY to be captured as a
        /// sidechain key (ba doc #277). They render through their whole
        /// chain — otherwise there is no audio to capture — and are then
        /// dropped before PDC, fader, aux sends and routing, so they key
        /// the stem without joining it. Always `false` outside stem
        /// rendering, where every track renders anyway.
        key_only: &'a (dyn Fn(TrackId) -> bool + Sync),
        /// The bus twin of `key_only`.
        key_only_bus: &'a (dyn Fn(BusId) -> bool + Sync),
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
pub(crate) struct TrackDisposition {
    /// `(previous, target)` gain ramp endpoints per channel. Bounce uses
    /// equal endpoints, which the ramp helpers reduce to a constant.
    pub(crate) gain_l: (f32, f32),
    pub(crate) gain_r: (f32, f32),
    /// Live: muted or solo-suppressed. Inherited by sub-tracks so a
    /// silenced parent fades its fan-out in the same block.
    pub(crate) silenced: bool,
    /// Live: the instrument still runs (NoteOffs keep flowing, voices
    /// don't stick on unmute) but its output is discarded once the mute
    /// ramp has fully faded the previous gain to zero.
    pub(crate) discard_after_instrument: bool,
    /// Bounce/stem: the instrument runs and its extra ports still fan out
    /// to sub-tracks, but this track's OWN main output (port 0) is
    /// discarded — it is only here to drive somebody else's fan-out (ba
    /// todo #1242). Unlike `discard_after_instrument` this must NOT skip
    /// the rest of the iteration, or the fan-out never happens.
    pub(crate) discard_own_output: bool,
    /// Silenced by mute / solo, but a sidechain key is tapped from it (or
    /// from one of its sub-tracks): render the source and FX chain so the
    /// key is captured — taps are post-FX, pre-fader, and must not depend
    /// on the source's mute (code review MIX-05) — then stop before PDC,
    /// fader, aux sends and routing, so nothing of it is heard.
    pub(crate) key_only: bool,
}

impl TrackDisposition {
    /// A silenced track rendered only for its sidechain key (see
    /// [`TrackDisposition::key_only`]). Marked `silenced` so its
    /// sub-tracks follow it out and fall to key-only themselves.
    pub(crate) fn key_only() -> Self {
        Self {
            gain_l: (0.0, 0.0),
            gain_r: (0.0, 0.0),
            silenced: true,
            discard_after_instrument: false,
            discard_own_output: false,
            key_only: true,
        }
    }
}

impl RenderStrategy<'_> {
    /// Whether this track takes part in this render at all, mute / solo
    /// aside: always live; only in-filter tracks in a bounce / stem. A
    /// silenced track that a key is tapped from renders key-only exactly
    /// when this holds (code review MIX-05).
    #[inline]
    pub(crate) fn renders(&self, id: TrackId) -> bool {
        match self {
            Self::Live { .. } => true,
            Self::Bounce { in_filter, .. } => in_filter(id),
        }
    }

    /// Live-only side effects: VU peak meters and the last-gain atomics
    /// that seed the next block's ramp. Bounce must not touch either —
    /// it can run while live playback owns them.
    #[inline]
    pub(crate) fn is_live(&self) -> bool {
        matches!(self, Self::Live { .. })
    }

    /// True when this track is rendered only so it can be captured as a
    /// key — see `RenderStrategy::Bounce::key_only`.
    #[inline]
    pub(crate) fn is_key_only(&self, id: TrackId) -> bool {
        match self {
            Self::Live { .. } => false,
            Self::Bounce { key_only, .. } => key_only(id),
        }
    }

    /// The bus twin of [`RenderStrategy::is_key_only`].
    #[inline]
    pub(crate) fn is_key_only_bus(&self, id: BusId) -> bool {
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
    pub(crate) fn force_master_route(&self) -> bool {
        matches!(self, Self::Bounce { freeze_raw: true, .. })
    }

    /// Acquire an effect plugin's lock. Live: non-blocking, skipping the
    /// plugin for this block on contention (and latching the transport
    /// snapshot on success). Bounce: blocking with spin + back-off.
    #[inline]
    pub(crate) fn lock_fx<'p>(
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

    /// Acquire an instrument plugin's lock — as [`Self::lock_fx`]. The
    /// caller replays MIDI parked during earlier contention (its slot's
    /// stash carry) before queueing this block's events.
    #[inline]
    pub(crate) fn lock_instrument<'p>(
        &self,
        mutex: &'p Mutex<SyncClapInstance>,
    ) -> Option<MutexGuard<'p, SyncClapInstance>> {
        self.lock_fx(mutex)
    }

    /// Decide whether and how a top-level track renders this block.
    /// `auto_gain` / `auto_mute` carry this block's automation overrides
    /// (already resolved against the lane snapshot); both are `None` when
    /// nothing automates the track, in which case the static fader / pan
    /// / mute apply exactly as before.
    pub(crate) fn track_disposition(
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
                let silenced = track_silenced(track, muted, any_solo);
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
                    key_only: false,
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
                if *respect_mute_solo && track_silenced(track, muted, any_solo) {
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
                    key_only: false,
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
    pub(crate) fn sub_track_disposition(
        &self,
        sub_track: &Track,
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
                let sub_silenced = sub_track_silenced(muted, parent_silenced);
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
                // Same resolution as live: a sub-track follows its
                // parent's solo, never its own flag (code review MIX-07).
                // A solo-suppressed parent only reaches its fan-out here
                // when it renders key-only (code review MIX-05), and then
                // its taps follow it out.
                if *respect_mute_solo && sub_track_silenced(muted, parent_silenced) {
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
    pub(crate) fn bus_disposition(
        &self,
        bus: &Bus,
        auto_gain: AutoGain,
        auto_mute: Option<bool>,
    ) -> Option<((f32, f32), (f32, f32))> {
        let muted = bus.muted() || auto_mute.unwrap_or(false);
        match self {
            Self::Live { .. } => {
                // A bus that has never completed a live block has no real
                // "previous gain" to ramp from: `last_gains()` is either
                // its zero construction placeholder or a target it was
                // given before it ever rendered (fader/mute set right
                // after `AddBus`, or a diff-path undo re-add). Muted and
                // never rendered stays silent outright, same as an
                // already-faded-down muted bus (FU-B6a).
                let never_rendered = !bus.rendered_once();
                let (bus_last_l, bus_last_r) = bus.last_gains();
                if muted && (never_rendered || (bus_last_l == 0.0 && bus_last_r == 0.0)) {
                    return None;
                }
                let (bus_target_l, bus_target_r) = if muted {
                    (0.0, 0.0)
                } else if let Some(((_, gl_end), (_, gr_end))) = auto_gain {
                    (gl_end, gr_end)
                } else {
                    bus_stereo_gains(bus)
                };
                // First-ever live block: render flat at the target rather
                // than ramping in from the placeholder above — otherwise a
                // track re-routed onto this brand-new bus dips for one
                // block while the bus fades in from silence it never
                // actually had.
                let (bus_last_l, bus_last_r) = if never_rendered {
                    (bus_target_l, bus_target_r)
                } else {
                    (bus_last_l, bus_last_r)
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
    pub(crate) fn mix_monitor(
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
