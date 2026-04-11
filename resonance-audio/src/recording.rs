/// Recording state and logic for the audio engine.

use std::collections::HashMap;

use crossbeam_channel::Sender;
use ringbuf::traits::Consumer;

use crate::decode;
use crate::types::*;

/// Per-track recording scratch: the audio buffer plus a snapshot of the
/// track's port/mono settings captured at record-start time so the drain
/// path can extract the right channel(s) without looking up track state.
pub(crate) struct TrackRecordingBuf {
    pub data: Vec<f32>,
    /// 0-indexed starting channel in the interleaved input stream. For
    /// a stereo track this is the left channel; for a mono track it's
    /// the single captured channel.
    pub input_port: u16,
    /// True = capture one channel and duplicate to L/R. False = capture
    /// two consecutive channels as L/R.
    pub mono: bool,
}

/// Groups all mutable recording state that lives on the engine thread.
pub(crate) struct RecordingState {
    pub buffers: HashMap<TrackId, TrackRecordingBuf>,
    pub start_sample: SamplePos,
    pub ring_consumer: Option<ringbuf::HeapCons<f32>>,
    pub input_stream: Option<cpal::Stream>,
    pub input_channels: u16,
    pub input_sample_rate: u32,
    pub loop_enabled: bool,
    pub loop_in: SamplePos,
    pub loop_out: SamplePos,
}

impl RecordingState {
    /// Create a new RecordingState with defaults for the given output sample rate.
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
        }
    }

    /// Drain all available samples from the ring buffer consumer into
    /// per-track recording buffers. Each track picks its own channel(s)
    /// out of the interleaved multi-channel stream via its snapshotted
    /// `input_port` and `mono` flags — so two tracks on the same input
    /// device can simultaneously capture different physical inputs.
    pub fn drain_ring_to_buffers(&mut self) {
        let Some(ref mut consumer) = self.ring_consumer else {
            return;
        };
        let channels = self.input_channels as usize;
        if channels == 0 {
            return;
        }
        let mut temp = [0.0f32; 4096];
        loop {
            let count = consumer.pop_slice(&mut temp);
            if count == 0 {
                break;
            }
            let chunk = &temp[..count];
            let frames = chunk.len() / channels;
            if frames == 0 {
                continue;
            }

            for track_buf in self.buffers.values_mut() {
                let port = (track_buf.input_port as usize).min(channels - 1);
                let right_port = if track_buf.mono {
                    port
                } else {
                    (port + 1).min(channels - 1)
                };
                track_buf.data.reserve(frames * 2);
                for f in 0..frames {
                    let base = f * channels;
                    let l = chunk[base + port];
                    let r = chunk[base + right_port];
                    track_buf.data.push(l);
                    track_buf.data.push(r);
                }
            }
        }
    }

    /// Finalize recording: drain remaining samples, create clips, emit events.
    pub fn finalize_recording(
        &mut self,
        output_sample_rate: u32,
        next_clip_id: &mut ClipId,
        clips: &parking_lot::RwLock<Vec<AudioClip>>,
        event_tx: &Sender<AudioEvent>,
    ) {
        self.drain_ring_to_buffers();

        for (track_id, track_buf) in self.buffers.drain() {
            if track_buf.data.is_empty() {
                continue;
            }

            let clip_id = *next_clip_id;
            *next_clip_id += 1;

            let final_data = if self.input_sample_rate != output_sample_rate {
                decode::linear_resample(&track_buf.data, self.input_sample_rate, output_sample_rate)
            } else {
                track_buf.data
            };

            // Trim to loop range if enabled
            let (clip_start_sample, final_data) =
                if self.loop_enabled && self.loop_out > self.loop_in {
                    let total_frames = (final_data.len() / 2) as u64;
                    let trim_start_frame =
                        self.loop_in.saturating_sub(self.start_sample);
                    let trim_end_frame = self
                        .loop_out
                        .saturating_sub(self.start_sample)
                        .min(total_frames);

                    if trim_start_frame >= trim_end_frame {
                        continue; // Nothing in the loop range
                    }

                    // Skip copy if trim covers the full buffer
                    if trim_start_frame == 0 && trim_end_frame == total_frames {
                        (self.loop_in, final_data)
                    } else {
                        let trim_start_idx = (trim_start_frame * 2) as usize;
                        let trim_end_idx = (trim_end_frame * 2) as usize;
                        (
                            self.loop_in,
                            final_data[trim_start_idx..trim_end_idx].to_vec(),
                        )
                    }
                } else {
                    (self.start_sample, final_data)
                };

            let duration_samples = (final_data.len() / 2) as u64;
            let name = format!("Recording {}", clip_id);
            let waveform_peaks = compute_waveform_peaks(&final_data);

            let clip = AudioClip {
                id: clip_id,
                track_id,
                start_sample: clip_start_sample,
                data: final_data,
                name: name.clone(),
                trim_start_frames: 0,
                trim_end_frames: 0,
            };
            // Minimize write lock duration: only hold lock for the push
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
                waveform_peaks,
            });
        }

        self.ring_consumer = None;
    }
}
