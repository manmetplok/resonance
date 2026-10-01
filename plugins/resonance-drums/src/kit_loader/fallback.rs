//! Fallback pad construction.
//!
//! Used when a manifest piece can't be loaded (or doesn't exist) so the
//! pad still produces sound from the embedded default sample. Also used
//! by the embedded "no-manifest" path for Clap / Cowbell.

use std::path::PathBuf;

use crate::drum_map::PadMapping;
use crate::kit::{decode_sample, LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};

use super::cache::{self, SampleKey};

/// The built-in pad for `mapping` at `target_sr`. Its sample comes
/// through the shared cache like any file's, keyed by the embedded
/// bytes, so every instance (and every kit) holds one copy.
#[doc(hidden)]
pub fn build_fallback_pad(mapping: &PadMapping, target_sr: f32) -> Result<LoadedPad, String> {
    let bytes = mapping.default_sample;
    let key = SampleKey {
        path: PathBuf::from(format!("builtin:{:p}", bytes.as_ptr())),
        modified: None,
        len: bytes.len() as u64,
        rate_bits: target_sr.to_bits(),
    };
    let (data, _) = cache::global()
        .get_or_insert_with(key, || decode_sample(bytes.to_vec(), target_sr))
        .map_err(|e| format!("decode embedded {}: {e}", mapping.name))?;
    let sample = LoadedSample::from_shared(data);
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
