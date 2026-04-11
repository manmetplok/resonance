//! Offline bounce-to-WAV renderer. Reads the full project state (tracks,
//! busses, clips, MIDI clips, plugins, tempo map) under locks held for
//! each chunk and writes a 32-bit float stereo WAV file. The render loop
//! mirrors live playback: per-track plugin chain, per-bus plugin chain
//! and routing, master volume, hard clip. Pure read — no shared state is
//! mutated.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::Sender;
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::clap_host::SyncClapInstance;
use crate::mixer;
use crate::types::*;

use super::{SharedState, MAX_BUSSES};

#[allow(clippy::too_many_arguments)]
pub(crate) fn to_wav(
    path: String,
    shared: &Arc<SharedState>,
    tracks: &Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: &Arc<RwLock<IndexMap<BusId, Bus>>>,
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

    // Open WAV writer
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

    // Reset all plugins for clean render
    {
        let plugins_guard = plugins.read();
        for mutex in plugins_guard.values() {
            let mut inst = mutex.lock();
            inst.0.reset_processing();
        }
    }

    // Offline render in chunks
    const BOUNCE_CHUNK: usize = 1024;
    let mut track_buf_l = vec![0.0f32; BOUNCE_CHUNK];
    let mut track_buf_r = vec![0.0f32; BOUNCE_CHUNK];
    // Bounce mirrors live playback: pre-allocate one stereo buffer per
    // potential bus so bus routing survives the offline render.
    let mut bounce_bus_bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..MAX_BUSSES)
        .map(|_| (vec![0.0f32; BOUNCE_CHUNK], vec![0.0f32; BOUNCE_CHUNK]))
        .collect();
    let mut bounce_note_buf: Vec<PendingNoteEvent> = Vec::with_capacity(256);
    let mut mix_buf = vec![0.0f32; BOUNCE_CHUNK * 2];
    let master_vol = f32::from_bits(shared.master_volume_bits.load(Ordering::Relaxed));
    let mut write_error = false;

    let bounce_spt = {
        let tm = tempo_map.read();
        tm.samples_per_beat(sample_rate) / TICKS_PER_QUARTER_NOTE as f64
    };

    let mut pos = render_start;
    while pos < render_end && !write_error {
        let frames = ((render_end - pos) as usize).min(BOUNCE_CHUNK);
        mix_buf[..frames * 2].fill(0.0);

        let tracks_guard = tracks.read();
        let busses_guard = busses.read();
        let clips_guard = clips.read();
        let midi_guard = midi_clips.read();
        let plugins_guard = plugins.read();

        let active_busses = busses_guard.len().min(bounce_bus_bufs.len());
        for (bl, br) in bounce_bus_bufs.iter_mut().take(active_busses) {
            bl[..frames].fill(0.0);
            br[..frames].fill(0.0);
        }

        let any_solo = tracks_guard.values().any(|t| t.soloed());

        for track in tracks_guard.values() {
            if track.muted() {
                continue;
            }
            if any_solo && !track.soloed() {
                continue;
            }

            track_buf_l[..frames].fill(0.0);
            track_buf_r[..frames].fill(0.0);
            let mut has_audio = false;

            if track.track_type == TrackType::Instrument {
                // Instrument track: collect MIDI events and process.
                bounce_note_buf.clear();
                mixer::collect_midi_events_bounce(
                    &midi_guard,
                    track.id,
                    pos,
                    frames,
                    bounce_spt,
                    &mut bounce_note_buf,
                );
                let mut plugin_iter = track.plugin_ids.iter();
                if let Some(&inst_id) = plugin_iter.next() {
                    if let Some(mutex) = plugins_guard.get(&inst_id) {
                        let mut inst = mutex.lock();
                        for ev in bounce_note_buf.iter() {
                            if ev.is_note_on {
                                inst.0.queue_note_on(ev.note, ev.velocity, ev.sample_offset);
                            } else {
                                inst.0.queue_note_off(ev.note, ev.sample_offset);
                            }
                        }
                        inst.0.process(
                            &mut track_buf_l[..frames],
                            &mut track_buf_r[..frames],
                            frames,
                        );
                        has_audio = true;
                    }
                }
                for &plugin_id in plugin_iter {
                    if let Some(mutex) = plugins_guard.get(&plugin_id) {
                        let mut inst = mutex.lock();
                        inst.0.process(
                            &mut track_buf_l[..frames],
                            &mut track_buf_r[..frames],
                            frames,
                        );
                        has_audio = true;
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
                            track_buf_l[frame_offset] += clip_data[clip_idx];
                            track_buf_r[frame_offset] += clip_data[clip_idx + 1];
                            has_audio = true;
                        }
                    }
                }

                // Process through plugin chain.
                if !track.plugin_ids.is_empty() {
                    for &plugin_id in &track.plugin_ids {
                        if let Some(mutex) = plugins_guard.get(&plugin_id) {
                            let mut inst = mutex.lock();
                            inst.0.process(
                                &mut track_buf_l[..frames],
                                &mut track_buf_r[..frames],
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
                        let (bl, br) = &mut bounce_bus_bufs[idx];
                        for f in 0..frames {
                            bl[f] += track_buf_l[f] * gain_l;
                            br[f] += track_buf_r[f] * gain_r;
                        }
                    })
                    .is_some(),
                TrackOutput::Master => false,
            };
            if !routed_to_bus {
                for f in 0..frames {
                    mix_buf[f * 2] += track_buf_l[f] * gain_l;
                    mix_buf[f * 2 + 1] += track_buf_r[f] * gain_r;
                }
            }
        }

        // Per-bus plugin chain + volume/pan + sum to master.
        for (bus_idx, bus) in busses_guard.values().enumerate().take(active_busses) {
            if bus.muted() {
                continue;
            }
            let (bl, br) = &mut bounce_bus_bufs[bus_idx];
            for &plugin_id in &bus.plugin_ids {
                if let Some(mutex) = plugins_guard.get(&plugin_id) {
                    let mut inst = mutex.lock();
                    inst.0.process(&mut bl[..frames], &mut br[..frames], frames);
                }
            }
            let bus_volume = bus.volume();
            let (bus_pan_l, bus_pan_r) = resonance_dsp::constant_power_pan(bus.pan());
            let bus_gain_l = bus_volume * bus_pan_l;
            let bus_gain_r = bus_volume * bus_pan_r;
            for f in 0..frames {
                mix_buf[f * 2] += bl[f] * bus_gain_l;
                mix_buf[f * 2 + 1] += br[f] * bus_gain_r;
            }
        }

        drop(plugins_guard);
        drop(clips_guard);
        drop(busses_guard);
        drop(tracks_guard);

        // Apply master volume and hard clip.
        for s in &mut mix_buf[..frames * 2] {
            *s = (*s * master_vol).clamp(-1.0, 1.0);
        }

        // Write to WAV.
        for &sample in &mix_buf[..frames * 2] {
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
                let _ = event_tx.send(AudioEvent::BounceError(format!(
                    "WAV finalize error: {e}"
                )));
            }
        }
    }
}
