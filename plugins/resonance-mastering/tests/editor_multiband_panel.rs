//! Layout and readouts of the multiband stage panel (ba todo #1317).
//!
//! The band controls went from three knobs to seven. What is pinned
//! here is that they still fit: four fixed-width band columns inside the
//! window, two knob lines per column inside the panel the tab asks for.
//! Get either wrong and the new controls exist but are clipped off the
//! bottom or pushed off the right — reachable in the host, unreachable
//! in the window, which is the exact failure the audit was about.

#![cfg(feature = "editor")]

use resonance_mastering::editor::controls::multiband::{
    BANDS_W, BAND_COLUMN_W, BAND_NAMES, REQUIRED_PANEL_H,
};
use resonance_mastering::editor::controls::{stage_panel_height, StageTab};
use resonance_mastering::editor::WINDOW_W;
use resonance_mastering::params::MasteringParams;
use resonance_mastering::stages::multiband::NUM_BANDS;
use resonance_plugin::Param;

#[test]
fn the_four_band_columns_fit_the_window() {
    assert!(
        BANDS_W <= WINDOW_W as f32,
        "band columns need {BANDS_W} px, window is {WINDOW_W} px"
    );
    // …and are wide enough for the four-knob line they have to hold.
    const { assert!(BAND_COLUMN_W >= 4.0 * 64.0) };
    assert_eq!(BAND_NAMES.len(), NUM_BANDS);
}

#[test]
fn the_multiband_tab_gets_room_for_two_knob_lines() {
    let h = stage_panel_height(StageTab::Multiband);
    assert!(
        h >= REQUIRED_PANEL_H,
        "multiband panel is {h} px, needs {REQUIRED_PANEL_H} px"
    );
    // The other stages are single-line and must not have grown.
    assert_eq!(stage_panel_height(StageTab::Glue), 260.0);
    assert_eq!(stage_panel_height(StageTab::Limiter), 260.0);
    // Nor may the panel eat the window: header + tabs + histories +
    // controls has to leave room for the spectrum.
    for tab in [
        StageTab::Assistant,
        StageTab::Multiband,
        StageTab::CorrectiveEq,
    ] {
        assert!(
            40.0 + 32.0 + 150.0 + stage_panel_height(tab) < 820.0,
            "{tab:?} panel leaves no room for the spectrum"
        );
    }
}

/// The knobs print `param.display(..)`, so what the panel shows is
/// whatever the parameter says — including the unit. No knob may render
/// a bare number.
#[test]
fn knob_readouts_carry_their_unit() {
    let params = MasteringParams::default();
    let mb = &params.multiband;
    assert_eq!(mb.xo1.display(120.0), "120 Hz");
    let band = &mb.bands[0];
    assert_eq!(band.threshold.display(-18.0), "-18.0 dB");
    assert_eq!(band.ratio.display(2.0), "2.0:1");
    assert_eq!(band.attack.display(30.0), "30.0 ms");
    assert_eq!(band.release.display(150.0), "150 ms");
    assert_eq!(band.knee.display(6.0), "6.0 dB");
    assert_eq!(band.mix.display(1.0), "100%");
    assert_eq!(band.gain.display(-3.0), "-3.0 dB");
}
