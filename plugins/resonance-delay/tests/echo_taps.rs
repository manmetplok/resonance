//! The echo-tap viz levels are accumulated multiplicatively
//! (`fb_gain *= fb` per tap) instead of `fb.powf(n)` per tap. Pin the
//! equivalence: tap n must still sit at `fb^n`, within float tolerance.

use resonance_delay::viz::MAX_ECHO_TAPS;
use resonance_delay::ResonanceDelay;
use resonance_dsp::linear_to_db;
use resonance_plugin::{EventIterator, OutputBuffer, Param, ResonancePlugin};

#[test]
fn echo_tap_levels_match_powf_reference() {
    let fb = 0.6f32;
    let mut plugin = ResonanceDelay::new();
    plugin.params.feedback.set_value(fb);
    plugin.params.sync.set_plain(0.0); // free-running
    plugin.initialize(48_000.0, 512);

    let mut left = [0.0f32; 512];
    let mut right = [0.0f32; 512];
    left[0] = 1.0;
    right[0] = 1.0;
    let mut outs = [OutputBuffer {
        left: &mut left,
        right: &mut right,
    }];
    let mut ev = EventIterator::empty();
    plugin.process(&mut outs, 512, &mut ev, None);

    let (_, levels_l, _, levels_r) = plugin.viz().read_echo_taps();
    for tap in 0..MAX_ECHO_TAPS {
        let expected = linear_to_db(fb.powf((tap + 1) as f32));
        assert!(
            (levels_l[tap] - expected).abs() < 1e-3,
            "tap {tap}: got {} dB, powf reference {} dB",
            levels_l[tap],
            expected
        );
        assert_eq!(levels_l[tap], levels_r[tap], "L/R mismatch at tap {tap}");
    }
    // Geometric decay: constant dB decrement from tap to tap.
    let step = levels_l[1] - levels_l[0];
    for tap in 1..MAX_ECHO_TAPS {
        let d = levels_l[tap] - levels_l[tap - 1];
        assert!((d - step).abs() < 1e-3, "non-constant decay at tap {tap}");
    }
}
