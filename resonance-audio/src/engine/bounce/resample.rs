//! Export-time sample-rate conversion (doc #196).
//!
//! When the export's target sample rate differs from the engine rate, the
//! render loop's interleaved stereo `f32` frames flow through a
//! [`ResampleStage`] before reaching the encoder sink. It wraps a
//! `rubato` FFT resampler, which processes fixed-size input chunks; this
//! stage accumulates streamed frames into those chunks, drops the
//! resampler's leading output delay, and trims the tail to the exact
//! expected output length so the file's duration matches the source.

use rubato::{FftFixedIn, Resampler};

use super::encoder::{EncoderError, EncoderSink};

/// Fixed input chunk fed to the FFT resampler. A power of two keeps the
/// internal FFT efficient; 1024 frames is ~21 ms at 48 kHz, small enough
/// that the per-chunk allocation churn is negligible against render cost.
const RESAMPLE_CHUNK: usize = 1024;

pub(super) struct ResampleStage {
    resampler: FftFixedIn<f32>,
    /// Per-channel input accumulation (planar), drained `chunk` at a time.
    in_l: Vec<f32>,
    in_r: Vec<f32>,
    chunk: usize,
    /// Remaining leading output frames to discard (the resampler's group
    /// delay), so the output aligns with the source.
    skip: u64,
    /// Output frames already emitted to the sink.
    written: u64,
    /// Exact target output length in frames; the tail is trimmed to this.
    target: u64,
    /// Total input frames seen, used to recompute `target` as frames arrive.
    in_frames: u64,
    ratio: f64,
}

impl ResampleStage {
    pub(super) fn new(from_sr: u32, to_sr: u32) -> Result<Self, EncoderError> {
        let resampler = FftFixedIn::<f32>::new(from_sr as usize, to_sr as usize, RESAMPLE_CHUNK, 2, 2)
            .map_err(|e| EncoderError::Io(format!("Resampler init failed: {e}")))?;
        let skip = resampler.output_delay() as u64;
        Ok(ResampleStage {
            resampler,
            in_l: Vec::new(),
            in_r: Vec::new(),
            chunk: RESAMPLE_CHUNK,
            skip,
            written: 0,
            target: 0,
            in_frames: 0,
            ratio: to_sr as f64 / from_sr as f64,
        })
    }

    /// Resample `frames` (interleaved stereo f32) and forward the result to
    /// `sink`. Whole input chunks are processed eagerly; a partial
    /// remainder is held until the next call or [`flush`](Self::flush).
    pub(super) fn process(
        &mut self,
        frames: &[f32],
        sink: &mut dyn EncoderSink,
    ) -> Result<(), EncoderError> {
        for f in frames.chunks_exact(2) {
            self.in_l.push(f[0]);
            self.in_r.push(f[1]);
        }
        self.in_frames += (frames.len() / 2) as u64;
        self.target = (self.in_frames as f64 * self.ratio).round() as u64;

        while self.in_l.len() >= self.chunk {
            self.process_one_chunk(false, sink)?;
        }
        Ok(())
    }

    /// Drain the remainder: pad the partial input chunk with silence and
    /// feed zero chunks until the expected output length is reached.
    pub(super) fn flush(&mut self, sink: &mut dyn EncoderSink) -> Result<(), EncoderError> {
        // Process any buffered partial chunk (zero-padded to `chunk`).
        if !self.in_l.is_empty() {
            self.process_one_chunk(true, sink)?;
        }
        // The resampler's group delay means some target frames are still in
        // flight; feed silent chunks until they emerge (bounded so a bad
        // ratio can't spin forever).
        let mut guard = 0;
        while self.written < self.target && guard < 64 {
            self.in_l.clear();
            self.in_r.clear();
            self.process_one_chunk(true, sink)?;
            guard += 1;
        }
        Ok(())
    }

    fn process_one_chunk(
        &mut self,
        pad: bool,
        sink: &mut dyn EncoderSink,
    ) -> Result<(), EncoderError> {
        if pad {
            self.in_l.resize(self.chunk, 0.0);
            self.in_r.resize(self.chunk, 0.0);
        }
        let input: [&[f32]; 2] = [&self.in_l[..self.chunk], &self.in_r[..self.chunk]];
        let out = self
            .resampler
            .process(&input, None)
            .map_err(|e| EncoderError::Io(format!("Resample error: {e}")))?;
        // Consume the processed input frames.
        self.in_l.drain(..self.chunk);
        self.in_r.drain(..self.chunk);

        let (out_l, out_r) = (&out[0], &out[1]);
        let mut interleaved = Vec::with_capacity(out_l.len() * 2);
        for i in 0..out_l.len() {
            // Drop leading group-delay frames, then stop at the exact target.
            if self.skip > 0 {
                self.skip -= 1;
                continue;
            }
            if self.written >= self.target {
                break;
            }
            interleaved.push(out_l[i]);
            interleaved.push(out_r[i]);
            self.written += 1;
        }
        if !interleaved.is_empty() {
            sink.write_frames(&interleaved)?;
        }
        Ok(())
    }
}
