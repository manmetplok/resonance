//! DSP2-10: Hi Cut, Lo Cut and Drive shape the wet signal itself, not
//! only the recirculation. At feedback 0 there is a single echo, and
//! all three controls must still act on it.

use resonance_delay::ResonanceDelay;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
const BLOCK: usize = 480;

fn single_echo_plugin(hi_cut: f32, lo_cut: f32, drive: f32) -> ResonanceDelay {
    let plugin = ResonanceDelay::new();
    plugin.params.sync.set_value(false);
    plugin.params.time_ms.set_value(50.0);
    plugin.params.feedback.set_value(0.0);
    plugin.params.mix.set_value(1.0);
    plugin.params.hi_cut.set_value(hi_cut);
    plugin.params.lo_cut.set_value(lo_cut);
    plugin.params.drive.set_value(drive);
    plugin.params.mod_depth.set_value(0.0);
    plugin
}

/// Steady-state wet RMS of a sine at `hz`, amplitude `amp`.
fn wet_rms(mut plugin: ResonanceDelay, hz: f32, amp: f32) -> f32 {
    plugin.initialize(SR, BLOCK as u32);
    let mut n = 0usize;
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for block in 0..60 {
        let mut left = [0.0f32; BLOCK];
        for s in left.iter_mut() {
            *s = amp * (std::f32::consts::TAU * hz * n as f32 / SR).sin();
            n += 1;
        }
        let mut right = left;
        let mut outs = [OutputBuffer {
            left: &mut left[..],
            right: &mut right[..],
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, BLOCK, &mut ev, None);
        if block >= 20 {
            for &v in left.iter() {
                sum += (v as f64) * (v as f64);
                count += 1;
            }
        }
    }
    (sum / count as f64).sqrt() as f32
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

#[test]
fn lo_cut_filters_the_first_echo() {
    let open = wet_rms(single_echo_plugin(20_000.0, 20.0, 0.0), 60.0, 0.1);
    let cut = wet_rms(single_echo_plugin(20_000.0, 600.0, 0.0), 60.0, 0.1);
    assert!(
        db(cut) < db(open) - 20.0,
        "a 600 Hz lo cut left a 60 Hz echo at {:.1} dB vs {:.1} dB open",
        db(cut),
        db(open)
    );
}

#[test]
fn hi_cut_filters_the_first_echo() {
    let open = wet_rms(single_echo_plugin(20_000.0, 20.0, 0.0), 8_000.0, 0.1);
    let cut = wet_rms(single_echo_plugin(500.0, 20.0, 0.0), 8_000.0, 0.1);
    assert!(
        db(cut) < db(open) - 15.0,
        "a 500 Hz hi cut left an 8 kHz echo at {:.1} dB vs {:.1} dB open",
        db(cut),
        db(open)
    );
}

#[test]
fn drive_saturates_the_first_echo() {
    let clean = wet_rms(single_echo_plugin(20_000.0, 20.0, 0.0), 220.0, 0.9);
    let driven = wet_rms(single_echo_plugin(20_000.0, 20.0, 1.0), 220.0, 0.9);
    assert!(
        db(driven) < db(clean) - 1.0,
        "full drive did not compress a loud echo: {:.2} dB vs {:.2} dB clean",
        db(driven),
        db(clean)
    );
}
