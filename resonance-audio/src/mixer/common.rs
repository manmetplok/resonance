//! Tiny helpers shared across the mixer submodules: transport latching,
//! pan-law gain math, the silent fallback playhead advance, and the
//! "panic" routine that flushes voices on instrument plugins at the
//! loop seam.

use indexmap::IndexMap;
use std::sync::atomic::Ordering;

use crate::clap_host::{PluginMap, SyncClapInstance};
use crate::engine::SharedState;
use crate::types::*;

/// Beat position for the CLAP transport event, derived through the
/// tempo map's bar table so tempo-synced plugins stay locked under
/// tempo ramps (a flat samples-per-beat factor drifts).
#[inline]
pub fn transport_pos_beats(map: &TempoMap, sample_pos: u64, sample_rate: u32) -> f64 {
    map.sample_to_abs_tick(sample_pos, sample_rate) as f64 / TICKS_PER_QUARTER_NOTE as f64
}

/// Transport snapshot captured once per audio buffer and latched onto
/// each plugin before `process()` so every CLAP transport event in the
/// block agrees on tempo, meter, and position.
#[derive(Clone, Copy)]
pub(crate) struct TransportSnap {
    pub bpm: f64,
    pub num: u16,
    pub den: u16,
    pub playing: bool,
    pub pos_beats: f64,
}

/// Latch a pre-captured transport snapshot onto a plugin instance so the
/// next `process()` call delivers it through the CLAP transport event.
#[inline]
pub(super) fn latch_transport(inst: &mut SyncClapInstance, snap: Option<TransportSnap>) {
    if let Some(s) = snap {
        inst.0.set_transport(s.bpm, s.num, s.den, s.playing, s.pos_beats);
    }
}

/// Publish the audio thread's playhead advance for a block that observed
/// the playhead at `observed`, unless the engine control thread moved it
/// in the meantime.
///
/// The callback reads the playhead once at the top of the block, renders
/// (the whole render time is the window), and publishes `observed +
/// frames` here. A Seek, a Stop-to-zero or a MIDI-clock song position
/// that landed in between is a plain `store` from the control thread;
/// an unconditional store here used to clobber it (code review MIX-01).
/// The compare-exchange makes the reposition win: it returns `false` and
/// leaves the playhead where the control thread put it, so the next
/// block simply starts there.
///
/// `false` therefore means "the transport was repositioned under this
/// block" — the hook a discontinuity handler (voice flush on seek) can
/// build on. Lock-free, allocation-free; `pub` for the race test.
#[inline]
pub fn commit_playhead(shared: &SharedState, observed: u64, new_playhead: u64) -> bool {
    shared
        .playhead
        .compare_exchange(observed, new_playhead, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
}

/// The audio thread's record of where the transport should be next, so it
/// can tell a playhead jump from continuous playback (code review MIX-06).
///
/// The engine control thread repositions the transport with plain stores
/// (seek, stop, MIDI-clock relocate) and flushes voices with a `try_lock`
/// panic that silently skips an instrument the audio thread is holding.
/// The callback owns the `MidiStash`, so a panic it issues is never lost
/// (it is parked on contention); this lets it issue one whenever the
/// block it is about to render does not continue the last one it
/// rendered. Audio-thread owned; two words, no allocation.
#[derive(Default)]
pub(crate) struct TransportContinuity {
    /// Where the last rendered playing block said the next one starts;
    /// `None` while the transport is not rolling.
    expected: Option<u64>,
    /// The last block's `commit_playhead` lost: the control thread moved
    /// the transport under it. Flush regardless of where it moved to.
    repositioned: bool,
    /// A block skipped since the last render dropped a NoteOff (or a
    /// seam panic) the instruments never saw (FU-M3b).
    lost_events: bool,
}

impl TransportContinuity {
    /// A playing block is about to render from `playhead`: whether it
    /// does NOT continue the previous rendered block, so held voices must
    /// be flushed first. A block skipped in between (the A/B reference,
    /// an offline render) never updated `expected`, so the advance it
    /// made reads as a jump too — its NoteOffs were never collected. A
    /// lock-contended block reports itself through [`Self::skipped`] and
    /// forces a flush only if it actually lost a NoteOff.
    pub(crate) fn jumped(&self, playhead: u64) -> bool {
        self.repositioned || self.lost_events || self.expected.is_some_and(|e| e != playhead)
    }

    /// A playing block rendered and published its advance to `next`;
    /// `committed` is `commit_playhead`'s result.
    pub(crate) fn rendered(&mut self, next: u64, committed: bool) {
        self.expected = Some(next);
        self.repositioned = !committed;
        self.lost_events = false;
    }

    /// A lock-contended playing block advanced `from` → `to` silently
    /// (FU-M3b). If it continued the last rendered block and its advance
    /// stuck, playback stays continuous and the next block flushes only
    /// when `lost` — a NoteOff (or a loop seam's panic) fell inside it.
    /// Otherwise `expected` stays behind and the next block reads as a
    /// jump, as before. Every contended block used to flush, cutting
    /// sustained notes during heavy UI edits.
    pub(crate) fn skipped(&mut self, from: u64, to: u64, committed: bool, lost: bool) {
        if committed && self.expected == Some(from) {
            self.expected = Some(to);
            self.lost_events |= lost;
        }
    }

    /// Whether the transport was rolling when it stopped — the stopped
    /// branch then flushes the run's voices once.
    pub(crate) fn was_rolling(&self) -> bool {
        self.expected.is_some()
    }

    /// The voices have been flushed for a stop.
    pub(crate) fn stopped(&mut self) {
        *self = Self::default();
    }
}

/// Fallback playhead advance used when the audio callback couldn't acquire
/// its locks. No audio is rendered on that path, so we only need to move
/// the playhead forward and wrap at the loop seam — stuck notes and audio
/// content leakage aren't possible when we're outputting silence. The
/// wrap carries the overshoot past `loop_in` exactly like the rendering
/// path's `loop_in + tail_frames` (code review MIX-11); a bare snap to
/// `loop_in` lost up to a buffer of timeline per pass. The sample-accurate
/// seam handling lives inline in `mix_audio`.
pub(super) fn advance_playhead_silent(
    shared: &SharedState,
    playhead: u64,
    frames: u64,
) -> u64 {
    let mut new_playhead = playhead + frames;
    if shared.loop_enabled.load(Ordering::Relaxed) {
        let lo = shared.loop_in.load(Ordering::Relaxed);
        let hi = shared.loop_out.load(Ordering::Relaxed);
        // `>=` matches the main path: when `new_playhead == hi` exactly, we
        // still need to snap back, or the next buffer lands past the loop
        // and never catches up.
        if hi > lo && playhead < hi && new_playhead >= hi {
            // Modulo keeps a loop shorter than one buffer inside the loop.
            new_playhead = lo + (new_playhead - hi) % (hi - lo);
        }
    }
    new_playhead
}

/// Compute stereo gains for a track from its fader and pan.
///
/// The pan control is a stereo BALANCE, not a constant-power pan: centre
/// is unity on both channels, and panning attenuates the far side only
/// (`resonance_dsp::stereo_balance`).
///
/// Every track in this engine carries a stereo signal — instruments
/// render stereo, clips are stereo-interleaved, the monitor path is
/// de-interleaved to a pair — so the constant-power law this used to
/// apply was pricing in a mono-to-stereo spread that never happens. Its
/// centre gain of 1/sqrt(2) meant a stem rendered from the project and
/// placed straight back measured 3.01 dB below its source, on a fader at
/// 0 dB with clip gain at 0 (ba doc #276 BUG 3). A hard-panned track is
/// unchanged by the switch; centre-panned material comes up 3 dB.
#[inline]
pub(super) fn track_stereo_gains(track: &Track) -> (f32, f32) {
    let volume = track.volume();
    let (pan_l, pan_r) = resonance_dsp::stereo_balance(track.pan());
    (volume * pan_l, volume * pan_r)
}

/// Compute stereo gains for a bus using the same balance law.
#[inline]
pub(super) fn bus_stereo_gains(bus: &Bus) -> (f32, f32) {
    let volume = bus.volume();
    let (pan_l, pan_r) = resonance_dsp::stereo_balance(bus.pan());
    (volume * pan_l, volume * pan_r)
}

/// Per-sample linear gain ramp across a block. Frame `f` (0-based) of
/// `frames` gets `from + (to - from) * (f + 1) / frames`: the previous
/// block ended exactly on `from`, so the first sample already steps
/// toward `to` and the last sample lands exactly on it. With
/// `from == to` this degenerates to the constant gain. `pub` for the
/// gain-ramp integration tests.
#[inline(always)]
pub fn ramped_gain(gain: (f32, f32), inv_frames: f32, f: usize) -> f32 {
    gain.0 + (gain.1 - gain.0) * ((f + 1) as f32 * inv_frames)
}

/// Accumulate a source track buffer into a destination stereo pair
/// (separate L/R Vecs, as used by bus summing buffers), ramping each
/// channel's gain linearly from `gain.0` (previous block) to `gain.1`
/// (current block) across the block.
#[inline]
pub fn sum_to_stereo(
    dst_l: &mut [f32],
    dst_r: &mut [f32],
    frames: usize,
    src_l: &[f32],
    src_r: &[f32],
    gain_l: (f32, f32),
    gain_r: (f32, f32),
) {
    if frames == 0 {
        return;
    }
    let inv_frames = 1.0 / frames as f32;
    for f in 0..frames {
        dst_l[f] += src_l[f] * ramped_gain(gain_l, inv_frames, f);
        dst_r[f] += src_r[f] * ramped_gain(gain_r, inv_frames, f);
    }
}

/// Sum track buffers into the interleaved output, ramping each
/// channel's gain linearly from `gain.0` to `gain.1` across the block.
#[inline]
pub fn sum_to_output(
    data: &mut [f32],
    channels: usize,
    frames: usize,
    track_buf_l: &[f32],
    track_buf_r: &[f32],
    gain_l: (f32, f32),
    gain_r: (f32, f32),
) {
    if frames == 0 {
        return;
    }
    let inv_frames = 1.0 / frames as f32;
    for f in 0..frames {
        let out_idx = f * channels;
        let gl = ramped_gain(gain_l, inv_frames, f);
        let gr = ramped_gain(gain_r, inv_frames, f);
        if channels >= 2 {
            data[out_idx] += track_buf_l[f] * gl;
            data[out_idx + 1] += track_buf_r[f] * gr;
        } else {
            data[out_idx] += track_buf_l[f] * gl + track_buf_r[f] * gr;
        }
    }
}

/// Post-fader stereo peak levels with the same gain ramp the sum
/// helpers apply, so VU meters match what was actually mixed.
#[inline]
pub(super) fn ramped_stereo_peaks(
    src_l: &[f32],
    src_r: &[f32],
    frames: usize,
    gain_l: (f32, f32),
    gain_r: (f32, f32),
) -> (f32, f32) {
    let mut peak_l = 0.0f32;
    let mut peak_r = 0.0f32;
    if frames == 0 {
        return (peak_l, peak_r);
    }
    let inv_frames = 1.0 / frames as f32;
    for f in 0..frames {
        peak_l = peak_l.max((src_l[f] * ramped_gain(gain_l, inv_frames, f)).abs());
        peak_r = peak_r.max((src_r[f] * ramped_gain(gain_r, inv_frames, f)).abs());
    }
    (peak_l, peak_r)
}

/// Fire all-notes-off on every instrument track's primary plugin. Used at
/// the loop seam to prevent notes started before `loop_out` from hanging
/// after the playhead snaps back to `loop_in`. If the lock is contended,
/// the panic is parked in the MIDI stash and fires on the next
/// successful lock instead of being lost.
///
/// `at_seam` keeps the note events an instrument carried past the head
/// sub-block — they belong to the tail, after the seam. Every other
/// panic (Stop, relocate) drops them too (FU-F2a); a parked panic always
/// does (see `MidiStash::deliver`).
pub(super) fn panic_instrument_tracks(
    tracks_guard: &IndexMap<TrackId, Track>,
    plugins_guard: &PluginMap,
    midi_stash: &mut super::midi_stash::MidiStash,
    at_seam: bool,
) {
    for track in tracks_guard.values() {
        if !track.track_type.accepts_midi() {
            continue;
        }
        let Some(inst_id) = track.plugins().first().copied() else {
            continue;
        };
        let Some(mutex) = plugins_guard.get(&inst_id) else {
            continue;
        };
        if let Some(mut inst) = mutex.try_lock() {
            if at_seam {
                inst.0.all_notes_off();
            } else {
                inst.0.all_notes_off_and_drop_carried();
            }
            // Stashed pre-seam events are superseded by the panic.
            midi_stash.discard(inst_id);
        } else {
            midi_stash.request_panic(inst_id);
        }
    }
}
