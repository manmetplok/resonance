//! Whole-frame pushes into the recording ring buffer.
//!
//! Not part of the mix at all — the *capture* callbacks
//! (`platform`, `input_pipewire`) call this on their side of the ring —
//! but it shares the mixer's whole-frame discipline: a partial frame left
//! behind by an overflow permanently rotates the channel interleave of
//! everything that follows it.

/// Largest sample count ≤ both `len` and `vacant` that is a whole
/// number of `frame_stride`-sample frames. Producers push exactly this
/// many samples so a full ring can't rotate the channel interleave.
#[inline]
pub fn whole_frame_push_len(len: usize, vacant: usize, frame_stride: usize) -> usize {
    len.min(vacant) / frame_stride * frame_stride
}

/// Push interleaved capture samples into the recording ring in whole
/// frames only — like the monitor path — so an overflow can never leave
/// a partial frame behind and permanently rotate the take's channel
/// alignment (doc #260 finding #17). Returns the number of frames that
/// were dropped (0 = everything landed) so the caller can add them to
/// the shared overflow counter and the damage can be reported
/// quantitatively (`AudioEvent::RecordingOverflow`).
#[inline]
pub fn push_recording_frames(
    prod: &mut ringbuf::HeapProd<f32>,
    samples: &[f32],
    frame_stride: usize,
) -> usize {
    use ringbuf::traits::{Observer, Producer};
    let stride = frame_stride.max(1);
    let take = whole_frame_push_len(samples.len(), prod.vacant_len(), stride);
    let _ = prod.push_slice(&samples[..take]);
    (samples.len() - take).div_ceil(stride)
}
