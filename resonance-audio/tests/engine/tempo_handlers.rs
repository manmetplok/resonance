//! The engine's `SetBpm` handler goes through the shared tempo-legality
//! rule (`sanitize_bpm`), not a private clamp (code review FU-D3).

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::types::{MAX_BPM, MIN_BPM};

#[test]
fn set_bpm_clamps_into_the_shared_range() {
    let mut h = EngineHandlerHarness::new();
    h.set_bpm(999.0);
    assert_eq!(h.published_bpm(), MAX_BPM);
    h.set_bpm(1.0);
    assert_eq!(h.published_bpm(), MIN_BPM);
    h.set_bpm(133.0);
    assert_eq!(h.published_bpm(), 133.0);
}

#[test]
fn set_bpm_ignores_non_finite_tempos() {
    let mut h = EngineHandlerHarness::new();
    h.set_bpm(97.0);
    h.set_bpm(f32::NAN);
    assert_eq!(h.published_bpm(), 97.0);
    h.set_bpm(f32::INFINITY);
    assert_eq!(h.published_bpm(), 97.0);
}
