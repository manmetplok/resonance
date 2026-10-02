/// IR (Impulse Response) WAV file loading and resampling.
use std::path::Path;

/// A loaded impulse response: one or two channels of f32 samples.
pub struct IrData {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    pub stereo: bool,
}

/// Load an IR from a WAV file, resampled to the target sample rate.
pub fn load_ir(path: &str, target_sample_rate: f32) -> Result<IrData, String> {
    let data = std::fs::read(Path::new(path)).map_err(|e| format!("Failed to read file: {e}"))?;
    load_ir_from_bytes(&data, target_sample_rate)
}

/// Load an IR from WAV bytes.
///
/// A resampled IR is rescaled by `source_rate / target_rate` (DSP2-01).
/// The shared resampler keeps unit DC gain per *sample*, which is right
/// for audio, but a convolution's gain is the *sum* of its taps, and
/// resampling changes the tap count by `target / source`. Without the
/// rescale a 96 kHz IR plays 6 dB quiet at 48 kHz, and a 48 kHz IR 6 dB
/// hot at 96 kHz. The decoder skips resampling within 1 Hz, and so does
/// the rescale.
pub fn load_ir_from_bytes(data: &[u8], target_sample_rate: f32) -> Result<IrData, String> {
    let mut channels = resonance_common::decode_wav_channels(data, target_sample_rate)
        .map_err(|e| e.to_string())?;
    let source_rate = channels.source_rate;
    if (source_rate - target_sample_rate).abs() > 1.0 && target_sample_rate > 0.0 {
        let scale = source_rate / target_sample_rate;
        for v in channels.left.iter_mut().chain(channels.right.iter_mut()) {
            *v *= scale;
        }
    }
    Ok(IrData {
        left: channels.left,
        right: channels.right,
        stereo: channels.stereo,
    })
}
