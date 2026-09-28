//! Auto-gain keeps the output within ±0.5 LU of the input — the
//! loudness-confound fix (warmth-width-depth.md §1, §6.1) — on pink noise
//! and on a drum loop, in every mode, across the drive range.
//!
//! Integrated loudness (BS.1770, gated) of the output against the input,
//! over the material after a one-second lead-in (the followers' time
//! constant is 0.8 s). `mix` is 1, so the whole output is the matched wet
//! signal; everything else is at its default, oversampling included.

mod common;

use common::*;
use resonance_color::dsp::Settings;
use resonance_color::params::Mode;
use resonance_metering::LufsMeter;

/// One second of lead-in plus four bars of the 120 BPM loop.
const SECONDS: f32 = 9.0;
const LEAD_IN_S: f32 = 1.0;
const TOLERANCE_LU: f32 = 0.5;
const DRIVES: [f32; 4] = [0.0, 0.35, 0.7, 1.0];

fn lufs(l: &[f32], r: &[f32]) -> f32 {
    let skip = (LEAD_IN_S * SR) as usize;
    LufsMeter::analyze_offline(SR, &l[skip..], &r[skip..]).integrated
}

fn check(mode: Mode, signal: &str, l: &[f32], r: &[f32]) {
    let want = lufs(l, r);
    assert!(want.is_finite() && want > -40.0, "{signal} is too quiet: {want} LUFS");
    for drive in DRIVES {
        let s = Settings {
            mode,
            drive,
            mix: 1.0,
            auto_gain: true,
            ..Settings::default()
        };
        let (ol, or) = render(&s, l, r);
        let got = lufs(&ol, &or);
        let delta = got - want;
        eprintln!("{:12} {signal:6} drive {drive:.2}: in {want:.2} out {got:.2} LUFS (Δ {delta:+.2} LU)", mode.label());
        assert!(
            delta.abs() <= TOLERANCE_LU,
            "{} on {signal} at drive {drive}: output {got:.2} LUFS vs input {want:.2} \
             (Δ {delta:+.2} LU, allowed ±{TOLERANCE_LU})",
            mode.label()
        );
    }
}

fn pink() -> (Vec<f32>, Vec<f32>) {
    pink_noise((SECONDS * SR) as usize, -18.0, 11)
}

fn drums() -> (Vec<f32>, Vec<f32>) {
    drum_loop((SECONDS * SR) as usize, -6.0, 23)
}

macro_rules! auto_gain_case {
    ($name:ident, $mode:expr, $signal:ident) => {
        #[test]
        fn $name() {
            let (l, r) = $signal();
            check($mode, stringify!($signal), &l, &r);
        }
    };
}

auto_gain_case!(tube_pink, Mode::Tube, pink);
auto_gain_case!(tube_drums, Mode::Tube, drums);
auto_gain_case!(tape_pink, Mode::Tape, pink);
auto_gain_case!(tape_drums, Mode::Tape, drums);
auto_gain_case!(transformer_pink, Mode::Transformer, pink);
auto_gain_case!(transformer_drums, Mode::Transformer, drums);
auto_gain_case!(console_pink, Mode::Console, pink);
auto_gain_case!(console_drums, Mode::Console, drums);
auto_gain_case!(warm_pink, Mode::Warm, pink);
auto_gain_case!(warm_drums, Mode::Warm, drums);

/// With auto-gain off the drive law alone decides the level, and at high
/// drive it is far from matched — which is what makes the test above
/// mean something.
#[test]
fn without_auto_gain_high_drive_moves_the_loudness() {
    let (l, r) = drums();
    let want = lufs(&l, &r);
    let s = Settings {
        mode: Mode::Tube,
        drive: 1.0,
        auto_gain: false,
        ..Settings::default()
    };
    let (ol, or) = render(&s, &l, &r);
    let delta = lufs(&ol, &or) - want;
    assert!(delta.abs() > 2.0, "auto-gain off moved loudness only {delta:+.2} LU");
}
