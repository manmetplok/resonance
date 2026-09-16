//! Write → read round-trips through the `IrViz` public API the audio
//! thread, the loader thread and the editor share.

use resonance_ir::viz::{IrSnapshot, IrViz};

#[test]
fn peaks_round_trip() {
    let viz = IrViz::new();
    assert_eq!(
        viz.read_in_peaks_db(),
        (f32::NEG_INFINITY, f32::NEG_INFINITY)
    );
    viz.store_peaks(-1.0, -2.0, -3.0, -4.0);
    assert_eq!(viz.read_in_peaks_db(), (-1.0, -2.0));
    assert_eq!(viz.read_out_peaks_db(), (-3.0, -4.0));
}

#[test]
fn engine_block_is_none_until_initialized() {
    let viz = IrViz::new();
    assert!(viz.engine_block().is_none());
    viz.store_engine_block(512, 48_000.0);
    assert_eq!(viz.engine_block(), Some((512, 48_000.0)));
}

#[test]
fn ir_snapshot_hand_off() {
    let viz = IrViz::new();
    assert!(viz.snapshot().is_none());

    let mut snap = IrSnapshot::empty();
    snap.wave_len = 7;
    snap.wave_left[0] = 0.25;
    viz.store_snapshot(snap);

    let got = viz.snapshot().expect("snapshot after store");
    assert_eq!(got.wave_len, 7);
    assert_eq!(got.wave_left[0], 0.25);

    viz.clear_snapshot();
    assert!(viz.snapshot().is_none());
}
