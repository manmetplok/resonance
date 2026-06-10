//! Render the whole project to a 32-bit float stereo WAV file.
//!
//! Includes master FX + master volume + hard-clip so the file plays
//! back identically outside the app.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::Sender;
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use crate::clap_host::SyncClapInstance;
use crate::types::*;

use super::super::SharedState;
use super::render::{
    build_latency_comp, render_chunk, reset_plugins, ChunkCtx, ChunkScratch, BOUNCE_CHUNK,
};

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
    tempo_map: &Arc<arc_swap::ArcSwap<TempoMap>>,
    sample_rate: u32,
    event_tx: &Sender<AudioEvent>,
) {
    // Same guard as the realtime bounce path: the offline renderer
    // shares plugin instances with the live mixer, so rendering while
    // the transport rolls would interleave process() calls (and the
    // reset below) with live playback, corrupting both outputs.
    if shared.playing.load(Ordering::Relaxed) {
        let _ = event_tx.send(AudioEvent::BounceError(
            "Stop transport before bouncing".into(),
        ));
        return;
    }

    // Compute project range from audio clips + MIDI clips.
    let (render_start, render_end) = {
        let clips_guard = clips.read();
        let midi_guard = midi_clips.read();
        let tm = tempo_map.load();

        if clips_guard.is_empty() && midi_guard.is_empty() {
            let _ = event_tx.send(AudioEvent::BounceError("No clips to bounce".into()));
            return;
        }
        let audio_start = clips_guard.iter().map(|c| c.start_sample).min();
        let audio_end = clips_guard.iter().map(|c| c.end_sample()).max();
        let midi_start = midi_guard.iter().map(|c| c.start_sample).min();
        // Tempo-aware end to match the renderer's tick_to_abs_sample
        // note scheduling under tempo changes.
        let midi_end = midi_guard
            .iter()
            .map(|c| tm.tick_to_abs_sample(c.start_sample, c.visible_duration_ticks(), sample_rate))
            .max();

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

    let bounce_tm = (**tempo_map.load()).clone();
    let master_vol = f32::from_bits(shared.master_volume_bits.load(Ordering::Relaxed));
    let latency_comp = build_latency_comp(tracks, busses, plugins);
    // Render `max_latency` extra frames and drop the same number from
    // the front: plugin-delay compensation shifts every track by the
    // pipeline latency, so trimming it re-aligns the file with the
    // timeline (and the extra tail catches the delayed final samples).
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

    let mut pos = render_start;
    let mut write_error = false;
    let mut cancelled = false;
    let everything = |_: TrackId| true;
    while pos < render_stop && !write_error {
        // Cooperative cancel — `AudioCommand::CancelBounce` flips this
        // flag from the engine thread. Checked once per chunk so the
        // UI's modal Cancel button releases the bounce promptly
        // (chunks are ~tens of ms each).
        if shared.bounce_cancel.load(Ordering::Relaxed) {
            cancelled = true;
            break;
        }
        let frames = ((render_stop - pos) as usize).min(BOUNCE_CHUNK);
        render_chunk(&ctx, &mut scratch, pos, frames, &everything, true, true);

        let drop_now = skip_frames.min(frames);
        skip_frames -= drop_now;
        for &sample in &scratch.mix_buf[drop_now * 2..frames * 2] {
            if let Err(e) = writer.write_sample(sample) {
                let _ = event_tx.send(AudioEvent::BounceError(format!("WAV write error: {e}")));
                write_error = true;
                break;
            }
        }

        pos += frames as u64;
    }

    if cancelled {
        // Drop the partial WAV — we don't want a half-rendered file
        // sitting next to its expected output. Reset the cancel flag
        // so the next bounce starts fresh.
        drop(writer);
        let _ = std::fs::remove_file(&path);
        shared.bounce_cancel.store(false, Ordering::Relaxed);
        let _ = event_tx.send(AudioEvent::BounceError("Bounce cancelled".into()));
    } else if !write_error {
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
