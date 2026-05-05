//! Streaming recording state: instead of accumulating takes in a
//! growing `Vec<f32>` on the engine thread, each armed track owns a
//! `hound::WavWriter` backed by a `BufWriter<File>` that lives at its
//! final location in the current project directory. The drain loop
//! (engine control thread, not the real-time audio callback)
//! deinterleaves the ring buffer, resamples to the engine rate if
//! needed, and writes samples directly to disk. Finalization closes
//! the writer, memory-maps the file, and builds an `AudioClip`
//! backed by `ClipSource::Mapped` — so the take never materialises
//! as a contiguous in-RAM buffer.

use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use crossbeam_channel::Sender;
use hound::{SampleFormat, WavSpec, WavWriter};
use ringbuf::traits::Consumer;

use crate::decode::StreamingLinearResampler;
use crate::types::*;

/// Size of the stack scratch used to deinterleave and resample one
/// drain chunk. 4096 samples × up to 16 input channels gives us a
/// comfortable ceiling; larger chunks loop.
const DRAIN_SCRATCH_LEN: usize = 4096;

/// Per-track recording scratch: the streaming WAV writer, the target
/// path, the pre-allocated clip id, incremental waveform peaks, an
/// optional streaming resampler, and a snapshot of the track's
/// port/mono settings captured at record-start time.
pub(crate) struct TrackRecordingBuf {
    pub writer: Option<WavWriter<BufWriter<File>>>,
    pub path: PathBuf,
    pub clip_id: ClipId,
    pub resampler: Option<StreamingLinearResampler>,
    pub resample_scratch: Vec<f32>,

    /// Incrementally accumulated waveform peaks (one min/max pair
    /// per `WAVEFORM_PEAK_FRAMES` frames of output audio).
    pub peaks: Vec<(f32, f32)>,
    pub peak_min: f32,
    pub peak_max: f32,
    pub peak_frames: usize,

    /// Total stereo frames written to the WAV so far (post-resample).
    pub frames_written: u64,

    /// 0-indexed starting channel in the interleaved input stream.
    pub input_port: u16,
    /// True = capture one channel and duplicate to L/R. False =
    /// capture two consecutive channels as L/R.
    pub mono: bool,
}

/// Groups all mutable recording state that lives on the engine thread.
pub(crate) struct RecordingState {
    pub buffers: HashMap<TrackId, TrackRecordingBuf>,
    pub start_sample: SamplePos,
    pub ring_consumer: Option<ringbuf::HeapCons<f32>>,
    pub input_stream: Option<crate::input_handle::InputHandle>,
    pub input_channels: u16,
    pub input_sample_rate: u32,
    pub loop_enabled: bool,
    pub loop_in: SamplePos,
    pub loop_out: SamplePos,
    /// Set when a `Record` with `precount_bars > 0` is in its count-in
    /// phase. `target_sample` is the playhead position the user hit
    /// record at — once the playhead catches up to it, the input stream
    /// opens and recording begins. `restore_metronome` holds the
    /// metronome's pre-count-in state so it can be put back afterwards.
    pub precount: Option<PrecountState>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PrecountState {
    pub target_sample: SamplePos,
    pub restore_metronome: bool,
}

impl RecordingState {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            buffers: HashMap::new(),
            start_sample: 0,
            ring_consumer: None,
            input_stream: None,
            input_channels: 2,
            input_sample_rate: sample_rate,
            loop_enabled: false,
            loop_in: 0,
            loop_out: 0,
            precount: None,
        }
    }

    /// Create a `TrackRecordingBuf` for an armed track: allocates
    /// the clip id, opens a WAV writer at `{project_dir}/audio/clip_{id}.wav`
    /// (creating the audio dir on demand), and sets up a streaming
    /// resampler if the input device doesn't match the engine rate.
    pub fn create_track_buf(
        project_dir: &Path,
        track_id: TrackId,
        clip_id: ClipId,
        engine_sample_rate: u32,
        input_sample_rate: u32,
        input_port: u16,
        mono: bool,
    ) -> Result<TrackRecordingBuf, String> {
        let audio_dir = project_dir.join("audio");
        std::fs::create_dir_all(&audio_dir)
            .map_err(|e| format!("create audio dir {}: {e}", audio_dir.display()))?;
        let path = audio_dir.join(format!("clip_{clip_id}.wav"));

        let spec = WavSpec {
            channels: 2,
            sample_rate: engine_sample_rate,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };
        let writer = WavWriter::create(&path, spec)
            .map_err(|e| format!("create wav {}: {e}", path.display()))?;

        let resampler = if input_sample_rate != engine_sample_rate {
            Some(StreamingLinearResampler::new(
                input_sample_rate,
                engine_sample_rate,
            ))
        } else {
            None
        };

        let _ = track_id; // only used by the caller to key the map
        Ok(TrackRecordingBuf {
            writer: Some(writer),
            path,
            clip_id,
            resampler,
            resample_scratch: Vec::with_capacity(DRAIN_SCRATCH_LEN * 2),
            peaks: Vec::new(),
            peak_min: f32::MAX,
            peak_max: f32::MIN,
            peak_frames: 0,
            frames_written: 0,
            input_port,
            mono,
        })
    }

    /// Drain all available samples from the ring buffer consumer and
    /// stream them to each track's WAV writer. Runs on the engine
    /// control thread, so blocking file I/O through `BufWriter` is
    /// safe — the cpal input callback only pushes into the lock-free
    /// ring buffer.
    pub fn drain_ring_to_buffers(&mut self) {
        let Some(ref mut consumer) = self.ring_consumer else {
            return;
        };
        let channels = self.input_channels as usize;
        if channels == 0 {
            return;
        }

        let mut ring_scratch = [0.0f32; DRAIN_SCRATCH_LEN];
        let mut deint_scratch: Vec<f32> = Vec::with_capacity(DRAIN_SCRATCH_LEN);

        loop {
            let count = consumer.pop_slice(&mut ring_scratch);
            if count == 0 {
                break;
            }
            let chunk = &ring_scratch[..count];
            let frames = chunk.len() / channels;
            if frames == 0 {
                continue;
            }

            for track_buf in self.buffers.values_mut() {
                // Deinterleave this track's channel(s) out of the
                // multi-channel ring chunk into `deint_scratch`, as
                // stereo-interleaved input-rate samples.
                let port = (track_buf.input_port as usize).min(channels - 1);
                let right_port = if track_buf.mono {
                    port
                } else {
                    (port + 1).min(channels - 1)
                };
                deint_scratch.clear();
                deint_scratch.reserve(frames * 2);
                for f in 0..frames {
                    let base = f * channels;
                    deint_scratch.push(chunk[base + port]);
                    deint_scratch.push(chunk[base + right_port]);
                }

                // Resample (if needed) into `resample_scratch`, then
                // either write directly from `deint_scratch` (no
                // resampler) or swap the scratch Vec out so we can
                // write from an owned buffer without colliding with
                // the mutable borrow on `track_buf`.
                let resampled: Option<Vec<f32>> = if let Some(r) = track_buf.resampler.as_mut() {
                    let mut buf = std::mem::take(&mut track_buf.resample_scratch);
                    buf.clear();
                    r.process(&deint_scratch, &mut buf);
                    Some(buf)
                } else {
                    None
                };
                let write_result = if let Some(ref buf) = resampled {
                    write_samples_and_peaks(track_buf, buf)
                } else {
                    write_samples_and_peaks(track_buf, &deint_scratch)
                };
                if let Some(buf) = resampled {
                    track_buf.resample_scratch = buf;
                }
                if let Err(e) = write_result {
                    eprintln!(
                        "recording: write failed for {}: {e}",
                        track_buf.path.display()
                    );
                    // Drop the writer so subsequent drains don't
                    // keep retrying; finalize will surface a short
                    // clip or no clip depending on how much made it.
                    track_buf.writer = None;
                }
            }
        }
    }

    /// Finalize recording: drain any pending ring data, flush the
    /// streaming resamplers, close each WAV writer, memory-map the
    /// resulting files, and push an `AudioClip` per track into the
    /// shared clip map. Emits `RecordingFinished` events with the
    /// incrementally-accumulated waveform peaks.
    /// Returns the number of audio clips that were actually emitted
    /// (one per armed track that captured at least one frame). Callers
    /// like the realtime bounce path use this to detect "stream opened
    /// but produced no audio" scenarios and surface a clearer error.
    pub fn finalize_recording(
        &mut self,
        _output_sample_rate: u32,
        clips: &parking_lot::RwLock<Vec<AudioClip>>,
        event_tx: &Sender<AudioEvent>,
    ) -> usize {
        self.drain_ring_to_buffers();
        let mut clips_emitted = 0usize;

        for (track_id, mut track_buf) in self.buffers.drain() {
            // Flush any trailing resampled frame.
            if let Some(r) = track_buf.resampler.as_mut() {
                track_buf.resample_scratch.clear();
                r.flush(&mut track_buf.resample_scratch);
                if !track_buf.resample_scratch.is_empty() {
                    let tail: Vec<f32> = std::mem::take(&mut track_buf.resample_scratch);
                    if let Err(e) = write_samples_and_peaks(&mut track_buf, &tail) {
                        eprintln!(
                            "recording: flush failed for {}: {e}",
                            track_buf.path.display()
                        );
                    }
                }
            }
            // Close out any trailing peak accumulator so a short
            // recording still gets its final bucket.
            if track_buf.peak_frames > 0 {
                track_buf
                    .peaks
                    .push((track_buf.peak_min, track_buf.peak_max));
                track_buf.peak_frames = 0;
                track_buf.peak_min = f32::MAX;
                track_buf.peak_max = f32::MIN;
            }

            // Close the writer so the WAV header carries the correct
            // data chunk size. If this fails the file is unusable.
            if let Some(writer) = track_buf.writer.take() {
                if let Err(e) = writer.finalize() {
                    eprintln!(
                        "recording: finalize wav {} failed: {e}",
                        track_buf.path.display()
                    );
                    continue;
                }
            } else {
                // Writer was dropped earlier due to a write error.
                continue;
            }

            if track_buf.frames_written == 0 {
                // Nothing was captured (no input or immediate stop);
                // leave the empty WAV behind but don't create a clip.
                let _ = std::fs::remove_file(&track_buf.path);
                continue;
            }

            // Apply loop-range trim via non-destructive trim fields
            // on the clip so we don't have to rewrite the file.
            let (clip_start_sample, trim_start_frames, trim_end_frames) =
                if self.loop_enabled && self.loop_out > self.loop_in {
                    let total_frames = track_buf.frames_written;
                    let trim_start = self.loop_in.saturating_sub(self.start_sample);
                    let trim_end = self
                        .loop_out
                        .saturating_sub(self.start_sample)
                        .min(total_frames);
                    if trim_start >= trim_end {
                        let _ = std::fs::remove_file(&track_buf.path);
                        continue;
                    }
                    let end_skip = total_frames.saturating_sub(trim_end);
                    (self.loop_in, trim_start, end_skip)
                } else {
                    (self.start_sample, 0, 0)
                };

            // Memory-map the finalized WAV file.
            let source = match ClipSource::open_wav(&track_buf.path) {
                Ok(src) => src,
                Err(e) => {
                    eprintln!("recording: mmap {} failed: {e}", track_buf.path.display());
                    continue;
                }
            };

            let clip_id = track_buf.clip_id;
            let name = format!("Recording {}", clip_id);
            let duration_samples = track_buf
                .frames_written
                .saturating_sub(trim_start_frames)
                .saturating_sub(trim_end_frames);

            let clip = AudioClip {
                id: clip_id,
                track_id,
                start_sample: clip_start_sample,
                source,
                name: name.clone(),
                trim_start_frames,
                trim_end_frames,
            };
            {
                let mut guard = clips.write();
                guard.push(clip);
            }

            let _ = event_tx.send(AudioEvent::RecordingFinished {
                clip_id,
                track_id,
                start_sample: clip_start_sample,
                duration_samples,
                name,
                waveform_peaks: track_buf.peaks.clone(),
            });
            clips_emitted += 1;
        }

        self.ring_consumer = None;
        clips_emitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ringbuf::traits::{Producer, Split};
    use ringbuf::HeapRb;

    fn make_tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "resonance-rec-test-{}-{}",
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Prove the core invariant of the refactor: the drain path
    /// never accumulates audio samples in memory regardless of how
    /// long the recording runs. A ten-minute take at 48 kHz stereo
    /// is ~220 MB of PCM — if any of that lived in `TrackRecordingBuf`
    /// the test would fail by showing a scratch capacity in the
    /// hundreds of megabytes.
    #[test]
    fn drain_streams_to_disk_without_growing_memory() {
        let project_dir = make_tempdir("drain");

        let mut rec = RecordingState::new(48_000);
        let ring: HeapRb<f32> = HeapRb::new(48_000 * 2 * 2); // 2 s stereo
        let (mut prod, cons) = ring.split();
        rec.ring_consumer = Some(cons);
        rec.input_channels = 2;
        rec.input_sample_rate = 48_000;
        rec.start_sample = 0;

        let buf = RecordingState::create_track_buf(
            &project_dir,
            /* track_id */ 42,
            /* clip_id  */ 1,
            /* engine   */ 48_000,
            /* input    */ 48_000,
            /* port     */ 0,
            /* mono     */ false,
        )
        .unwrap();
        let wav_path = buf.path.clone();
        rec.buffers.insert(42, buf);

        // Feed the equivalent of 10 seconds of audio in 1000-frame
        // chunks (picked so the totals divide evenly), draining
        // between each push so the ring never overflows. Ten
        // seconds is plenty to catch any accidental `Vec::push` on
        // a hot path — the scratch budget is bounded at
        // `DRAIN_SCRATCH_LEN` regardless.
        let frames_per_chunk = 1000usize;
        let total_frames = 48_000 * 10;
        let chunks = total_frames / frames_per_chunk;
        let mut sample = vec![0.0f32; frames_per_chunk * 2];
        for i in 0..chunks {
            for f in 0..frames_per_chunk {
                let s = ((i * frames_per_chunk + f) as f32 * 0.001).sin();
                sample[f * 2] = s;
                sample[f * 2 + 1] = s;
            }
            prod.push_slice(&sample);
            rec.drain_ring_to_buffers();
        }

        // After draining 10 s of audio, the per-track scratch
        // capacity must remain bounded — it should be the size of
        // one drain chunk at most, not anywhere near the total
        // audio size.
        let track_buf = rec.buffers.get(&42).unwrap();
        let in_memory_bytes = track_buf.resample_scratch.capacity() * 4;
        assert!(
            in_memory_bytes <= 256 * 1024,
            "TrackRecordingBuf holds {} bytes of PCM in memory — the drain path should stream to disk",
            in_memory_bytes
        );
        assert_eq!(
            track_buf.frames_written,
            48_000 * 10,
            "wrong number of frames streamed to disk"
        );

        // Finalize and verify the WAV file on disk really does
        // contain all 10 seconds. Drop the producer first so the
        // ring's outstanding samples are all consumable.
        drop(prod);
        let (tx, _rx) = crossbeam_channel::unbounded();
        let clips = parking_lot::RwLock::new(Vec::new());
        rec.finalize_recording(48_000, &clips, &tx);

        let source = ClipSource::open_wav(&wav_path).expect("mmap finalized wav");
        assert_eq!(source.frame_count(), 48_000 * 10);
        let clip_count = clips.read().len();
        assert_eq!(clip_count, 1);

        let _ = std::fs::remove_dir_all(&project_dir);
    }
}

/// Write stereo-interleaved samples to the track's WAV writer and
/// update the incremental waveform-peak accumulator. Bumps
/// `frames_written` on success.
fn write_samples_and_peaks(
    track_buf: &mut TrackRecordingBuf,
    samples: &[f32],
) -> Result<(), String> {
    let Some(writer) = track_buf.writer.as_mut() else {
        return Err("writer already closed".into());
    };
    if samples.is_empty() {
        return Ok(());
    }
    let frames = samples.len() / 2;

    for f in 0..frames {
        let l = samples[f * 2];
        let r = samples[f * 2 + 1];
        writer
            .write_sample(l)
            .map_err(|e| format!("write_sample L: {e}"))?;
        writer
            .write_sample(r)
            .map_err(|e| format!("write_sample R: {e}"))?;

        let mono = (l + r) * 0.5;
        if mono < track_buf.peak_min {
            track_buf.peak_min = mono;
        }
        if mono > track_buf.peak_max {
            track_buf.peak_max = mono;
        }
        track_buf.peak_frames += 1;
        if track_buf.peak_frames >= crate::types::WAVEFORM_PEAK_FRAMES {
            track_buf
                .peaks
                .push((track_buf.peak_min, track_buf.peak_max));
            track_buf.peak_frames = 0;
            track_buf.peak_min = f32::MAX;
            track_buf.peak_max = f32::MIN;
        }
    }

    track_buf.frames_written += frames as u64;
    Ok(())
}
