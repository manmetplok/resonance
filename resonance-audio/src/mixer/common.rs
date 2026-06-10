//! Tiny helpers shared across the mixer submodules: transport latching,
//! pan-law gain math, the silent fallback playhead advance, and the
//! "panic" routine that flushes voices on instrument plugins at the
//! loop seam.

use indexmap::IndexMap;
use std::sync::atomic::Ordering;

use crate::clap_host::SyncClapInstance;
use crate::engine::SharedState;
use crate::types::*;

/// Latch a pre-captured transport snapshot onto a plugin instance so the
/// next `process()` call delivers it through the CLAP transport event.
#[inline]
pub(super) fn latch_transport(
    inst: &mut SyncClapInstance,
    snap: Option<(f64, u16, u16, bool, f64)>,
) {
    if let Some((bpm, num, den, playing, pos)) = snap {
        inst.0.set_transport(bpm, num, den, playing, pos);
    }
}

/// Fallback playhead advance used when the audio callback couldn't acquire
/// its locks. No audio is rendered on that path, so we only need to move
/// the playhead forward and handle the loop seam by snapping back — stuck
/// notes and audio content leakage aren't possible when we're outputting
/// silence. The sample-accurate seam handling lives inline in `mix_audio`.
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
            new_playhead = lo;
        }
    }
    new_playhead
}

/// Compute stereo gains for a track using equal-power pan law.
#[inline]
pub(super) fn track_stereo_gains(track: &Track) -> (f32, f32) {
    let volume = track.volume();
    let (pan_l, pan_r) = resonance_dsp::constant_power_pan(track.pan());
    (volume * pan_l, volume * pan_r)
}

/// Compute stereo gains for a bus using the same equal-power pan law.
#[inline]
pub(super) fn bus_stereo_gains(bus: &Bus) -> (f32, f32) {
    let volume = bus.volume();
    let (pan_l, pan_r) = resonance_dsp::constant_power_pan(bus.pan());
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
pub(super) fn panic_instrument_tracks(
    tracks_guard: &IndexMap<TrackId, Track>,
    plugins_guard: &IndexMap<PluginInstanceId, parking_lot::Mutex<SyncClapInstance>>,
    midi_stash: &mut super::midi_stash::MidiStash,
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
            inst.0.all_notes_off();
            // Stashed pre-seam events are superseded by the panic.
            midi_stash.discard(inst_id);
        } else {
            midi_stash.request_panic(inst_id);
        }
    }
}
