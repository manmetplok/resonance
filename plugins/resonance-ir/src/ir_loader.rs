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
pub fn load_ir_from_bytes(data: &[u8], target_sample_rate: f32) -> Result<IrData, String> {
    let channels = resonance_common::decode_wav_channels(data, target_sample_rate)?;
    Ok(IrData {
        left: channels.left,
        right: channels.right,
        stereo: channels.stereo,
    })
}
