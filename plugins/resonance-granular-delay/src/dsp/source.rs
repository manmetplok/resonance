//! Stage 1 — the source ring: the stereo circular buffer every grain
//! reads from, the write head that fills it, the freeze crossfade that
//! stops that head (ba todo #1075) and the coarse peak mip of what was
//! written (ba todo #1135).

use crate::viz::PEAK_BINS;

use super::feedback::FeedbackStage;
use super::BlockParams;

/// Maximum delay time the ring buffer is sized for at activation.
pub const MAX_DELAY_SECONDS: f32 = 4.0;

/// Freeze engage/resume ramp length (ba todo #1075, doc #252 §1): the
/// write gain crossfades over this many seconds so stopping/restarting
/// the write head never splices a discontinuity into the buffer.
pub const FREEZE_RAMP_SECONDS: f32 = 0.005;

/// The grain source buffer and everything that writes into it.
pub(super) struct SourceRing {
    /// Stereo circular source buffers; length is a power of two (the
    /// `read_hermite_wrapped` contract) sized for [`MAX_DELAY_SECONDS`].
    pub(super) buf_l: Vec<f32>,
    pub(super) buf_r: Vec<f32>,
    mask: usize,
    /// Absolute write-head position in samples (monotonic; masked for
    /// indexing, passed absolute to the engines).
    pub(super) write_pos: u64,
    /// Freeze crossfade position: 0 = live (writes at full gain), 1 =
    /// fully frozen (write head stopped, buffer untouched). Ramps by
    /// one sample step per input sample toward the freeze target, and
    /// the write is an equal-power blend of held content and incoming
    /// signal while in between (ba todo #1075).
    freeze_xf: f32,
    /// Coarse absolute-peak bins over the source ring (ba todo #1135):
    /// bin `((pos >> peak_shift) & (PEAK_BINS − 1))` accumulates the
    /// peak of the samples written there; a bin resets when the write
    /// head first enters it. Published to the viz at block rate for
    /// the editor's backdrop silhouette.
    pub(super) peak_bins: [f32; PEAK_BINS],
    /// `log2(samples per peak bin)` — the ring length and bin count are
    /// both powers of two, so binning is a shift + mask.
    pub(super) peak_shift: u32,
    /// Bin the write head last accumulated into (`usize::MAX` = none).
    peak_last_bin: usize,
}

impl SourceRing {
    pub(super) fn new(ring_len: usize) -> Self {
        Self {
            buf_l: vec![0.0; ring_len],
            buf_r: vec![0.0; ring_len],
            mask: ring_len - 1,
            write_pos: 0,
            freeze_xf: 0.0,
            peak_bins: [0.0; PEAK_BINS],
            peak_shift: (ring_len / PEAK_BINS).max(1).trailing_zeros(),
            peak_last_bin: usize::MAX,
        }
    }

    pub(super) fn clear(&mut self) {
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.write_pos = 0;
        self.freeze_xf = 0.0;
        self.peak_bins = [0.0; PEAK_BINS];
        self.peak_last_bin = usize::MAX;
    }

    /// Ring-buffer length in seconds (>= [`MAX_DELAY_SECONDS`]).
    pub(super) fn buffer_seconds(&self, sample_rate: f32) -> f32 {
        self.buf_l.len() as f32 / sample_rate
    }

    /// Write into the circular buffer (ba todo #1074): the dry input,
    /// plus — on the Wet→Buffer route — the conditioned wet bus of the
    /// previous block, so each recirculation is re-granulated. The
    /// Output-only route keeps the buffer clean. Returns the number of
    /// samples actually written (the head advance).
    ///
    /// Freeze (ba todo #1075) gates this whole write — dry *and*
    /// feedback, so a frozen buffer cannot run away no matter the
    /// loop gain. `freeze_xf` ramps per sample; while in between the
    /// write is an equal-power blend of the held content and the
    /// incoming signal (the engage ramp thereby morphs the recorded
    /// stream into the lap-old material ahead of the stop point, and
    /// resume morphs back out of it, so the boundary is always
    /// splice-free), and once fully frozen the write head stops
    /// (samples are neither written nor consumed for head advance)
    /// and the buffer is left bit-untouched. Because the head is
    /// static, grain read origins (`write_pos - delay`) become
    /// absolute buffer offsets — grains do not chase a stopped head.
    ///
    /// `track_in` receives the dry mid of exactly the samples written,
    /// for the pitch tracker (ba todo #1082).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn write_input(
        &mut self,
        left: &[f32],
        right: &[f32],
        frames: usize,
        sample_rate: f32,
        wet_to_buffer: bool,
        params: &BlockParams,
        feedback: &FeedbackStage,
        track_in: &mut [f32],
    ) -> usize {
        let base = self.write_pos as usize;
        let freeze_target: f32 = if params.freeze { 1.0 } else { 0.0 };
        let freeze_step = 1.0 / (FREEZE_RAMP_SECONDS * sample_rate).max(1.0);
        let mut advanced = 0usize;
        for i in 0..frames {
            if self.freeze_xf < freeze_target {
                self.freeze_xf = (self.freeze_xf + freeze_step).min(1.0);
            } else if self.freeze_xf > freeze_target {
                self.freeze_xf = (self.freeze_xf - freeze_step).max(0.0);
            }
            if self.freeze_xf >= 1.0 {
                continue; // fully frozen: hold the buffer, stop the head
            }
            let idx = (base + advanced) & self.mask;
            let (mut in_l, mut in_r) = (left[i], right[i]);
            if params.pitch_sync {
                // The tracker is fed the *dry* mid of exactly the
                // samples written, so its markers map 1:1 onto
                // write-stream positions (ba todo #1082).
                track_in[advanced] = 0.5 * (left[i] + right[i]);
            }
            if wet_to_buffer && i < feedback.bus_len {
                in_l += feedback.bus_l[i];
                in_r += feedback.bus_r[i];
            }
            if self.freeze_xf > 0.0 {
                // Engage/resume ramp: equal-power blend of held content
                // and the incoming stream, click-free at both ends.
                let phase = std::f32::consts::FRAC_PI_2 * self.freeze_xf;
                let (keep_g, write_g) = phase.sin_cos();
                self.buf_l[idx] = self.buf_l[idx] * keep_g + in_l * write_g;
                self.buf_r[idx] = self.buf_r[idx] * keep_g + in_r * write_g;
            } else {
                self.buf_l[idx] = in_l;
                self.buf_r[idx] = in_r;
            }
            // Coarse peak mip of the written content (ba todo #1135):
            // shift + mask binning, bin reset on head entry. While
            // frozen nothing is written, so the backdrop holds.
            let bin = ((base + advanced) >> self.peak_shift) & (PEAK_BINS - 1);
            if bin != self.peak_last_bin {
                self.peak_bins[bin] = 0.0;
                self.peak_last_bin = bin;
            }
            let mag = self.buf_l[idx].abs().max(self.buf_r[idx].abs());
            if mag > self.peak_bins[bin] {
                self.peak_bins[bin] = mag;
            }
            advanced += 1;
        }
        advanced
    }
}
