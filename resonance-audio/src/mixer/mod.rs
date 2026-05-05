//! Audio mixing callback: runs on the cpal audio thread. Must be
//! allocation-free (uses pre-allocated buffers).
//!
//! The work is split across submodules by concern:
//! - [`midi_events`]: per-block MIDI tick→sample collection.
//! - [`monitor`]: live-input monitoring and per-track de-interleave.
//! - [`track_block`]: per-track / per-bus / sub-track mix orchestration.
//! - [`master`]: master FX insert chain + master volume / peaks.
//! - [`click`]: count-in and timeline metronome click synthesis.
//! - [`common`]: tiny helpers (pan-law gains, transport latching, the
//!   silent fallback playhead, the loop-seam panic routine).
//!
//! `mod.rs` itself only owns [`mix_audio`]: the top-level orchestrator
//! that snapshots tempo / transport, picks one of the count-in / no-play /
//! play / lock-contended branches, and stitches the per-block render
//! across the loop seam.

mod click;
mod common;
mod master;
mod midi_events;
mod monitor;
mod track_block;

pub(crate) use crate::limits::MAX_PLUGIN_OUTPUT_PORTS;
pub use midi_events::collect_midi_events_bounce;
pub(crate) use midi_events::MAX_MIDI_EVENTS_PER_BUFFER;

use ringbuf::traits::{Consumer, Observer};
use std::sync::atomic::Ordering;

use crate::engine::SharedState;
use crate::types::*;

use click::{render_count_in_clicks, render_metronome_clicks};
use common::{advance_playhead_silent, panic_instrument_tracks, sum_to_output, track_stereo_gains};
use master::{apply_master_fx_chain, apply_master_volume_and_peaks};
use monitor::process_monitor_track;
use track_block::render_timeline_block;

/// Snapshot of the tempo map taken once per audio buffer. Held while
/// the buffer renders so the bar/beat table stays stable across the
/// per-track render and the metronome pass.
struct TempoSnap {
    bpm: f64,
    num: u16,
    den: u16,
    metronome: bool,
}

/// Mix audio from all active clips into the output buffer.
/// This runs on the cpal audio callback thread -- must be allocation-free
/// (uses pre-allocated track_buf_l/track_buf_r).
#[allow(clippy::too_many_arguments)]
pub(crate) fn mix_audio(
    data: &mut [f32],
    channels: usize,
    shared: &SharedState,
    tracks: &parking_lot::RwLock<indexmap::IndexMap<TrackId, Track>>,
    busses: &parking_lot::RwLock<indexmap::IndexMap<BusId, Bus>>,
    master: &parking_lot::RwLock<MasterBus>,
    clips: &parking_lot::RwLock<Vec<AudioClip>>,
    midi_clips: &parking_lot::RwLock<Vec<MidiClip>>,
    plugins: &parking_lot::RwLock<
        indexmap::IndexMap<PluginInstanceId, parking_lot::Mutex<crate::clap_host::SyncClapInstance>>,
    >,
    tempo_map: &parking_lot::RwLock<TempoMap>,
    sample_rate: u32,
    track_buf_l: &mut Vec<f32>,
    track_buf_r: &mut Vec<f32>,
    bus_bufs: &mut [(Vec<f32>, Vec<f32>)],
    // Per-plugin-output-port scratch used for multi-output instruments
    // (e.g. resonance-drums with its 7 group/overhead ports). Sized to
    // `MAX_PLUGIN_OUTPUT_PORTS` pairs by the engine; mix_audio only
    // touches the first N slots on any given block, where N is the
    // active plugin's declared port count.
    port_scratch: &mut [(Vec<f32>, Vec<f32>)],
    note_event_buf: &mut Vec<PendingNoteEvent>,
    monitor_cons: &mut ringbuf::HeapCons<f32>,
    monitor_temp: &mut Vec<f32>,
    buf_frames: usize,
    quantum: usize,
) {
    resonance_common::flush_denormals();

    // Zero the output buffer
    data.fill(0.0);

    let output_frames = data.len() / channels;
    let frames = output_frames.min(buf_frames);

    // Snapshot tempo once per block. Hold the read guard so the bar
    // table is available for tempo-map-aware MIDI tick→sample conversion
    // in the rendering path. The read lock is held for one audio buffer
    // (~1 ms) — writers (engine thread) wait only during tempo changes.
    let playhead_now = shared.playhead.load(Ordering::Relaxed);
    let tempo_guard = tempo_map.try_read();
    let tempo_snap = tempo_guard.as_ref().map(|tm| TempoSnap {
        bpm: tm.bpm as f64,
        num: tm.numerator as u16,
        den: tm.denominator as u16,
        metronome: tm.metronome_enabled,
    });
    let snap_bpm = tempo_snap.as_ref().map(|s| s.bpm).unwrap_or(120.0);

    let transport_snap: Option<(f64, u16, u16, bool, f64)> = tempo_snap.as_ref().map(|s| {
        let playing = shared.playing.load(Ordering::Relaxed);
        let pos = playhead_now as f64 / sample_rate as f64 * s.bpm / 60.0;
        (s.bpm, s.num, s.den, playing, pos)
    });

    // Read monitor input with jitter margin to avoid underflows.
    // Skip stale monitor data to keep latency at ~1 buffer period.
    // The monitor stream carries raw interleaved multi-channel data now
    // (one `input_channels` block per frame), so sample counts scale by
    // the current input channel count.
    let input_channels = shared.input_channels.load(Ordering::Relaxed) as usize;
    let frame_stride = input_channels.max(1);
    let needed = frames * frame_stride;
    let available = monitor_cons.occupied_len();
    if available > needed + quantum * frame_stride {
        monitor_cons.skip(available - needed);
    }
    let to_read = needed.min(monitor_cons.occupied_len());
    let monitor_samples = monitor_cons.pop_slice(&mut monitor_temp[..to_read]);
    let monitor_frames = monitor_samples / frame_stride;

    // Count-in branch: hold the playhead, skip track/clip rendering,
    // and emit metronome ticks from a count-in-local elapsed counter
    // so the last click lands exactly one beat before the punch-in
    // line. `count_in_active` stays set across the brief window
    // between `count_in_remaining` hitting zero and the engine
    // control thread opening the recording stream, so the playhead
    // stays pinned to the punch-in line throughout.
    if shared.count_in_active.load(Ordering::Relaxed) {
        let count_in_remaining = shared.count_in_remaining.load(Ordering::Relaxed);
        let count_in_total = shared.count_in_total.load(Ordering::Relaxed);
        let elapsed_at_start = count_in_total.saturating_sub(count_in_remaining);
        let click_frames = (output_frames as u64).min(count_in_remaining) as usize;

        // Monitor pass-through so the performer can hear themselves
        // warm up during the count-in. Mirrors the playing=false
        // monitor branch but is best-effort on lock contention —
        // dropping monitor audio for one buffer is acceptable; losing
        // the count-in tick is not.
        if monitor_frames > 0 && shared.monitoring.load(Ordering::Relaxed) {
            if let (Some(tracks_guard), Some(plugins_guard)) =
                (tracks.try_read(), plugins.try_read())
            {
                let any_solo = tracks_guard.values().any(|t| t.soloed());
                let is_audible = |t: &&Track| -> bool {
                    t.monitor_enabled() && !t.muted() && (!any_solo || t.soloed())
                };
                if let Some(track) = tracks_guard.values().find(|t| is_audible(&t)) {
                    let processed_frames = process_monitor_track(
                        track,
                        monitor_temp,
                        monitor_frames,
                        monitor_frames,
                        input_channels,
                        track_buf_l,
                        track_buf_r,
                        &plugins_guard,
                        transport_snap,
                    );
                    let (gain_l, gain_r) = track_stereo_gains(track);
                    let mut peak_l = 0.0f32;
                    let mut peak_r = 0.0f32;
                    for f in 0..processed_frames {
                        peak_l = peak_l.max((track_buf_l[f] * gain_l).abs());
                        peak_r = peak_r.max((track_buf_r[f] * gain_r).abs());
                    }
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
                }
            }
        }

        // Metronome click synthesis using a count-in-local timeline.
        // Beats are indexed from the start of the count-in; with
        // `count_in_total == precount_bars * numerator * spb`, the
        // final click in the loop lands at elapsed
        // `(precount_bars * numerator - 1) * spb`, leaving exactly
        // one beat of silence before the punch-in line.
        if let Some(tm) = tempo_map.try_read() {
            render_count_in_clicks(
                data,
                channels,
                sample_rate,
                &tm,
                elapsed_at_start,
                click_frames,
            );
        }

        // Master volume + peaks so the count-in audio hits meters the
        // same way normal playback does.
        apply_master_volume_and_peaks(data, channels, shared);

        // Decrement the remaining-clicks counter. Once it hits zero
        // the metronome goes quiet, but `count_in_active` keeps the
        // mixer in this branch until the engine control thread has
        // actually opened the recording stream — that cross-thread
        // handoff is what guarantees the playhead doesn't start
        // advancing until recording is armed.
        let new_remaining = count_in_remaining.saturating_sub(output_frames as u64);
        shared
            .count_in_remaining
            .store(new_remaining, Ordering::Relaxed);
        return;
    }

    if !shared.playing.load(Ordering::Relaxed) {
        // Even when stopped, output monitored audio for armed tracks
        if monitor_frames > 0 && shared.monitoring.load(Ordering::Relaxed) {
            let (Some(tracks_guard), Some(plugins_guard)) = (tracks.try_read(), plugins.try_read())
            else {
                return;
            };
            let any_solo = tracks_guard.values().any(|t| t.soloed());
            let is_audible = |t: &&Track| -> bool {
                t.monitor_enabled() && !t.muted() && (!any_solo || t.soloed())
            };
            let any_monitor = tracks_guard.values().any(|t| is_audible(&t));

            if any_monitor {
                if let Some(track) = tracks_guard.values().find(|t| is_audible(&t)) {
                    let processed_frames = process_monitor_track(
                        track,
                        monitor_temp,
                        monitor_frames,
                        monitor_frames,
                        input_channels,
                        track_buf_l,
                        track_buf_r,
                        &plugins_guard,
                        transport_snap,
                    );

                    let (gain_l, gain_r) = track_stereo_gains(track);

                    // Compute post-fader peak levels for VU meters
                    let mut peak_l = 0.0f32;
                    let mut peak_r = 0.0f32;
                    for f in 0..processed_frames {
                        peak_l = peak_l.max((track_buf_l[f] * gain_l).abs());
                        peak_r = peak_r.max((track_buf_r[f] * gain_r).abs());
                    }
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
                }

                // Apply master volume and compute master peak levels
                apply_master_volume_and_peaks(data, channels, shared);
            }
        }
        return;
    }

    let playhead = shared.playhead.load(Ordering::Relaxed);

    let (
        Some(tracks_guard),
        Some(busses_guard),
        Some(clips_guard),
        Some(midi_clips_guard),
        Some(plugins_guard),
    ) = (
        tracks.try_read(),
        busses.try_read(),
        clips.try_read(),
        midi_clips.try_read(),
        plugins.try_read(),
    )
    else {
        // Lock contended -- advance playhead to avoid desync, output silence this buffer
        let new_playhead = advance_playhead_silent(shared, playhead, output_frames as u64);
        shared.playhead.store(new_playhead, Ordering::Relaxed);
        return;
    };

    let active_busses = busses_guard.len().min(bus_bufs.len());

    // Resolve a &TempoMap for tempo-map-aware MIDI tick→sample conversion.
    let default_tm = TempoMap::default();
    let tm_ref: &TempoMap = tempo_guard.as_deref().unwrap_or(&default_tm);

    let any_solo = tracks_guard
        .values()
        .filter(|t| t.sub_track_of.is_none())
        .any(|t| t.soloed());

    // Detect a loop seam inside this buffer. When the callback reaches or
    // crosses `loop_out`, we render two sub-blocks: the pre-wrap portion
    // from the current playhead, then (after an all-notes-off on instrument
    // plugins) the post-wrap portion starting from `loop_in`. This gives
    // sample-accurate cycle playback — no silent gap and no stray audio
    // from past `loop_out` bleeding across the seam.
    //
    // The `>=` on the end-of-block check is load-bearing: when the buffer
    // size divides the loop length exactly (common with small pro-audio
    // quanta like 128 frames), a strict `>` would miss the seam every time
    // — the block would end exactly on `loop_out` and the next block would
    // start past it, failing the `playhead < hi` test. With `>=`, that
    // aligned case renders the full block as `head` and sets `tail = 0`,
    // snapping the playhead back to `loop_in` for the next buffer.
    let seam_split: Option<(usize, usize, u64)> = if shared.loop_enabled.load(Ordering::Relaxed) {
        let lo = shared.loop_in.load(Ordering::Relaxed);
        let hi = shared.loop_out.load(Ordering::Relaxed);
        if hi > lo && playhead < hi && playhead + frames as u64 >= hi {
            let head = (hi - playhead) as usize;
            let tail = frames - head;
            Some((head, tail, lo))
        } else {
            None
        }
    } else {
        None
    };

    let new_playhead = if let Some((head_frames, tail_frames, loop_in)) = seam_split {
        // ---- Pre-wrap sub-block (plays to `loop_out`) ---------------------
        let head_monitor_frames = monitor_frames.min(head_frames);
        render_timeline_block(
            &mut data[..head_frames * channels],
            channels,
            &tracks_guard,
            &busses_guard,
            &clips_guard,
            &midi_clips_guard,
            &plugins_guard,
            tm_ref,
            sample_rate,
            any_solo,
            active_busses,
            playhead,
            head_frames,
            track_buf_l,
            track_buf_r,
            bus_bufs,
            port_scratch,
            note_event_buf,
            &monitor_temp[..head_monitor_frames * frame_stride],
            head_monitor_frames,
            input_channels,
            transport_snap,
        );

        // Flush instrument voices at the seam.
        panic_instrument_tracks(&tracks_guard, &plugins_guard);

        // ---- Post-wrap sub-block (plays from `loop_in`) -------------------
        let tail_monitor_start = head_monitor_frames * frame_stride;
        let tail_monitor_avail = monitor_frames.saturating_sub(head_monitor_frames);
        let tail_monitor_frames = tail_monitor_avail.min(tail_frames);
        render_timeline_block(
            &mut data[head_frames * channels..(head_frames + tail_frames) * channels],
            channels,
            &tracks_guard,
            &busses_guard,
            &clips_guard,
            &midi_clips_guard,
            &plugins_guard,
            tm_ref,
            sample_rate,
            any_solo,
            active_busses,
            loop_in,
            tail_frames,
            track_buf_l,
            track_buf_r,
            bus_bufs,
            port_scratch,
            note_event_buf,
            &monitor_temp
                [tail_monitor_start..tail_monitor_start + tail_monitor_frames * frame_stride],
            tail_monitor_frames,
            input_channels,
            transport_snap,
        );

        loop_in + tail_frames as u64
    } else {
        render_timeline_block(
            &mut data[..frames * channels],
            channels,
            &tracks_guard,
            &busses_guard,
            &clips_guard,
            &midi_clips_guard,
            &plugins_guard,
            tm_ref,
            sample_rate,
            any_solo,
            active_busses,
            playhead,
            frames,
            track_buf_l,
            track_buf_r,
            bus_bufs,
            port_scratch,
            note_event_buf,
            &monitor_temp[..monitor_frames * frame_stride],
            monitor_frames,
            input_channels,
            transport_snap,
        );
        playhead + output_frames as u64
    };

    // Master FX chain: run over the full callback buffer post-bus-sum,
    // before the metronome click is layered in and before the master
    // volume pass. Skipped when globally bypassed.
    if !shared.master_fx_bypassed.load(Ordering::Relaxed) {
        apply_master_fx_chain(
            data,
            channels,
            master,
            &plugins_guard,
            track_buf_l,
            track_buf_r,
            transport_snap,
        );
    }

    drop(plugins_guard);

    // Metronome click synthesis. When a loop seam split the callback, the
    // mapping from output frame index to timeline frame changes at the seam:
    // frames before `head_frames` play from `playhead`, frames after play
    // from `loop_in`.
    if let Some(snap) = tempo_snap.as_ref() {
        if snap.metronome {
            render_metronome_clicks(
                data,
                channels,
                sample_rate,
                tm_ref,
                snap_bpm,
                snap.num,
                output_frames,
                playhead,
                seam_split,
            );
        }
    }

    // Apply master volume, hard clip, and compute master peak levels
    apply_master_volume_and_peaks(data, channels, shared);

    shared.playhead.store(new_playhead, Ordering::Relaxed);
}
