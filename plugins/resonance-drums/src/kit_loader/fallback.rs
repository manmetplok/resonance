//! The built-in kit's pads, from the samples embedded in the plugin.
//!
//! The built-in kit plays only when no kit is selected, or the selected
//! kit is missing altogether (drums-plugin-rework.md D7): a kit that lacks
//! a piece leaves that pad silent rather than filling it from here
//! ([`crate::pad_map`]).

use std::path::PathBuf;

use crate::drum_map::PadMapping;
use crate::kit::{decode_sample, LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};

use super::cache::{self, SampleKey, Source};

/// The built-in pad for `mapping` at `target_sr`. Its sample comes
/// through the shared cache like any file's, keyed by the embedded
/// bytes, so every instance (and every kit) holds one copy.
#[doc(hidden)]
pub fn build_fallback_pad(mapping: &PadMapping, target_sr: f32) -> Result<LoadedPad, String> {
    build_fallback_pad_sourced(mapping, target_sr).map(|(pad, _)| pad)
}

/// [`build_fallback_pad`], and whether the cache already held its take
/// (another instance, or this one) or decoded it now.
#[doc(hidden)]
pub fn build_fallback_pad_sourced(
    mapping: &PadMapping,
    target_sr: f32,
) -> Result<(LoadedPad, Source), String> {
    let bytes = mapping.default_sample;
    // Keyed by the embedded bytes' address. That is only an identity
    // within one load of this library — but so is the cache: it is a
    // static of the same `.so`, so it dies with it, and no key can
    // outlive the address it names.
    let key = SampleKey {
        path: PathBuf::from(format!("builtin:{:p}", bytes.as_ptr())),
        modified: None,
        len: bytes.len() as u64,
        rate_bits: target_sr.to_bits(),
        // Embedded bytes have no file to stream from.
        preload: 0,
    };
    let (data, source) = cache::global()
        .get_or_insert_with(key, || decode_sample(bytes.to_vec(), target_sr))
        .map_err(|e| format!("decode embedded {}: {e}", mapping.name))?;
    let sample = LoadedSample::from_shared(data);
    Ok((build_pad(mapping, sample), source))
}

fn build_pad(mapping: &PadMapping, sample: LoadedSample) -> LoadedPad {
    LoadedPad {
        name: mapping.name.to_string(),
        choke_group: mapping.choke_group,
        output_group: mapping.output_group,
        close_mics: vec![LoadedMicBank {
            position: "fallback".to_string(),
            setup_key: String::new(),
            layers: vec![VelocityLayer::new(vec![sample])],
        }],
        extra_banks: Vec::new(),
        overhead: None,
    }
}
