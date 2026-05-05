//! Offline bounce renderers. Two entry points share one chunked render
//! core (`render_chunk`):
//!
//! * [`to_wav`] — render the whole project to a 32-bit float stereo WAV
//!   file. Includes master FX + master volume + hard-clip so the file
//!   plays back identically outside the app.
//!
//! * [`to_audio_clip`] — render a single instrument track (and any of its
//!   sub-tracks) to an in-RAM stereo buffer, then push it as a fresh
//!   [`AudioClip`] on a target track. Excludes master FX / master volume
//!   / hard-clip because the audio will play through master on the next
//!   playback (which would otherwise apply master FX twice). Used by the
//!   "bounce in place" workflow for internal-synth instrument tracks.
//!
//! Both render loops mirror live playback: per-track plugin chain,
//! per-bus plugin chain and routing. They reset every plugin once at the
//! start so plugin internal state is deterministic.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::Sender;
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::clap_host::SyncClapInstance;
use crate::mixer;
use crate::types::*;

use super::bounce_common::midi_render_range;
use super::{SharedState, MAX_BUSSES};

const BOUNCE_CHUNK: usize = 1024;

/// Mutable scratch buffers reused across chunks. Allocated once by the
/// caller and lent to [`render_chunk`].
struct ChunkScratch {
    track_buf_l: Vec<f32>,
    track_buf_r: Vec<f32>,
    bus_bufs: Vec<(Vec<f32>, Vec<f32>)>,
    note_buf: Vec<PendingNoteEvent>,
    mix_buf: Vec<f32>,
}

impl ChunkScratch {
    fn new() -> Self {
        Self {
            track_buf_l: vec![0.0f32; BOUNCE_CHUNK],
            track_buf_r: vec![0.0f32; BOUNCE_CHUNK],
            bus_bufs: (0..MAX_BUSSES)
                .map(|_| (vec![0.0f32; BOUNCE_CHUNK], vec![0.0f32; BOUNCE_CHUNK]))
                .collect(),
            note_buf: Vec::with_capacity(256),
            mix_buf: vec![0.0f32; BOUNCE_CHUNK * 2],
        }
    }
}

/// Read-only context shared by every chunk in a bounce run. Holds
/// references to the engine's locked state so the render loop can
/// re-acquire each lock per chunk (matching live playback's contention
/// pattern).
struct ChunkCtx<'a> {
    shared: &'a Arc<SharedState>,
    tracks: &'a Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &'a Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: &'a Arc<RwLock<MasterBus>>,
    clips: &'a Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: &'a Arc<RwLock<Vec<MidiClip>>>,
    plugins: &'a Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: &'a TempoMap,
    sample_rate: u32,
    master_vol: f32,
}

/// Reset every plugin so the bounce starts from a clean state. Without
/// this, leftover envelope phase / reverb tail / etc. from previous
/// playback would bleed into the first frame.
fn reset_plugins(plugins: &Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>) {
    let plugins_guard = plugins.read();
    for mutex in plugins_guard.values() {
        let mut inst = mutex.lock();
        inst.0.reset_processing();
    }
}

/// Render one chunk into `scratch.mix_buf`. The output is interleaved
/// stereo of length `frames * 2`. When `include_master_fx` is true,
/// master FX, master volume and hard-clip are applied; otherwise the
/// raw bus-summed mix is left for the caller (used by bounce-in-place
/// so master FX aren't applied twice on playback).
///
/// The closure `in_filter` decides which tracks contribute. Any track
/// for which the closure returns false is skipped exactly like a muted
/// one — but its bus isn't drained either, so reverb tails on shared
/// buses still come from the in-filter tracks only.
fn render_chunk(
    ctx: &ChunkCtx<'_>,
    scratch: &mut ChunkScratch,
    pos: u64,
    frames: usize,
    in_filter: &dyn Fn(TrackId) -> bool,
    include_master_fx: bool,
    respect_mute_solo: bool,
) {
    scratch.mix_buf[..frames * 2].fill(0.0);

    let tracks_guard = ctx.tracks.read();
    let busses_guard = ctx.busses.read();
    let clips_guard = ctx.clips.read();
    let midi_guard = ctx.midi_clips.read();
    let plugins_guard = ctx.plugins.read();

    let active_busses = busses_guard.len().min(scratch.bus_bufs.len());
    for (bl, br) in scratch.bus_bufs.iter_mut().take(active_busses) {
        bl[..frames].fill(0.0);
        br[..frames].fill(0.0);
    }

    let any_solo = tracks_guard.values().any(|t| t.soloed());

    for track in tracks_guard.values() {
        // For `to_wav` we honour the user's mix (muted/non-soloed tracks
        // drop out). For `to_audio_clip` (bounce-in-place) `in_filter`
        // already gates to the source + sub-tracks — and the source is
        // explicitly muted by `finalize_bounce` after every successful
        // bounce, so respecting `muted` here would silence every
        // re-bounce of the same track.
        if respect_mute_solo {
            if track.muted() {
                continue;
            }
            if any_solo && !track.soloed() {
                continue;
            }
        }
        if !in_filter(track.id) {
            continue;
        }

        scratch.track_buf_l[..frames].fill(0.0);
        scratch.track_buf_r[..frames].fill(0.0);
        let mut has_audio = false;

        if track.track_type == TrackType::Instrument {
            // Instrument track: collect MIDI events and process.
            scratch.note_buf.clear();
            mixer::collect_midi_events_bounce(
                &midi_guard,
                track.id,
                pos,
                frames,
                ctx.tempo_map,
                ctx.sample_rate,
                &mut scratch.note_buf,
            );
            let mut plugin_iter = track.plugin_ids.iter();
            if let Some(&inst_id) = plugin_iter.next() {
                if let Some(mutex) = plugins_guard.get(&inst_id) {
                    let mut inst = mutex.lock();
                    for ev in scratch.note_buf.iter() {
                        if ev.is_note_on {
                            inst.0.queue_note_on(ev.note, ev.velocity, ev.sample_offset);
                        } else {
                            inst.0.queue_note_off(ev.note, ev.sample_offset);
                        }
                    }
                    inst.0.process(
                        &mut scratch.track_buf_l[..frames],
                        &mut scratch.track_buf_r[..frames],
                        frames,
                    );
                    has_audio = true;
                }
            }
            if !track.fx_bypassed() {
                for &plugin_id in plugin_iter {
                    if let Some(mutex) = plugins_guard.get(&plugin_id) {
                        let mut inst = mutex.lock();
                        inst.0.process(
                            &mut scratch.track_buf_l[..frames],
                            &mut scratch.track_buf_r[..frames],
                            frames,
                        );
                        has_audio = true;
                    }
                }
            }
        } else {
            // Audio track: mix clips + plugin chain.
            for clip in clips_guard.iter() {
                if clip.track_id != track.id {
                    continue;
                }
                let clip_start = clip.start_sample;
                let clip_end = clip_start + clip.duration_frames();
                let buf_end = pos + frames as u64;
                if buf_end <= clip_start || pos >= clip_end {
                    continue;
                }
                let overlap_start = pos.max(clip_start);
                let overlap_end = buf_end.min(clip_end);
                let clip_data = clip.source.as_frames();
                for timeline_frame in overlap_start..overlap_end {
                    let frame_offset = (timeline_frame - pos) as usize;
                    let clip_frame = (timeline_frame - clip_start) as usize
                        + clip.trim_start_frames as usize;
                    let clip_idx = clip_frame * 2;
                    if clip_idx + 1 < clip_data.len() {
                        scratch.track_buf_l[frame_offset] += clip_data[clip_idx];
                        scratch.track_buf_r[frame_offset] += clip_data[clip_idx + 1];
                        has_audio = true;
                    }
                }
            }

            // Process through plugin chain (skipped when bypassed).
            if !track.plugin_ids.is_empty() && !track.fx_bypassed() {
                for &plugin_id in &track.plugin_ids {
                    if let Some(mutex) = plugins_guard.get(&plugin_id) {
                        let mut inst = mutex.lock();
                        inst.0.process(
                            &mut scratch.track_buf_l[..frames],
                            &mut scratch.track_buf_r[..frames],
                            frames,
                        );
                        has_audio = true;
                    }
                }
            }
        }

        if !has_audio {
            continue;
        }

        // Apply track volume + pan, route to master or bus.
        let volume = track.volume();
        let (pan_l, pan_r) = resonance_dsp::constant_power_pan(track.pan());
        let gain_l = volume * pan_l;
        let gain_r = volume * pan_r;

        let routed_to_bus = match track.output() {
            TrackOutput::Bus(bus_id) => busses_guard
                .get_index_of(&bus_id)
                .filter(|idx| *idx < active_busses)
                .map(|idx| {
                    let (bl, br) = &mut scratch.bus_bufs[idx];
                    for f in 0..frames {
                        bl[f] += scratch.track_buf_l[f] * gain_l;
                        br[f] += scratch.track_buf_r[f] * gain_r;
                    }
                })
                .is_some(),
            TrackOutput::Master => false,
        };
        if !routed_to_bus {
            for f in 0..frames {
                scratch.mix_buf[f * 2] += scratch.track_buf_l[f] * gain_l;
                scratch.mix_buf[f * 2 + 1] += scratch.track_buf_r[f] * gain_r;
            }
        }
    }

    // Per-bus plugin chain + volume/pan + sum to master.
    for (bus_idx, bus) in busses_guard.values().enumerate().take(active_busses) {
        if bus.muted() {
            continue;
        }
        let (bl, br) = &mut scratch.bus_bufs[bus_idx];
        if !bus.fx_bypassed() {
            for &plugin_id in &bus.plugin_ids {
                if let Some(mutex) = plugins_guard.get(&plugin_id) {
                    let mut inst = mutex.lock();
                    inst.0.process(&mut bl[..frames], &mut br[..frames], frames);
                }
            }
        }
        let bus_volume = bus.volume();
        let (bus_pan_l, bus_pan_r) = resonance_dsp::constant_power_pan(bus.pan());
        let bus_gain_l = bus_volume * bus_pan_l;
        let bus_gain_r = bus_volume * bus_pan_r;
        for f in 0..frames {
            scratch.mix_buf[f * 2] += bl[f] * bus_gain_l;
            scratch.mix_buf[f * 2 + 1] += br[f] * bus_gain_r;
        }
    }

    // Master FX chain: run over the summed mix in place. Skipped when
    // the caller asked us to leave the raw bus-summed mix alone (so the
    // master FX won't be applied twice when the result plays back).
    if include_master_fx && !ctx.shared.master_fx_bypassed.load(Ordering::Relaxed) {
        let master_guard = ctx.master.read();
        if !master_guard.plugin_ids.is_empty() {
            for f in 0..frames {
                scratch.track_buf_l[f] = scratch.mix_buf[f * 2];
                scratch.track_buf_r[f] = scratch.mix_buf[f * 2 + 1];
            }
            for &plugin_id in &master_guard.plugin_ids {
                if let Some(mutex) = plugins_guard.get(&plugin_id) {
                    let mut inst = mutex.lock();
                    inst.0.process(
                        &mut scratch.track_buf_l[..frames],
                        &mut scratch.track_buf_r[..frames],
                        frames,
                    );
                }
            }
            for f in 0..frames {
                scratch.mix_buf[f * 2] = scratch.track_buf_l[f];
                scratch.mix_buf[f * 2 + 1] = scratch.track_buf_r[f];
            }
        }
    }

    drop(plugins_guard);
    drop(clips_guard);
    drop(busses_guard);
    drop(tracks_guard);

    if include_master_fx {
        for s in &mut scratch.mix_buf[..frames * 2] {
            *s = (*s * ctx.master_vol).clamp(-1.0, 1.0);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn to_wav(
    path: String,
    shared: &Arc<SharedState>,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: &Arc<RwLock<MasterBus>>,
    clips: &Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    plugins: &Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: &Arc<RwLock<TempoMap>>,
    sample_rate: u32,
    event_tx: &Sender<AudioEvent>,
) {
    // Compute project range from audio clips + MIDI clips.
    let (render_start, render_end) = {
        let clips_guard = clips.read();
        let midi_guard = midi_clips.read();
        let tm = tempo_map.read();
        let spt = tm.samples_per_beat(sample_rate) / TICKS_PER_QUARTER_NOTE as f64;

        if clips_guard.is_empty() && midi_guard.is_empty() {
            let _ = event_tx.send(AudioEvent::BounceError("No clips to bounce".into()));
            return;
        }
        let audio_start = clips_guard.iter().map(|c| c.start_sample).min();
        let audio_end = clips_guard.iter().map(|c| c.end_sample()).max();
        let midi_start = midi_guard.iter().map(|c| c.start_sample).min();
        let midi_end = midi_guard.iter().map(|c| c.end_sample(spt)).max();

        let start = audio_start.into_iter().chain(midi_start).min().unwrap_or(0);
        let end = audio_end.into_iter().chain(midi_end).max().unwrap_or(0);
        (start, end)
    };

    if render_end <= render_start {
        let _ = event_tx.send(AudioEvent::BounceError("No audio to bounce".into()));
        return;
    }

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = match hound::WavWriter::create(&path, spec) {
        Ok(w) => w,
        Err(e) => {
            let _ = event_tx.send(AudioEvent::BounceError(format!(
                "Failed to create WAV file: {e}"
            )));
            return;
        }
    };

    reset_plugins(plugins);

    let bounce_tm = tempo_map.read().clone();
    let master_vol = f32::from_bits(shared.master_volume_bits.load(Ordering::Relaxed));
    let ctx = ChunkCtx {
        shared,
        tracks,
        busses,
        master,
        clips,
        midi_clips,
        plugins,
        tempo_map: &bounce_tm,
        sample_rate,
        master_vol,
    };
    let mut scratch = ChunkScratch::new();

    let mut pos = render_start;
    let mut write_error = false;
    let everything = |_: TrackId| true;
    while pos < render_end && !write_error {
        let frames = ((render_end - pos) as usize).min(BOUNCE_CHUNK);
        render_chunk(&ctx, &mut scratch, pos, frames, &everything, true, true);

        for &sample in &scratch.mix_buf[..frames * 2] {
            if let Err(e) = writer.write_sample(sample) {
                let _ = event_tx.send(AudioEvent::BounceError(format!("WAV write error: {e}")));
                write_error = true;
                break;
            }
        }

        pos += frames as u64;
    }

    if !write_error {
        match writer.finalize() {
            Ok(()) => {
                let _ = event_tx.send(AudioEvent::BounceComplete { path });
            }
            Err(e) => {
                let _ = event_tx.send(AudioEvent::BounceError(format!("WAV finalize error: {e}")));
            }
        }
    }
}

/// Bounce one instrument track (and any of its sub-tracks) to a single
/// in-RAM stereo `AudioClip` on `target_track_id`. Excludes master FX
/// and master volume so the bounced clip plays back through master once
/// without doubling those processors.
///
/// The render range is `[earliest MIDI start, latest MIDI end + 2 s]`
/// on the source track. The 2 s tail catches FX / bus reverb decay.
///
/// Public so integration tests can drive the renderer directly without
/// going through the full engine command path; production callers
/// route through [`AudioCommand::BounceTrackToAudio`].
#[allow(clippy::too_many_arguments)]
pub fn to_audio_clip(
    source_track_id: TrackId,
    target_track_id: TrackId,
    target_clip_id: ClipId,
    name: String,
    shared: &Arc<SharedState>,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: &Arc<RwLock<MasterBus>>,
    clips: &Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: &Arc<RwLock<Vec<MidiClip>>>,
    plugins: &Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: &Arc<RwLock<TempoMap>>,
    sample_rate: u32,
    event_tx: &Sender<AudioEvent>,
) {
    // Resolve source + sub-tracks (multi-output instruments like
    // resonance-drums spawn sibling tracks fed by parent output ports).
    let filter_set: HashSet<TrackId> = {
        let tracks_guard = tracks.read();
        if !tracks_guard.contains_key(&source_track_id) {
            let _ = event_tx.send(AudioEvent::TrackBounceError(format!(
                "Source track {source_track_id} not found"
            )));
            return;
        }
        if !tracks_guard.contains_key(&target_track_id) {
            let _ = event_tx.send(AudioEvent::TrackBounceError(format!(
                "Target track {target_track_id} not found"
            )));
            return;
        }
        let mut set = HashSet::new();
        set.insert(source_track_id);
        for t in tracks_guard.values() {
            if let Some((parent, _)) = t.sub_track_of {
                if parent == source_track_id {
                    set.insert(t.id);
                }
            }
        }
        set
    };

    // Compute render range from MIDI clips on the source track only —
    // sub-tracks have no clips of their own. Add a fixed tail past the
    // last MIDI clip end for FX / bus reverb decay.
    let (render_start, render_end) =
        match midi_render_range(midi_clips, tempo_map, source_track_id, sample_rate) {
            Ok(range) => range,
            Err(msg) => {
                let _ = event_tx.send(AudioEvent::TrackBounceError(msg.into()));
                return;
            }
        };

    if render_end <= render_start {
        let _ = event_tx.send(AudioEvent::TrackBounceError("Empty render range".into()));
        return;
    }

    reset_plugins(plugins);

    let bounce_tm = tempo_map.read().clone();
    let master_vol = f32::from_bits(shared.master_volume_bits.load(Ordering::Relaxed));
    let ctx = ChunkCtx {
        shared,
        tracks,
        busses,
        master,
        clips,
        midi_clips,
        plugins,
        tempo_map: &bounce_tm,
        sample_rate,
        master_vol,
    };
    let mut scratch = ChunkScratch::new();

    let total_frames = (render_end - render_start) as usize;
    // Stereo interleaved output buffer for the whole bounce.
    let mut output = vec![0.0f32; total_frames * 2];

    let in_filter = move |id: TrackId| filter_set.contains(&id);
    let mut pos = render_start;
    while pos < render_end {
        let frames = ((render_end - pos) as usize).min(BOUNCE_CHUNK);
        render_chunk(&ctx, &mut scratch, pos, frames, &in_filter, false, false);
        let dst_start = ((pos - render_start) as usize) * 2;
        let src = &scratch.mix_buf[..frames * 2];
        output[dst_start..dst_start + frames * 2].copy_from_slice(src);
        pos += frames as u64;
    }

    let waveform_peaks = compute_waveform_peaks(&output);
    let duration_samples = total_frames as u64;

    // Build the AudioClip in-RAM; SaveClipsToProjectDir will transcode
    // it to disk on the next project save (same flow as imported clips).
    let clip = AudioClip {
        id: target_clip_id,
        track_id: target_track_id,
        start_sample: render_start,
        source: ClipSource::Memory(output),
        name: name.clone(),
        trim_start_frames: 0,
        trim_end_frames: 0,
    };
    clips.write().push(clip);

    let _ = event_tx.send(AudioEvent::TrackBounceCompleted {
        source_track_id,
        target_track_id,
        clip: Some(BouncedClipData {
            clip_id: target_clip_id,
            start_sample: render_start,
            duration_samples,
            name,
            waveform_peaks,
        }),
    });
}
