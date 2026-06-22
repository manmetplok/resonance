//! Audio mixing callback: runs on the cpal audio thread. Must be
//! allocation-free (uses pre-allocated buffers).
//!
//! The work is split across submodules by concern:
//! - [`midi_events`]: per-block MIDI tick→sample collection.
//! - [`monitor`]: live-input monitoring and per-track de-interleave.
//! - [`render_core`]: per-track / per-bus / sub-track render core shared
//!   with the offline bounce path, parameterized by `RenderStrategy`.
//! - [`track_block`]: live wrapper over `render_core`.
//! - [`master`]: master FX insert chain + master volume / peaks.
//! - [`click`]: count-in and timeline metronome click synthesis.
//! - [`common`]: tiny helpers (pan-law gains, transport latching, the
//!   silent fallback playhead, the loop-seam panic routine).
//!
//! `mod.rs` itself only owns [`mix_audio`]: the top-level orchestrator
//! that snapshots tempo / transport, picks one of the count-in / no-play /
//! play / lock-contended branches, and stitches the per-block render
//! across the loop seam.

mod audition;
mod click;
mod common;
mod master;
mod midi_events;
mod midi_stash;
mod monitor;
mod render_core;
mod track_block;

pub(crate) use crate::limits::MAX_PLUGIN_OUTPUT_PORTS;
pub use common::{ramped_gain, sum_to_output, sum_to_stereo, transport_pos_beats};
pub use midi_events::collect_midi_events_bounce;
pub(crate) use midi_events::MAX_MIDI_EVENTS_PER_BUFFER;
pub use midi_stash::{MidiStash, NoteSink};
pub(crate) use render_core::{render_block, RenderStrategy};
pub use render_core::mix_track_clips;

use ringbuf::traits::{Consumer, Observer};
use std::sync::atomic::Ordering;

use crate::engine::SharedState;
use crate::types::*;

pub use audition::mix_audition_overlay;

/// Test-only harness around [`render_block`]: assembles the `IndexMap` /
/// scratch-buffer plumbing from plain vecs, renders one offline (Bounce)
/// block at playhead 0, and returns the interleaved-stereo master output
/// plus the per-bus summing buffers. After the call each bus buffer still
/// holds that bus's accumulated signal *before* its own fader — for a
/// return bus that is exactly the summed aux-send contribution, so an
/// integration test can assert the tap in isolation. Keeps `indexmap` and
/// the render scaffolding out of the `tests/` crate.
#[doc(hidden)]
#[allow(clippy::type_complexity)]
pub fn render_aux_for_test(
    tracks: Vec<Track>,
    busses: Vec<Bus>,
    clips: Vec<AudioClip>,
    aux_sends: Vec<AuxSend>,
    frames: usize,
    sample_rate: u32,
) -> (Vec<f32>, Vec<(Vec<f32>, Vec<f32>)>) {
    use indexmap::IndexMap;

    let tracks_guard: IndexMap<TrackId, Track> = tracks.into_iter().map(|t| (t.id, t)).collect();
    let busses_guard: IndexMap<BusId, Bus> = busses.into_iter().map(|b| (b.id, b)).collect();
    let plugins_guard: IndexMap<
        PluginInstanceId,
        parking_lot::Mutex<crate::clap_host::SyncClapInstance>,
    > = IndexMap::new();
    let midi_clips: Vec<MidiClip> = Vec::new();
    let tempo_map = TempoMap::default();
    let latency = crate::latency::LatencyComp::empty();
    let active_busses = busses_guard.len();

    let mut data = vec![0.0f32; frames * 2];
    let mut track_buf_l = vec![0.0f32; frames];
    let mut track_buf_r = vec![0.0f32; frames];
    let mut bus_bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..active_busses)
        .map(|_| (vec![0.0f32; frames], vec![0.0f32; frames]))
        .collect();
    let mut port_scratch: Vec<(Vec<f32>, Vec<f32>)> = Vec::new();
    let mut note_buf: Vec<PendingNoteEvent> = Vec::new();

    let in_filter = |_id: TrackId| true;
    let mut strategy = RenderStrategy::Bounce {
        in_filter: &in_filter,
        respect_mute_solo: false,
    };

    render_block(
        &mut data,
        2,
        &tracks_guard,
        &busses_guard,
        &clips,
        &midi_clips,
        &plugins_guard,
        &tempo_map,
        sample_rate,
        false,
        active_busses,
        &aux_sends,
        0,
        frames,
        &mut track_buf_l,
        &mut track_buf_r,
        &mut bus_bufs,
        &mut port_scratch,
        &mut note_buf,
        &latency,
        &mut strategy,
    );

    (data, bus_bufs)
}

use click::{render_count_in_clicks, render_metronome_clicks};
use common::{advance_playhead_silent, panic_instrument_tracks, TransportSnap};
use master::{apply_master_fx_chain, apply_master_volume_and_peaks};
use monitor::mix_monitor_passthrough;
use track_block::render_timeline_block;

/// One-shot warning when cpal requests a buffer larger than our
/// pre-allocated scratch. Latches via `AtomicBool` so the audio thread
/// doesn't flood stderr; subsequent oversize buffers are silently
/// clamped (audio plays slower than real-time, but does not desync).
fn log_oversize_buffer(requested: usize, scratch: usize) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "audio: cpal requested buf={} frames but scratch is {} — clamping; audio will run slow",
            requested, scratch
        );
    }
}

/// Largest sample count ≤ both `len` and `vacant` that is a whole
/// number of `frame_stride`-sample frames. Producers push exactly this
/// many samples so a full ring can't rotate the channel interleave.
#[inline]
pub fn whole_frame_push_len(len: usize, vacant: usize, frame_stride: usize) -> usize {
    len.min(vacant) / frame_stride * frame_stride
}

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
    tempo_map: &arc_swap::ArcSwap<TempoMap>,
    latency_comp: &arc_swap::ArcSwap<crate::latency::LatencyComp>,
    sample_rate: u32,
    track_buf_l: &mut [f32],
    track_buf_r: &mut [f32],
    bus_bufs: &mut [(Vec<f32>, Vec<f32>)],
    // Per-plugin-output-port scratch used for multi-output instruments
    // (e.g. resonance-drums with its 7 group/overhead ports). Sized to
    // `MAX_PLUGIN_OUTPUT_PORTS` pairs by the engine; mix_audio only
    // touches the first N slots on any given block, where N is the
    // active plugin's declared port count.
    port_scratch: &mut [(Vec<f32>, Vec<f32>)],
    note_event_buf: &mut Vec<PendingNoteEvent>,
    midi_stash: &mut MidiStash,
    monitor_cons: &mut ringbuf::HeapCons<f32>,
    monitor_temp: &mut [f32],
    buf_frames: usize,
    quantum: usize,
) {
    resonance_common::flush_denormals();

    // Zero the output buffer
    data.fill(0.0);

    let raw_output_frames = data.len() / channels;
    let frames = raw_output_frames.min(buf_frames);
    // If cpal hands us a buffer larger than our scratch can hold (only
    // possible under the BufferSize::Default fallback path), clamp every
    // downstream calculation to what we can actually render. Advancing
    // by `raw_output_frames` while only rendering `frames` would race
    // the playhead past the audio and silently miss loop seams. The
    // OS-side tail of the buffer stays at zero from `data.fill(0.0)`.
    if raw_output_frames > buf_frames {
        log_oversize_buffer(raw_output_frames, buf_frames);
    }
    let output_frames = frames;

    // Snapshot tempo once per block. Hold the ArcSwap guard so the bar
    // table is available for tempo-map-aware MIDI tick→sample conversion
    // in the rendering path. The guard pins this block's snapshot; the
    // engine thread publishes tempo changes wait-free via ArcSwap::store.
    let playhead_now = shared.playhead.load(Ordering::Relaxed);
    let tempo_guard = tempo_map.load();
    let tempo_snap = TempoSnap {
        bpm: tempo_guard.bpm as f64,
        num: tempo_guard.numerator as u16,
        den: tempo_guard.denominator as u16,
        metronome: tempo_guard.metronome_enabled,
    };
    let snap_bpm = tempo_snap.bpm;

    let transport_snap = Some(TransportSnap {
        bpm: tempo_snap.bpm,
        num: tempo_snap.num,
        den: tempo_snap.den,
        playing: shared.playing.load(Ordering::Relaxed),
        pos_beats: transport_pos_beats(&tempo_guard, playhead_now, sample_rate),
    });

    // Read monitor input with jitter margin to avoid underflows.
    // Skip stale monitor data to keep latency at ~1 buffer period.
    // The monitor stream carries raw interleaved multi-channel data now
    // (one `input_channels` block per frame), so sample counts scale by
    // the current input channel count.
    let input_channels = shared.input_channels.load(Ordering::Relaxed) as usize;
    let frame_stride = input_channels.max(1);
    // Skip and read in whole frames only: a sub-frame skip or read
    // would permanently rotate the channel interleave.
    let needed = frames * frame_stride;
    let available = monitor_cons.occupied_len();
    let catchup = monitor_catchup_skip(available, needed, quantum, frame_stride);
    if catchup > 0 {
        monitor_cons.skip(catchup);
    }
    let to_read = monitor_read_len(needed, monitor_cons.occupied_len(), frame_stride);
    let monitor_samples = monitor_cons.pop_slice(&mut monitor_temp[..to_read]);
    let monitor_frames = monitor_samples / frame_stride;

    // The arrangement render below has several early-exit branches (count-in,
    // not-playing, lock-contended). Wrap them in a labeled block so each one
    // `break`s to a common tail that mixes the audition preview overlay — the
    // preview must be audible regardless of which branch the arrangement took.
    'arrangement: {
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
                mix_monitor_passthrough(
                    data,
                    channels,
                    &tracks_guard,
                    &plugins_guard,
                    monitor_temp,
                    monitor_frames,
                    input_channels,
                    track_buf_l,
                    track_buf_r,
                    transport_snap,
                );
            }
        }

        // Metronome click synthesis using a count-in-local timeline.
        // Beats are indexed from the start of the count-in; with
        // `count_in_total == precount_bars * numerator * spb`, the
        // final click in the loop lands at elapsed
        // `(precount_bars * numerator - 1) * spb`, leaving exactly
        // one beat of silence before the punch-in line.
        render_count_in_clicks(
            data,
            channels,
            sample_rate,
            &tempo_guard,
            elapsed_at_start,
            click_frames,
        );

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
        break 'arrangement;
    }

    if !shared.playing.load(Ordering::Relaxed) {
        // Even when stopped, output monitored audio for armed tracks
        if monitor_frames > 0 && shared.monitoring.load(Ordering::Relaxed) {
            let (Some(tracks_guard), Some(plugins_guard)) = (tracks.try_read(), plugins.try_read())
            else {
                break 'arrangement;
            };
            let any_monitor = mix_monitor_passthrough(
                data,
                channels,
                &tracks_guard,
                &plugins_guard,
                monitor_temp,
                monitor_frames,
                input_channels,
                track_buf_l,
                track_buf_r,
                transport_snap,
            );
            if any_monitor {
                // Apply master volume and compute master peak levels
                apply_master_volume_and_peaks(data, channels, shared);
            }
        }
        break 'arrangement;
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
        break 'arrangement;
    };

    let active_busses = busses_guard.len().min(bus_bufs.len());

    // Resolve a &TempoMap for tempo-map-aware MIDI tick→sample conversion.
    let tm_ref: &TempoMap = &tempo_guard;

    // Snapshot the plugin-delay-compensation table once per buffer.
    // Wait-free load; the engine thread publishes a new table whenever
    // the track/bus/plugin topology changes.
    let comp_guard = latency_comp.load();
    let comp_ref: &crate::latency::LatencyComp = &comp_guard;

    // Snapshot the aux-send table once per buffer (wait-free; the engine
    // thread republishes it on every send add/remove/clear).
    let aux_guard = shared.aux_sends.load();
    let aux_ref: &[AuxSend] = &aux_guard;

    let any_solo = any_top_level_solo(tracks_guard.values());

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
            aux_ref,
            playhead,
            head_frames,
            track_buf_l,
            track_buf_r,
            bus_bufs,
            port_scratch,
            note_event_buf,
            midi_stash,
            &monitor_temp[..head_monitor_frames * frame_stride],
            head_monitor_frames,
            input_channels,
            transport_snap,
            comp_ref,
        );

        // Flush instrument voices at the seam.
        panic_instrument_tracks(&tracks_guard, &plugins_guard, midi_stash);

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
            aux_ref,
            loop_in,
            tail_frames,
            track_buf_l,
            track_buf_r,
            bus_bufs,
            port_scratch,
            note_event_buf,
            midi_stash,
            &monitor_temp
                [tail_monitor_start..tail_monitor_start + tail_monitor_frames * frame_stride],
            tail_monitor_frames,
            input_channels,
            transport_snap,
            comp_ref,
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
            aux_ref,
            playhead,
            frames,
            track_buf_l,
            track_buf_r,
            bus_bufs,
            port_scratch,
            note_event_buf,
            midi_stash,
            &monitor_temp[..monitor_frames * frame_stride],
            monitor_frames,
            input_channels,
            transport_snap,
            comp_ref,
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
    if tempo_snap.metronome {
        render_metronome_clicks(
            data,
            channels,
            sample_rate,
            tm_ref,
            snap_bpm,
            tempo_snap.num,
            output_frames,
            playhead,
            seam_split,
        );
    }

    // Apply master volume, hard clip, and compute master peak levels
    apply_master_volume_and_peaks(data, channels, shared);

    shared.playhead.store(new_playhead, Ordering::Relaxed);
    } // 'arrangement

    // Audition preview overlay: summed in after the arrangement + master pass,
    // independent of transport, so a sample audition is audible whether or not
    // the project is rolling. Bypasses the master fader/FX by design — it's a
    // monitor-style preview, not part of the mix.
    mix_audition_overlay(data, channels, shared);
}
