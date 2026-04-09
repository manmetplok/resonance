/// Drum kit definition and loaded pad data.

/// A loaded pad with its decoded audio data.
#[allow(dead_code)]
pub struct LoadedPad {
    pub note: u8,
    pub name: String,
    /// Stereo interleaved f32 sample data.
    pub sample_data: Vec<f32>,
    /// Number of stereo frames.
    pub sample_frames: usize,
    pub choke_group: Option<u8>,
}

/// Decode a WAV file from a byte slice into stereo interleaved f32 samples,
/// resampled to the target sample rate if necessary.
pub fn decode_wav(data: &[u8], target_sample_rate: f32) -> Result<Vec<f32>, String> {
    resonance_common::decode_wav_stereo(data, target_sample_rate)
}
