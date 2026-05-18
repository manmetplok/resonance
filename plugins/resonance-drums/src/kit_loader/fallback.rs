//! Fallback pad construction.
//!
//! Used when a manifest piece can't be loaded (or doesn't exist) so the
//! pad still produces sound from the embedded default sample. Also used
//! by the embedded "no-manifest" path for Clap / Cowbell.

use crate::drum_map::PadMapping;
use crate::kit::{decode_wav, LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};

#[doc(hidden)]
pub fn build_fallback_pad(mapping: &PadMapping, target_sr: f32) -> Result<LoadedPad, String> {
    let data = decode_wav(mapping.default_sample, target_sr)
        .map_err(|e| format!("decode embedded {}: {e}", mapping.name))?;
    let sample = LoadedSample::from_data(data);
    Ok(LoadedPad {
        name: mapping.name.to_string(),
        choke_group: mapping.choke_group,
        output_group: mapping.output_group,
        close_mics: vec![LoadedMicBank {
            position: "fallback".to_string(),
            setup_key: String::new(),
            layers: vec![VelocityLayer {
                round_robins: vec![sample],
            }],
        }],
        overhead: None,
    })
}
