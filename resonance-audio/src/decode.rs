/// Audio file decoding using symphonia.
use std::path::Path;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

/// Decode an audio file to stereo interleaved f32 samples at the target sample rate.
pub fn decode_file(path: &str, target_sample_rate: u32) -> Result<(Vec<f32>, String), String> {
    let path = Path::new(path);
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("untitled")
        .to_string();

    let file = std::fs::File::open(path).map_err(|e| format!("Failed to open file: {}", e))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| format!("Failed to probe format: {}", e))?;

    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| "No default audio track found".to_string())?;

    let audio_params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| "Track has no audio codec parameters".to_string())?
        .clone();

    let source_sample_rate = audio_params.sample_rate.unwrap_or(44100);
    let channels = audio_params
        .channels
        .as_ref()
        .map(|c| c.count())
        .unwrap_or(2);
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&audio_params, &AudioDecoderOptions::default())
        .map_err(|e| format!("Failed to create decoder: {}", e))?;

    let mut raw_samples: Vec<f32> = Vec::new();

    while let Some(packet) = format
        .next_packet()
        .map_err(|e| format!("Failed to read packet: {}", e))?
    {
        if packet.track_id != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(buf) => buf,
            Err(_) => continue,
        };

        decoded.copy_to_vec_interleaved(&mut raw_samples);
    }

    // Convert to stereo interleaved
    let stereo = to_stereo_interleaved(&raw_samples, channels);

    // Resample if needed
    let output = if source_sample_rate != target_sample_rate {
        linear_resample(&stereo, source_sample_rate, target_sample_rate)
    } else {
        stereo
    };

    Ok((output, name))
}

/// Convert any channel layout to stereo interleaved.
fn to_stereo_interleaved(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 2 {
        return samples.to_vec();
    }

    let frames = samples.len() / channels;
    let mut stereo = Vec::with_capacity(frames * 2);

    for frame in 0..frames {
        let base = frame * channels;
        let left = samples[base];
        let right = if channels > 1 {
            samples[base + 1]
        } else {
            left
        };
        stereo.push(left);
        stereo.push(right);
    }

    stereo
}

/// Simple linear interpolation resampler for stereo interleaved audio.
pub fn linear_resample(input: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    let source_frames = input.len() / 2;
    let ratio = source_rate as f64 / target_rate as f64;
    let target_frames = (source_frames as f64 / ratio) as usize;
    let mut output = Vec::with_capacity(target_frames * 2);

    for i in 0..target_frames {
        let src_pos = i as f64 * ratio;
        let src_idx = src_pos as usize;
        let frac = src_pos - src_idx as f64;

        let idx0 = src_idx.min(source_frames.saturating_sub(1));
        let idx1 = (src_idx + 1).min(source_frames.saturating_sub(1));

        let l0 = input[idx0 * 2];
        let r0 = input[idx0 * 2 + 1];
        let l1 = input[idx1 * 2];
        let r1 = input[idx1 * 2 + 1];

        let frac = frac as f32;
        output.push(l0 + (l1 - l0) * frac);
        output.push(r0 + (r1 - r0) * frac);
    }

    output
}

/// Stateful linear resampler for stereo interleaved audio that can
/// be fed in chunks without introducing discontinuities at chunk
/// boundaries. Used by the recording drain loop so takes stream to
/// disk at the engine sample rate without ever materialising the
/// full buffer.
///
/// The algorithm matches [`linear_resample`] in steady state — for
/// the same total input the emitted samples agree to within the
/// precision of `f64` phase accumulation — but carries a one-frame
/// tail and a fractional phase across calls. Callers should emit a
/// final [`StreamingLinearResampler::flush`] when the input stream
/// ends so the trailing frame isn't lost.
pub struct StreamingLinearResampler {
    /// Ratio = source_rate / target_rate. Each output frame advances
    /// the read head by `ratio` input frames.
    ratio: f64,
    /// Next read position, measured in input frames, as a
    /// floating-point offset into the virtual concatenation of every
    /// chunk the caller has pushed so far.
    next_src_pos: f64,
    /// Total input frames consumed across all prior calls. Used to
    /// translate `next_src_pos` into a chunk-local index.
    consumed_frames: u64,
    /// The last input frame from the previous chunk, retained so that
    /// interpolating at position `consumed_frames - 1 + frac` works
    /// correctly on the first output frame of the next chunk.
    last_frame: Option<[f32; 2]>,
}

impl StreamingLinearResampler {
    pub fn new(source_rate: u32, target_rate: u32) -> Self {
        Self {
            ratio: source_rate as f64 / target_rate as f64,
            next_src_pos: 0.0,
            consumed_frames: 0,
            last_frame: None,
        }
    }

    /// Process a chunk of stereo-interleaved f32 input, appending
    /// resampled stereo frames to `output`. Does not emit the final
    /// partial frame; call [`StreamingLinearResampler::flush`] once
    /// after the last chunk to drain it.
    pub fn process(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let chunk_frames = input.len() / 2;
        if chunk_frames == 0 {
            return;
        }

        // Total frames available in the virtual stream up to the end
        // of this chunk.
        let total_frames = self.consumed_frames + chunk_frames as u64;

        // Emit output frames while both neighbours of the
        // interpolation window are fully inside the data we've seen
        // so far. The right neighbour lives at index `src_idx + 1`,
        // so we stop when `src_idx + 1 >= total_frames`, i.e. when
        // `src_idx >= total_frames - 1`.
        loop {
            let src_idx_f = self.next_src_pos.floor();
            if src_idx_f < 0.0 {
                // Shouldn't happen — phase is monotonically
                // non-negative — but guard defensively.
                self.next_src_pos += self.ratio;
                continue;
            }
            let src_idx = src_idx_f as u64;
            if src_idx + 1 >= total_frames {
                break;
            }
            let frac = (self.next_src_pos - src_idx_f) as f32;

            let [l0, r0] = self.sample_at(src_idx, input);
            let [l1, r1] = self.sample_at(src_idx + 1, input);

            output.push(l0 + (l1 - l0) * frac);
            output.push(r0 + (r1 - r0) * frac);

            self.next_src_pos += self.ratio;
        }

        // Retain the last input frame for the next chunk so that a
        // read at index `total_frames - 1` still resolves.
        self.last_frame = Some([
            input[(chunk_frames - 1) * 2],
            input[(chunk_frames - 1) * 2 + 1],
        ]);
        self.consumed_frames = total_frames;
    }

    /// Emit any trailing output frame that wasn't produced during
    /// [`StreamingLinearResampler::process`] because the right
    /// neighbour would have fallen past the end of the stream. Uses
    /// the retained last frame for both neighbours (equivalent to
    /// clamping the read position to the final input frame).
    pub fn flush(&mut self, output: &mut Vec<f32>) {
        let Some(last) = self.last_frame else {
            return;
        };
        let total_frames = self.consumed_frames;
        if total_frames == 0 {
            return;
        }
        loop {
            let src_idx_f = self.next_src_pos.floor();
            let src_idx = src_idx_f as u64;
            if src_idx >= total_frames {
                break;
            }
            // Both neighbours clamped to the last valid frame.
            output.push(last[0]);
            output.push(last[1]);
            self.next_src_pos += self.ratio;
        }
    }

    /// Resolve a frame at a virtual index into either the retained
    /// tail frame (if it points at `consumed_frames - 1`) or the
    /// current chunk (otherwise). Assumes the caller has already
    /// checked that `idx < consumed_frames + chunk_frames`.
    #[inline]
    fn sample_at(&self, idx: u64, chunk: &[f32]) -> [f32; 2] {
        if idx < self.consumed_frames {
            // Only the single most-recent past frame is retained,
            // so any earlier reference is a bug. Clamp as a safety
            // net — the first `process` call starts at `0` anyway.
            if let Some(last) = self.last_frame {
                return last;
            }
            return [chunk[0], chunk[1]];
        }
        let local = (idx - self.consumed_frames) as usize;
        [chunk[local * 2], chunk[local * 2 + 1]]
    }
}
