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
pub struct TrackRecordingBuf {
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
pub struct RecordingState {
    pub buffers: HashMap<TrackId, TrackRecordingBuf>,
    pub start_sample: SamplePos,
    pub ring_consumer: Option<ringbuf::HeapCons<f32>>,
    pub(crate) input_stream: Option<crate::input_handle::InputHandle>,
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
    /// Reusable per-track deinterleave scratch. Lives here rather than
    /// being a stack local in `drain_ring_to_buffers` so the engine
    /// thread doesn't allocate a fresh `Vec` 60× per second while
    /// recording.
    deint_scratch: Vec<f32>,
}

#[derive(Debug, Clone, Copy)]
pub struct PrecountState {
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
            deint_scratch: Vec::with_capacity(DRAIN_SCRATCH_LEN),
        }
    }

    /// Create a `TrackRecordingBuf` for an armed track: allocates
    /// the clip id, opens a WAV writer at `{project_dir}/audio/clip_{id}.wav`
    /// (creating the audio dir on demand), and sets up a streaming
    /// resampler if the input device doesn't match the engine rate.
    ///
    /// The filesystem work (creating the audio dir and opening the
    /// WAV writer) is delegated to [`open_track_wav_file`], so this
    /// function's remaining responsibility is the pure struct
    /// assembly: resampler decision plus default-value initialisation.
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
        let (path, writer) = open_track_wav_file(&audio_dir, clip_id, engine_sample_rate)?;

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
        // Pop whole frames only: a partial frame left in the scratch
        // tail would be consumed but never written, rotating channel
        // alignment for the rest of the take.
        let scratch_len = (DRAIN_SCRATCH_LEN / channels) * channels;
        let deint_scratch = &mut self.deint_scratch;

        loop {
            let count = consumer.pop_slice(&mut ring_scratch[..scratch_len]);
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
                    r.process(deint_scratch, &mut buf);
                    Some(buf)
                } else {
                    None
                };
                let write_result = if let Some(ref buf) = resampled {
                    write_samples_and_peaks(track_buf, buf)
                } else {
                    write_samples_and_peaks(track_buf, deint_scratch)
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
            // Hand off the writer-flush / peak-close / WavWriter::finalize
            // work to a single helper so the rest of this loop is pure
            // state-mutation (clip emission, event broadcast, file removal
            // for empty or out-of-range takes).
            if let Err(e) = finalize_wav_file(&mut track_buf) {
                eprintln!("recording: {e}");
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
                fade_in_frames: 0,
                fade_in_curve: FadeCurve::default(),
                fade_out_frames: 0,
                fade_out_curve: FadeCurve::default(),
                gain_db: 0.0,
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

/// Open a streaming WAV writer for one recording take. Creates
/// `audio_dir` on demand and returns the final path alongside the
/// writer. Pulled out of [`RecordingState::create_track_buf`] so the
/// struct-assembly half of that function stays filesystem-free.
fn open_track_wav_file(
    audio_dir: &Path,
    clip_id: ClipId,
    engine_sample_rate: u32,
) -> Result<(PathBuf, WavWriter<BufWriter<File>>), String> {
    std::fs::create_dir_all(audio_dir)
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
    Ok((path, writer))
}

/// Close out one track's WAV: flush any trailing resampled frame,
/// commit the in-progress peak bucket, and finalize the `WavWriter`
/// so its header carries the correct data-chunk size.
///
/// Pulled out of [`RecordingState::finalize_recording`] so the
/// state-mutation half (clip emission, event broadcast, file removal
/// on empty / out-of-range takes) operates on a `TrackRecordingBuf`
/// whose writer is already closed. A return value of `Ok(())` means
/// `track_buf.writer` is now `None` and the on-disk file is valid;
/// `Err(_)` means the file should be considered corrupt.
fn finalize_wav_file(track_buf: &mut TrackRecordingBuf) -> Result<(), String> {
    // Flush any trailing resampled frame.
    if let Some(r) = track_buf.resampler.as_mut() {
        track_buf.resample_scratch.clear();
        r.flush(&mut track_buf.resample_scratch);
        if !track_buf.resample_scratch.is_empty() {
            let tail: Vec<f32> = std::mem::take(&mut track_buf.resample_scratch);
            if let Err(e) = write_samples_and_peaks(track_buf, &tail) {
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
    let Some(writer) = track_buf.writer.take() else {
        // Writer was dropped earlier due to a write error.
        return Err("writer already closed".into());
    };
    writer
        .finalize()
        .map_err(|e| format!("finalize wav {}: {e}", track_buf.path.display()))
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
