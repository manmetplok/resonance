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
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// After a write failure: the frames actually salvaged on disk.
    pub frames_written: u64,
    /// A write to this take's WAV failed: the writer is closed, the file
    /// was cut back to the whole frames that reached the disk (with a
    /// header describing them) and `frames_written` counts those. The
    /// take still becomes a clip; later drains skip the track until the
    /// next pass opens a fresh writer (code review ENG-03).
    pub write_failed: bool,
    /// After a write failure whose header repair ALSO failed (e.g. a
    /// copy-on-write filesystem that cannot rewrite a block on a full
    /// disk): the salvaged frames, read back into memory. The take then
    /// becomes a `ClipSource::Memory` clip instead of mapping the file
    /// whose header is stale, and the next project save writes it out
    /// (FU-F2b).
    pub salvaged_audio: Option<Vec<f32>>,

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
    /// Cycle-record mode (set by `AudioCommand::SetLoopRecordMode`). When
    /// true and the transport loops while recording, the engine control
    /// thread rolls the capture into a distinct take at each loop seam
    /// (see [`RecordingState::roll_audio_pass`]) instead of producing one
    /// trimmed clip for the whole run.
    pub loop_record: bool,
    /// Set when a `Record` with `precount_bars > 0` is in its count-in
    /// phase. `target_sample` is the playhead position the user hit
    /// record at — once the playhead catches up to it, the input stream
    /// opens and recording begins. `restore_metronome` holds the
    /// metronome's pre-count-in state so it can be put back afterwards.
    pub precount: Option<PrecountState>,
    /// Timeline shift applied to the takes of this session when they
    /// are finalized, in samples (positive = content arrived that late
    /// and the clip is placed that much earlier). Set by the realtime
    /// bounce to the source external instrument's round-trip offset so
    /// the bounced take lands where the live return sounded (doc #260
    /// finding #4); 0 for normal recording.
    pub take_shift_samples: i64,
    /// Whether this session's latched (I/O-aligned) start position has
    /// been applied to `start_sample` by the engine loop (doc #260
    /// finding #2). Reset when a session opens; applied once the input
    /// callback clears `SharedState::recording_start_pending`.
    pub start_latch_applied: bool,
    /// Reusable per-track deinterleave scratch. Lives here rather than
    /// being a stack local in `drain_ring_to_buffers` so the engine
    /// thread doesn't allocate a fresh `Vec` 60× per second while
    /// recording.
    deint_scratch: Vec<f32>,
    /// Whether the current take's ring overflow has already been
    /// reported via `AudioEvent::RecordingOverflow`. Latched by
    /// [`RecordingState::poll_overflow`] so a sustained overflow emits
    /// once per take rather than once per drain poll; re-armed by
    /// [`RecordingState::begin_overflow_episode`] when the next take
    /// starts capturing.
    overflow_reported: bool,
    /// User-facing reports of take-file write failures not yet emitted;
    /// drained into `AudioEvent::Error` by
    /// [`RecordingState::poll_write_errors`].
    write_errors: Vec<String>,
}

/// Shift a finalized take `shift` samples earlier on the timeline: the
/// captured content arrived `shift` samples after the events that
/// caused it (an external instrument's round trip), so placing the clip
/// earlier by the same amount re-aligns content with the timeline. A
/// clip that would start before 0 is pinned at 0 and the overflow is
/// converted into leading trim so the audible alignment is preserved.
/// Non-positive shifts are no-ops. Pure; unit-tested.
pub fn apply_take_shift(
    start_sample: SamplePos,
    trim_start_frames: SamplePos,
    shift: i64,
) -> (SamplePos, SamplePos) {
    if shift <= 0 {
        return (start_sample, trim_start_frames);
    }
    let shift = shift as SamplePos;
    if start_sample >= shift {
        (start_sample - shift, trim_start_frames)
    } else {
        (0, trim_start_frames + (shift - start_sample))
    }
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
            loop_record: false,
            precount: None,
            take_shift_samples: 0,
            start_latch_applied: true,
            deint_scratch: Vec::with_capacity(DRAIN_SCRATCH_LEN),
            overflow_reported: false,
            write_errors: Vec::new(),
        }
    }

    /// Emit an [`AudioEvent::Error`] for every take-file write failure
    /// recorded since the last poll. The engine loop calls this every
    /// tick; [`RecordingState::finalize_recording`] calls it too, so a
    /// failure found by its final drain is reported with the take.
    pub fn poll_write_errors(&mut self, event_tx: &Sender<AudioEvent>) {
        for msg in self.write_errors.drain(..) {
            let _ = event_tx.send(AudioEvent::Error(EngineError::internal(msg)));
        }
    }

    /// Start a fresh overflow episode: zero the shared dropped-frame
    /// counter (`SharedState::recording_overflow`) and re-arm the
    /// one-shot report, so a take never inherits the previous take's
    /// damage count. Called when a record session opens its capture
    /// stream and again at each cycle-record seam — a new take starts
    /// clean.
    pub fn begin_overflow_episode(&mut self, dropped_frames: &AtomicU64) {
        dropped_frames.store(0, Ordering::Relaxed);
        self.overflow_reported = false;
    }

    /// One-shot overflow report for the current take. If the capture
    /// callbacks have discarded frames (`dropped_frames` — the shared
    /// `SharedState::recording_overflow` counter — is nonzero) and it
    /// has not been reported yet, emit [`AudioEvent::RecordingOverflow`]
    /// carrying the count so far and latch. The engine loop polls this
    /// right after every recording drain, so the report reaches the
    /// user while the damaged take is still being recorded without a
    /// sustained overflow flooding the event queue.
    pub fn poll_overflow(&mut self, dropped_frames: &AtomicU64, event_tx: &Sender<AudioEvent>) {
        if self.overflow_reported {
            return;
        }
        let dropped = dropped_frames.load(Ordering::Relaxed);
        if dropped == 0 {
            return;
        }
        self.overflow_reported = true;
        let _ = event_tx.send(AudioEvent::RecordingOverflow {
            dropped_frames: dropped,
        });
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
            write_failed: false,
            salvaged_audio: None,
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
                // A take whose file failed stays closed for the rest
                // of the pass; its salvaged audio is already final.
                if track_buf.writer.is_none() {
                    continue;
                }
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
                    self.write_errors.push(salvage_failed_take(track_buf, &e));
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
        // Tell the user about a failed take file alongside the take.
        self.poll_write_errors(event_tx);

        for (track_id, mut track_buf) in self.buffers.drain() {
            // Hand off the writer-flush / peak-close / WavWriter::finalize
            // work to a single helper so the rest of this loop is pure
            // state-mutation (clip emission, event broadcast, file removal
            // for empty or out-of-range takes).
            if let Err(e) = finalize_wav_file(&mut track_buf) {
                tracing::error!("recording: {e}");
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
            // Realtime-bounce external-instrument compensation: the
            // captured return is uniformly late by the source's round
            // trip, so the take is placed earlier by the same amount
            // (doc #260 finding #4).
            let (clip_start_sample, trim_start_frames) =
                apply_take_shift(clip_start_sample, trim_start_frames, self.take_shift_samples);

            // Memory-map the finalized WAV file (or adopt the in-memory
            // salvage of one whose header could not be repaired).
            let source = match take_clip_source(&track_buf.path, &mut track_buf.salvaged_audio) {
                Ok(src) => src,
                Err(e) => {
                    tracing::error!("recording: mmap {} failed: {e}", track_buf.path.display());
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
                vocal_tuning: None,
                warp_enabled: false,
                original_bpm: None,
                transpose_semitones: 0.0,
                warp_algorithm: Default::default(),
                warp_markers: Vec::new(),
                tuning_render_cache: None,
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

    /// Roll the current cycle-record pass for every armed audio track into
    /// a distinct clip and (when `reopen`) start a fresh writer for the
    /// next pass.
    ///
    /// The streaming resampler keeps running across the seam, so no input
    /// frames are dropped; its held-back lookahead is flushed into the
    /// finished take, so each take covers exactly its own input span. `clip_start_sample`
    /// positions the finished clips on the timeline (the loop region's
    /// start). Pass `reopen = true` at a loop seam (keeps capturing) and
    /// `reopen = false` for the trailing pass at transport stop (flushes
    /// the resampler tail and leaves the buffers closed).
    ///
    /// Returns one [`RolledAudioTake`] per track that captured at least one
    /// frame this pass; the caller wraps each into an
    /// `AudioEvent::TakeCaptured`.
    pub fn roll_audio_pass(
        &mut self,
        engine_sample_rate: u32,
        clip_start_sample: SamplePos,
        clips: &parking_lot::RwLock<Vec<AudioClip>>,
        audio_dir: &Path,
        next_clip_id: &mut ClipId,
        reopen: bool,
    ) -> Vec<RolledAudioTake> {
        // Stream any pending input into the current writers first so the
        // finished take includes everything captured up to the seam.
        self.drain_ring_to_buffers();

        let mut rolled = Vec::new();
        for (track_id, track_buf) in self.buffers.iter_mut() {
            // Close the current take's writer. At a seam the resampler's
            // held-back tail goes into this take and it keeps running, so
            // the next pass continues seamlessly; on the trailing pass we
            // flush its held tail and finish.
            if reopen {
                close_pass_writer(track_buf);
            } else if let Err(e) = finalize_wav_file(track_buf) {
                tracing::error!("recording: {e}");
                continue;
            }

            let finished_path = track_buf.path.clone();
            let finished_clip_id = track_buf.clip_id;
            let finished_frames = track_buf.frames_written;
            let finished_peaks = std::mem::take(&mut track_buf.peaks);
            let mut finished_salvage = track_buf.salvaged_audio.take();

            // Reopen a fresh writer for the next pass (seam only).
            if reopen {
                let new_clip_id = *next_clip_id;
                *next_clip_id += 1;
                match open_track_wav_file(audio_dir, new_clip_id, engine_sample_rate) {
                    Ok((path, writer)) => {
                        track_buf.writer = Some(writer);
                        track_buf.path = path;
                        track_buf.clip_id = new_clip_id;
                        track_buf.frames_written = 0;
                        track_buf.write_failed = false;
                        track_buf.peak_min = f32::MAX;
                        track_buf.peak_max = f32::MIN;
                        track_buf.peak_frames = 0;
                    }
                    Err(e) => {
                        tracing::error!("recording: reopen pass writer failed: {e}");
                        self.write_errors.push(format!(
                            "Recording stopped on this track: could not open the next take \
                             file ({e})."
                        ));
                        // Leave the writer closed; later passes for this
                        // track simply produce nothing — and must not
                        // re-emit the take that just rolled.
                        track_buf.frames_written = 0;
                    }
                }
            }

            if finished_frames == 0 {
                // Empty pass (no input, or stop landed on the seam): drop
                // the empty WAV and emit no take.
                let _ = std::fs::remove_file(&finished_path);
                continue;
            }

            let source = match take_clip_source(&finished_path, &mut finished_salvage) {
                Ok(src) => src,
                Err(e) => {
                    tracing::error!("recording: mmap {} failed: {e}", finished_path.display());
                    continue;
                }
            };
            let name = format!("Take {}", finished_clip_id);
            let clip = AudioClip {
                id: finished_clip_id,
                track_id: *track_id,
                start_sample: clip_start_sample,
                source,
                name,
                trim_start_frames: 0,
                trim_end_frames: 0,
                fade_in_frames: 0,
                fade_in_curve: FadeCurve::default(),
                fade_out_frames: 0,
                fade_out_curve: FadeCurve::default(),
                gain_db: 0.0,
                vocal_tuning: None,
                warp_enabled: false,
                original_bpm: None,
                transpose_semitones: 0.0,
                warp_algorithm: Default::default(),
                warp_markers: Vec::new(),
                tuning_render_cache: None,
            };
            clips.write().push(clip);
            rolled.push(RolledAudioTake {
                track_id: *track_id,
                clip_id: finished_clip_id,
                start_sample: clip_start_sample,
                duration_samples: finished_frames,
                waveform_peaks: finished_peaks,
            });
        }

        if !reopen {
            self.buffers.clear();
            self.ring_consumer = None;
        }
        rolled
    }
}

/// A finished audio take produced by rolling a cycle-record pass at a loop
/// seam (or at transport stop for the trailing pass). The owning
/// [`RecordingState::roll_audio_pass`] has already pushed the matching
/// [`AudioClip`] into the shared clip map; this carries the metadata the
/// engine control thread needs to emit `AudioEvent::TakeCaptured`.
#[derive(Debug, Clone)]
pub struct RolledAudioTake {
    pub track_id: TrackId,
    pub clip_id: ClipId,
    pub start_sample: SamplePos,
    pub duration_samples: u64,
    pub waveform_peaks: Vec<(f32, f32)>,
}

impl RolledAudioTake {
    /// The stretch of timeline this pass actually recorded over — the
    /// rolled clip's own `[start_sample, start_sample + duration_samples)`.
    ///
    /// **Not the loop slot**, and that is the point. Pass 0's writer starts
    /// where the user punched in rather than at the loop start, and the
    /// trailing pass at transport stop ends wherever the user stopped, so
    /// both are strictly shorter than the region they are filed under.
    /// `finalize_loop_record_pass` files this onto the take and reports it
    /// on `AudioEvent::TakeCaptured`, because it is the app's **only**
    /// account of what a pass recorded: no `RecordingFinished` follows a
    /// take clip, so the clip never reaches the app's mirror and a
    /// consumer told only the slot would claim material that does not
    /// exist (ba todo #1396,
    /// [`resonance_common::Take::audible_extent`]).
    ///
    /// Defined here, on the value the emit site reads, so the capture path
    /// and `tests/engine/loop_record_takes.rs` cannot describe it differently.
    pub fn extent(&self) -> resonance_common::TimelineRange {
        resonance_common::TimelineRange::new(self.start_sample, self.duration_samples)
    }
}

/// Close a take's WAV writer at a loop seam. The streaming resampler is
/// flushed into this take (it holds back ~1 ms of lookahead) but keeps
/// running: its flush is resumable, so the next pass's writer continues
/// the input stream on the same time grid instead of starting with this
/// pass's held-back tail (LIB-01). Commits the trailing peak bucket and
/// finalizes the writer so the on-disk WAV header carries the correct
/// data-chunk size.
fn close_pass_writer(track_buf: &mut TrackRecordingBuf) {
    if !track_buf.write_failed && track_buf.writer.is_some() {
        if let Some(r) = track_buf.resampler.as_mut() {
            let mut tail = std::mem::take(&mut track_buf.resample_scratch);
            tail.clear();
            r.flush(&mut tail);
            if let Err(e) = write_samples_and_peaks(track_buf, &tail) {
                tracing::error!(
                    "recording: seam flush failed for {}: {e}",
                    track_buf.path.display()
                );
            }
            track_buf.resample_scratch = tail;
        }
    }
    if track_buf.peak_frames > 0 {
        track_buf
            .peaks
            .push((track_buf.peak_min, track_buf.peak_max));
        track_buf.peak_frames = 0;
        track_buf.peak_min = f32::MAX;
        track_buf.peak_max = f32::MIN;
    }
    if let Some(writer) = track_buf.writer.take() {
        if let Err(e) = writer.finalize() {
            tracing::error!(
                "recording: finalize pass wav {}: {e}",
                track_buf.path.display()
            );
        }
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
    // A take whose file failed mid-recording was already salvaged and
    // closed (header fixed, peaks trimmed); the file is valid as is.
    if track_buf.write_failed {
        return Ok(());
    }
    // Flush any trailing resampled frame.
    if let Some(r) = track_buf.resampler.as_mut() {
        track_buf.resample_scratch.clear();
        r.flush(&mut track_buf.resample_scratch);
        if !track_buf.resample_scratch.is_empty() {
            let tail: Vec<f32> = std::mem::take(&mut track_buf.resample_scratch);
            if let Err(e) = write_samples_and_peaks(track_buf, &tail) {
                tracing::error!(
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

/// Handle a failed write to a take's WAV: close the writer, cut the file
/// back to the whole frames that actually reached the disk and rewrite
/// its header to match, so the take up to the failure stays a valid,
/// mappable clip (code review ENG-03). `frames_written` becomes the
/// salvaged count (0 if nothing could be recovered) and the peaks are
/// trimmed to it. Returns the user-facing error message.
fn salvage_failed_take(track_buf: &mut TrackRecordingBuf, err: &str) -> String {
    tracing::error!(
        "recording: write failed for {}: {err}",
        track_buf.path.display()
    );
    if let Some(writer) = track_buf.writer.take() {
        // Best effort: flushes what it can and drops the file handle
        // (dropping it later could still append buffered bytes behind
        // the repair below). Its header update usually fails too.
        let _ = writer.finalize();
    }
    track_buf.write_failed = true;
    let salvaged = repair_wav_data_len(&track_buf.path);
    let frames = match &salvaged {
        Ok(WavRepair::Repaired { frames }) => *frames,
        Ok(WavRepair::HeaderStale { samples, .. }) => (samples.len() / 2) as u64,
        Err(_) => 0,
    };
    track_buf.frames_written = frames;

    // Keep the peaks in step with the audio that survived.
    let peak_frames = crate::types::WAVEFORM_PEAK_FRAMES as u64;
    let want = frames.div_ceil(peak_frames) as usize;
    if track_buf.peaks.len() < want && track_buf.peak_frames > 0 {
        track_buf
            .peaks
            .push((track_buf.peak_min, track_buf.peak_max));
    }
    track_buf.peaks.truncate(want);
    track_buf.peak_frames = 0;
    track_buf.peak_min = f32::MAX;
    track_buf.peak_max = f32::MIN;

    let name = track_buf
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match salvaged {
        Ok(WavRepair::Repaired { frames }) => format!(
            "Recording stopped on this track: writing {name} failed ({err}). The take was \
             kept up to the failure ({frames} frames)."
        ),
        Ok(WavRepair::HeaderStale {
            samples,
            header_error,
        }) => {
            tracing::error!(
                "recording: header repair of {} failed: {header_error}",
                track_buf.path.display()
            );
            track_buf.salvaged_audio = Some(samples);
            format!(
                "Recording stopped on this track: writing {name} failed ({err}). The take was \
                 kept up to the failure ({frames} frames), but its file header could not be \
                 repaired ({header_error}), so it is held in memory only — free some disk \
                 space and save the project to write it out."
            )
        }
        Err(e) => format!(
            "Recording stopped on this track: writing {name} failed ({err}), and the audio \
             captured so far could not be recovered ({e})."
        ),
    }
}

/// Outcome of [`repair_wav_data_len`] when the audio itself was found.
enum WavRepair {
    /// The file was cut back to whole frames and its header rewritten;
    /// it maps as a valid WAV of `frames` frames.
    Repaired { frames: u64 },
    /// The in-place header rewrite failed (e.g. a copy-on-write
    /// filesystem needs free space even to overwrite a block), so the
    /// file's header is stale; the whole frames present were read back
    /// into `samples` (stereo interleaved) instead (FU-F2b).
    HeaderStale {
        samples: Vec<f32>,
        header_error: String,
    },
}

/// Make a partially written stereo float WAV valid again: find its
/// `data` chunk, cut the file back to the whole frames present (at most
/// what a RIFF size field can describe), and rewrite the RIFF and `data`
/// sizes. Only shrinks the file and overwrites header bytes in place, so
/// it works on the full disk that caused the failure on most
/// filesystems. Where even that fails, the frames are read back into
/// memory rather than lost ([`WavRepair::HeaderStale`]). `Err` only when
/// the audio could not be located or read at all.
fn repair_wav_data_len(path: &Path) -> Result<WavRepair, String> {
    const FRAME_BYTES: u64 = 2 * 4; // stereo f32
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    // Reading needs no free space; open for writing only if we can.
    let (mut file, open_rw_error) = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    {
        Ok(f) => (f, None),
        Err(e) => (File::open(path).map_err(err)?, Some(err(e))),
    };
    let file_len = file.metadata().map_err(err)?.len();

    // Walk the chunk list after "RIFF....WAVE" to the data chunk. Every
    // chunk before it is complete (hound writes them up front).
    let mut pos = 12u64;
    let data_start = loop {
        if pos + 8 > file_len {
            return Err("no data chunk".into());
        }
        let mut head = [0u8; 8];
        file.seek(SeekFrom::Start(pos)).map_err(err)?;
        file.read_exact(&mut head).map_err(err)?;
        if &head[0..4] == b"data" {
            break pos + 8;
        }
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as u64;
        pos += 8 + size + (size & 1);
    };

    let max_data = (u32::MAX as u64 + 8).saturating_sub(data_start);
    let data_len = (file_len - data_start).min(max_data) / FRAME_BYTES * FRAME_BYTES;

    let rewrite = |file: &mut File| -> Result<(), String> {
        if let Some(e) = &open_rw_error {
            return Err(e.clone());
        }
        file.set_len(data_start + data_len).map_err(err)?;
        file.seek(SeekFrom::Start(4)).map_err(err)?;
        file.write_all(&((data_start + data_len - 8) as u32).to_le_bytes())
            .map_err(err)?;
        file.seek(SeekFrom::Start(data_start - 4)).map_err(err)?;
        file.write_all(&(data_len as u32).to_le_bytes())
            .map_err(err)?;
        file.sync_all().map_err(err)
    };
    match rewrite(&mut file) {
        Ok(()) => Ok(WavRepair::Repaired {
            frames: data_len / FRAME_BYTES,
        }),
        Err(header_error) => {
            // A failed set_len may or may not have shrunk the file; only
            // read what is still there.
            let present = file
                .metadata()
                .map_err(err)?
                .len()
                .saturating_sub(data_start)
                .min(data_len)
                / FRAME_BYTES
                * FRAME_BYTES;
            let mut bytes = vec![0u8; present as usize];
            file.seek(SeekFrom::Start(data_start)).map_err(err)?;
            file.read_exact(&mut bytes).map_err(err)?;
            let samples = bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();
            Ok(WavRepair::HeaderStale {
                samples,
                header_error,
            })
        }
    }
}

/// The clip source for a finished take: its in-memory salvage when the
/// file's header could not be repaired (FU-F2b), otherwise the mapped
/// file.
fn take_clip_source(path: &Path, salvaged: &mut Option<Vec<f32>>) -> Result<ClipSource, String> {
    match salvaged.take() {
        Some(samples) => Ok(ClipSource::Memory(samples)),
        None => ClipSource::open_wav(path),
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
