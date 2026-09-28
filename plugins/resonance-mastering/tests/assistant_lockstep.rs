//! The assistant's engine lives in `resonance-mastering-assist`, which
//! knows the plugin's params only as string keys and index values. Pin
//! them to the real params here: every key it can emit resolves through
//! `param_by_key`, and the values it writes for choices are the plugin's
//! indices.

use resonance_dsp::BandType;
use resonance_mastering::assistant::decide::{param_by_key, EMITTED_PARAM_KEYS, MS_STEREO_INDEX};
use resonance_mastering::params::MasteringParams;
use resonance_mastering::stages::linear_phase_eq::MsMode;

#[test]
fn every_key_the_engine_emits_is_a_mastering_param() {
    let params = MasteringParams::default();
    let missing: Vec<_> = EMITTED_PARAM_KEYS
        .iter()
        .filter(|k| param_by_key(&params, k).is_none())
        .collect();
    assert!(missing.is_empty(), "not mastering params: {missing:?}");
}

/// The shelf writes read back, through the tonal EQ's own snapshot, as
/// the band type and M/S mode the engine meant.
#[test]
fn the_engine_writes_the_plugins_choice_indices() {
    assert_eq!(MS_STEREO_INDEX, MsMode::Stereo.to_index() as f32);
    let params = MasteringParams::default();
    for (band, band_type) in [(0, BandType::LowShelf), (3, BandType::HighShelf)] {
        let set = |suffix: &str, v: f32| {
            let key = format!("tone_b{band}_{suffix}");
            param_by_key(&params, &key).unwrap().set_plain(f64::from(v));
        };
        set("ms", MsMode::Side.to_index() as f32);
        set("type", band_type.to_index() as f32);
        set("ms", MS_STEREO_INDEX);
        let b = params.tonal_eq.snapshot()[band];
        assert_eq!(b.band_type, band_type, "band {band}");
        assert_eq!(b.ms, MsMode::Stereo, "band {band}");
    }
}
