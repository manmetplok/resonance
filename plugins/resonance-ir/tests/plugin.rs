use resonance_ir::ResonanceIr;
use resonance_plugin::ResonancePlugin;

/// Full save_state → load_state round-trip preserves the persisted IR
/// path. Exercises the trait-default `save_state` / `load_state` that
/// the CLAP bridge calls on the owned plugin instance.
#[test]
fn state_roundtrip_preserves_ir_path() {
    let src = ResonanceIr::new();
    *src.params.ir_path.lock() = "/some/cabs/resonance_cab.wav".to_string();

    let bytes = src.save_state();

    let mut dst = ResonanceIr::new();
    assert!(dst.load_state(&bytes), "load_state should succeed");
    assert_eq!(
        dst.params.ir_path.lock().clone(),
        "/some/cabs/resonance_cab.wav"
    );
}
