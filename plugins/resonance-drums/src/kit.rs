/// Drum kit data model: a pad is a grid of (velocity layer, round robin)
/// samples. Single-sample embedded defaults are represented as a pad with
/// one layer containing one round robin.

/// A single decoded stereo sample ready for playback.
pub struct LoadedSample {
    /// Stereo interleaved f32 at the host sample rate.
    pub data: Vec<f32>,
    /// Number of stereo frames.
    pub frames: usize,
}

/// All round-robin takes recorded at a given velocity.
pub struct VelocityLayer {
    pub round_robins: Vec<LoadedSample>,
}

/// A loaded pad with velocity layers and round robins.
#[allow(dead_code)]
pub struct LoadedPad {
    pub note: u8,
    pub name: String,
    /// Velocity layers sorted soft → loud. Always at least one layer.
    pub layers: Vec<VelocityLayer>,
    pub choke_group: Option<u8>,
}

impl LoadedSample {
    pub fn from_data(data: Vec<f32>) -> Self {
        let frames = data.len() / 2;
        Self { data, frames }
    }
}

/// Decode a WAV file from a byte slice into stereo interleaved f32 samples,
/// resampled to the target sample rate if necessary.
pub fn decode_wav(data: &[u8], target_sample_rate: f32) -> Result<Vec<f32>, String> {
    resonance_common::decode_wav_stereo(data, target_sample_rate)
}
