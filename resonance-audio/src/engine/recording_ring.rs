//! How big the recording ring is: the safety margin between the input
//! callback (producer) and the engine control thread's drain-to-WAV loop
//! (consumer).

/// Seconds of capture the recording ring holds: the safety margin
/// between the input callback (producer) and the engine control thread's
/// drain-to-WAV loop (consumer). The engine thread wakes at ~60 Hz but
/// also runs every blocking command handler, so the margin is generous.
pub const RECORDING_RING_SECONDS: usize = 10;

/// The recording ring's length in samples for a capture stream of
/// `input_channels` interleaved channels at `sample_rate`: the ring holds
/// whole frames, so it is sized in frames × channels (code review RT-17 —
/// a fixed sample count held 20 s of stereo but ~2 s of an 18-in
/// interface). Never below the historical 10 s of 96 kHz stereo.
/// Allocated by the engine thread when the stream is set up, never on
/// the audio thread.
pub fn recording_ring_len(sample_rate: u32, input_channels: u16) -> usize {
    const FLOOR: usize = 96_000 * 2 * RECORDING_RING_SECONDS;
    let per_second = sample_rate.max(1) as usize * input_channels.max(1) as usize;
    (per_second * RECORDING_RING_SECONDS).max(FLOOR)
}
