//! `SlimmableContainer` files and slimmable packed weights.
//!
//! Two related A2 shapes live here: a container file, which holds complete
//! submodels at several sizes and picks one at load time, and a plain
//! WaveNet whose layer arrays carry `slimmable` descriptors, whose weight
//! vector is packed and must be sliced before construction.

use super::super::wavenet::slimmable;
use super::super::NamInference;
use super::config::parse_full_wavenet_config;
use super::schema::{parse_sample_rate, ContainerConfigFile, SubmodelFile};
use super::weights::build_model;

/// Raw-JSON probe: does any layer array of a (new-format) WaveNet config
/// carry a non-null `slimmable` descriptor? Old flat configs have no
/// `layers` array and are never slimmable; non-slimmable files must keep
/// taking the historical load path bit-for-bit, so this never rejects — the
/// typed parse validates the descriptors once a file IS slimmable.
pub(super) fn wavenet_config_is_slimmable(config: &serde_json::Value) -> bool {
    config
        .get("layers")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|layers| {
            layers
                .iter()
                .any(|l| l.get("slimmable").is_some_and(|s| !s.is_null()))
        })
}

/// Verify a slimmable WaveNet's packed weight vector against the full-size
/// layout, before a single weight is consumed.
///
/// The typed parse comes first because it validates every slimmable
/// descriptor — unknown methods, unsorted allowed_channels, last entry !=
/// channels — with clear errors (see `params::SlimmableParams`).
pub(super) fn verify_full_size_slice(
    config: &serde_json::Value,
    weights: &[f32],
) -> Result<(), String> {
    let full = parse_full_wavenet_config(config)?;
    if full.head.is_some() {
        return Err("Slimmable WaveNet: a post-stack head is not supported".to_string());
    }
    let target = slimmable::channels_for_size(&full.layer_arrays, slimmable::FULL_SIZE);
    // v1 always selects the full size, and full allowed_channels lists end
    // at the full channel count (validated), so the slice is the whole
    // packed vector and the weights flow to construction unchanged. Smaller
    // sizes additionally need `derive_params_for_channels` + engine-config
    // rederivation when runtime A2-Lite size selection lands (follow-on
    // todo).
    if !slimmable::is_full_size(&full.layer_arrays, &target) {
        return Err(
            "Slimmable WaveNet: only the full-size slice is supported (runtime A2-Lite size selection is not implemented yet)"
                .to_string(),
        );
    }
    // The full-size extraction walk verifies the packed vector against the
    // full-size layout exactly (every tensor, head_scale included) and
    // returns it bit-identically.
    let slice = slimmable::extract_slimmed_weights(&full.layer_arrays, weights, &target)?;
    debug_assert_eq!(slice, weights, "full-size slice must be the identity");
    Ok(())
}

/// Reference `ContainerModel::_get_index_for_slimmable_size`: the first
/// submodel whose `max_value` exceeds the requested size; the last submodel
/// is the fallback (and the load-time default, size = 1.0 -> full).
fn container_submodel_index(max_values: &[f64], size: f64) -> usize {
    max_values
        .iter()
        .position(|&mv| size < mv)
        .unwrap_or(max_values.len() - 1)
}

/// Build an A2 slimmable container (e.g. the A2.nam trainer export): one
/// file holding complete submodels at different sizes (reference
/// NAM/container.cpp). The top-level weight vector is unused — every
/// submodel carries its own. v1 constructs the FULL (last) submodel only;
/// the descriptor validation and index selection mirror the reference so
/// runtime size selection can pick another submodel later without reshaping
/// this path.
pub(super) fn build_container(
    config: serde_json::Value,
    sample_rate: f32,
) -> Result<Box<dyn NamInference>, String> {
    let cfg: ContainerConfigFile = serde_json::from_value(config)
        .map_err(|e| format!("Invalid SlimmableContainer config: {e}"))?;
    if cfg.submodels.is_empty() {
        return Err("SlimmableContainer: 'submodels' must be a non-empty array".to_string());
    }
    for pair in cfg.submodels.windows(2) {
        if pair[1].max_value <= pair[0].max_value {
            return Err(
                "SlimmableContainer: submodels must be sorted by ascending max_value".to_string(),
            );
        }
    }
    let max_values: Vec<f64> = cfg.submodels.iter().map(|s| s.max_value).collect();
    if *max_values.last().expect("non-empty") < 1.0 {
        return Err("SlimmableContainer: last submodel max_value must be >= 1.0".to_string());
    }
    let index = container_submodel_index(&max_values, slimmable::FULL_SIZE);
    let entry = cfg
        .submodels
        .into_iter()
        .nth(index)
        .expect("index selected from the same list");
    let sub: SubmodelFile = serde_json::from_value(entry.model)
        .map_err(|e| format!("SlimmableContainer: invalid submodel {index}: {e}"))?;
    // Reference ContainerModel ctor: a submodel trained at a different rate
    // than the container is a broken export (an absent submodel rate counts
    // as unknown and skips the check).
    if let Some(v) = &sub.sample_rate {
        let sub_rate = parse_sample_rate(Some(v));
        if sub_rate != sample_rate {
            return Err(format!(
                "SlimmableContainer: submodel {index} sample rate ({sub_rate}) doesn't match container sample rate ({sample_rate})"
            ));
        }
    }
    build_model(
        &sub.architecture,
        sub.config,
        &sub.weights,
        sample_rate,
        true,
        // Containers only exist in the A2 era: submodels always run with
        // the exact activation flavor.
        true,
    )
}
