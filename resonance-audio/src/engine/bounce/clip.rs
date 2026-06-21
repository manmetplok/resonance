//! Bounce one instrument track (+ sub-tracks) to an in-RAM `AudioClip`.
//!
//! Excludes master FX and master volume because the bounced clip will
//! play back through master on the next playback (which would otherwise
//! double those processors).

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::Sender;
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::clap_host::SyncClapInstance;
use crate::types::*;

use super::super::bounce_common::midi_render_range;
use super::super::SharedState;
use super::render::{
    build_latency_comp, render_chunk, reset_plugins, ChunkCtx, ChunkScratch, BOUNCE_CHUNK,
};

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
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
    event_tx: &Sender<AudioEvent>,
) {
    // Same guard as the realtime bounce path: the offline renderer
    // shares plugin instances with the live mixer, so rendering while
    // the transport rolls would interleave process() calls (and the
    // reset below) with live playback, corrupting both outputs.
    if shared.playing.load(Ordering::Relaxed) {
        let _ = event_tx.send(AudioEvent::TrackBounceError(
            "Stop transport before bouncing".into(),
        ));
        return;
    }

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

    // Compute render range. If the user drew a punch-in/out loop the
    // loop range wins; otherwise we fall back to the source track's
    // MIDI extent + 2 s tail.
    let (render_start, render_end) = match midi_render_range(
        midi_clips,
        tempo_map,
        shared,
        source_track_id,
        sample_rate,
    ) {
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

    // Clear any stale cancel flag from a previous run before we start
    // — the same atomic gates this offline render and serves as the
    // realtime path's cancel signal, so a leftover `true` would abort
    // the render before its first chunk.
    shared.bounce_cancel.store(false, Ordering::Relaxed);

    reset_plugins(plugins);

    let bounce_tm = (**tempo_map.load()).clone();
    let master_vol = f32::from_bits(shared.master_volume_bits.load(Ordering::Relaxed));
    let latency_comp = build_latency_comp(tracks, busses, plugins);
    // Render `max_latency` extra frames and drop the same number from
    // the front: plugin-delay compensation shifts every contributing
    // track by the pipeline latency, so trimming it gives the bounced
    // clip zero net shift — it plays back (through an empty chain that
    // then gets the full live compensation delay) exactly where the
    // source track sounded.
    let comp_latency = latency_comp.max_latency();
    let render_stop = render_end + comp_latency;
    let mut skip_frames = comp_latency as usize;
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
        latency_comp: &latency_comp,
    };
    let mut scratch = ChunkScratch::new();

    let total_frames = (render_end - render_start) as usize;
    // Stereo interleaved output buffer for the whole bounce.
    let mut output = vec![0.0f32; total_frames * 2];

    // Send a 0% progress event up front so the UI shows the modal
    // populated even before the first chunk completes (which on a long
    // bounce could be several hundred ms in).
    let _ = event_tx.send(AudioEvent::BounceProgress { fraction: 0.0 });

    let in_filter = move |id: TrackId| filter_set.contains(&id);
    let mut pos = render_start;
    let mut written: usize = 0;
    let mut last_emitted_pct: i32 = 0;
    while pos < render_stop {
        if shared.bounce_cancel.load(Ordering::Relaxed) {
            // Cooperative cancel: tear down the half-rendered target
            // track + clip allocation and report back. The clip wasn't
            // pushed yet (we only push at the very end), so we just
            // need to remove the freshly-added empty target track.
            shared.bounce_cancel.store(false, Ordering::Relaxed);
            let _ = tracks.write().shift_remove(&target_track_id);
            let _ = event_tx.send(AudioEvent::TrackRemoved {
                track_id: target_track_id,
            });
            let _ = event_tx
                .send(AudioEvent::TrackBounceCancelled { target_track_id });
            return;
        }

        let frames = ((render_stop - pos) as usize).min(BOUNCE_CHUNK);
        render_chunk(&ctx, &mut scratch, pos, frames, &in_filter, false, false);
        let drop_now = skip_frames.min(frames);
        skip_frames -= drop_now;
        let copy = (frames - drop_now).min(total_frames - written);
        output[written * 2..(written + copy) * 2]
            .copy_from_slice(&scratch.mix_buf[drop_now * 2..(drop_now + copy) * 2]);
        written += copy;
        pos += frames as u64;

        // Emit progress at most once per integer percent so we don't
        // flood the GUI event channel on a long bounce.
        let pct = (((pos - render_start) as f32 / (render_stop - render_start) as f32) * 100.0)
            as i32;
        if pct > last_emitted_pct {
            last_emitted_pct = pct;
            let _ = event_tx.send(AudioEvent::BounceProgress {
                fraction: pct as f32 / 100.0,
            });
        }
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
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        vocal_tuning: None,
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
