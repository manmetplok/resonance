//! Write → read round-trips through the `DelayViz` public API the audio
//! thread and the editor share.

use resonance_delay::viz::{echo_taps, DelayViz, MAX_ECHO_TAPS};

#[test]
fn fresh_viz_reads_as_silence() {
    let viz = DelayViz::new();
    assert_eq!(
        viz.read_in_peaks_db(),
        (f32::NEG_INFINITY, f32::NEG_INFINITY)
    );
    assert_eq!(
        viz.read_out_peaks_db(),
        (f32::NEG_INFINITY, f32::NEG_INFINITY)
    );
    let (_, levels_l, _, levels_r) = viz.read_echo_taps();
    assert!(levels_l.iter().all(|&v| v == f32::NEG_INFINITY));
    assert!(levels_r.iter().all(|&v| v == f32::NEG_INFINITY));
}

#[test]
fn scalars_round_trip() {
    let viz = DelayViz::new();
    viz.store_peaks(-3.0, -6.0, -9.0, -12.0);
    assert_eq!(viz.read_in_peaks_db(), (-3.0, -6.0));
    assert_eq!(viz.read_out_peaks_db(), (-9.0, -12.0));

    viz.store_delay_time_ms(375.0);
    assert_eq!(viz.read_delay_time_ms(), 375.0);
    viz.store_bpm(128.0);
    assert_eq!(viz.read_bpm(), 128.0);
}

#[test]
fn echo_taps_round_trip() {
    let viz = DelayViz::new();
    let taps = echo_taps(250.0, 375.0, 0.5, 0);
    viz.store_taps(&taps);
    let (times_l, levels_l, times_r, levels_r) = viz.read_echo_taps();
    for i in 0..MAX_ECHO_TAPS {
        assert_eq!(times_l[i], taps.times_l[i], "times_l[{i}]");
        assert_eq!(levels_l[i], taps.levels_l[i], "levels_l[{i}]");
        assert_eq!(times_r[i], taps.times_r[i], "times_r[{i}]");
        assert_eq!(levels_r[i], taps.levels_r[i], "levels_r[{i}]");
    }
}
