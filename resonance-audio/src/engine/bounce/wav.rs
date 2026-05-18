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
use super::render::{render_chunk, reset_plugins, ChunkCtx, ChunkScratch, BOUNCE_CHUNK};

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
    // Compute project range from audio clips + MIDI clips.
    let (render_start, render_end) = {
        let clips_guard = clips.read();
        let midi_guard = midi_clips.read();
        let tm = tempo_map.load();
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

    let bounce_tm = (**tempo_map.load()).clone();
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
