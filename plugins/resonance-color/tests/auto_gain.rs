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
use resonance_color::ResonanceColor;
use resonance_metering::LufsMeter;
use resonance_plugin::ResonancePlugin;

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

// ---------------------------------------------------------------------------
// Onsets: after silence and after a reset
// ---------------------------------------------------------------------------

/// One bar of the 120 BPM loop (2 s), tiled so every bar is the same
/// audio and a segment can be compared with the same content elsewhere.
const BAR: usize = 96_000;
/// The window judged after an onset.
const ONSET_WINDOW: usize = (0.4 * SR) as usize;
const ONSET_TOLERANCE_DB: f32 = 1.0;
const ONSET_DRIVES: [f32; 2] = [0.7, 1.0];

fn tiled_bars(bar: &(Vec<f32>, Vec<f32>), bars: usize) -> (Vec<f32>, Vec<f32>) {
    let l = bar.0.iter().copied().cycle().take(bars * BAR).collect();
    let r = bar.1.iter().copied().cycle().take(bars * BAR).collect();
    (l, r)
}

fn peak_db(l: &[f32], r: &[f32]) -> f32 {
    let p = l.iter().chain(r).fold(0.0f32, |m, v| m.max(v.abs()));
    20.0 * p.max(1e-12).log10()
}

fn rms_db(l: &[f32], r: &[f32]) -> f32 {
    let e: f64 = l.iter().chain(r).map(|v| (*v as f64).powi(2)).sum();
    (10.0 * (e / (l.len() + r.len()) as f64).max(1e-24).log10()) as f32
}

/// The output over `ONSET_WINDOW` from `at` against the same audio at
/// `steady`: the peak, the 400 ms (momentary-length) level and every
/// 100 ms slice's level must all sit within the tolerance (with
/// `only_louder`, need only not exceed it).
fn onset_error(
    what: &str,
    l: &[f32],
    r: &[f32],
    at: usize,
    steady: usize,
    only_louder: bool,
) -> Option<String> {
    let seg = |from: usize, len: usize| (&l[from..from + len], &r[from..from + len]);
    let (al, ar) = seg(at, ONSET_WINDOW);
    let (sl, sr) = seg(steady, ONSET_WINDOW);
    let d_peak = peak_db(al, ar) - peak_db(sl, sr);
    let d_rms = rms_db(al, ar) - rms_db(sl, sr);
    let slice = ONSET_WINDOW / 4;
    let mut d_slice = 0.0f32;
    for k in 0..4 {
        let (al, ar) = seg(at + k * slice, slice);
        let (sl, sr) = seg(steady + k * slice, slice);
        let d = rms_db(al, ar) - rms_db(sl, sr);
        if d.abs() > d_slice.abs() {
            d_slice = d;
        }
    }
    eprintln!("{what}: peak {d_peak:+.2} dB, 400 ms {d_rms:+.2} dB, worst 100 ms {d_slice:+.2} dB");
    [("peak", d_peak), ("400 ms level", d_rms), ("100 ms level", d_slice)]
        .into_iter()
        .find(|(_, d)| if only_louder { *d > ONSET_TOLERANCE_DB } else { d.abs() > ONSET_TOLERANCE_DB })
        .map(|(label, d)| format!("{what}: the first 400 ms {label} is {d:+.2} dB off steady state"))
}

fn assert_no_onset_errors(errors: Vec<String>) {
    assert!(
        errors.is_empty(),
        "onsets off by more than ±{ONSET_TOLERANCE_DB} dB:\n{}",
        errors.join("\n")
    );
}

fn onset_settings(mode: Mode, drive: f32) -> Settings {
    Settings {
        mode,
        drive,
        mix: 1.0,
        auto_gain: true,
        ..Settings::default()
    }
}

/// Loop, 10 s of silence, loop: the gain the followers held before the
/// silence is the right one for the loop's return, so the return must
/// not overshoot while they re-converge.
#[test]
fn the_loop_returning_after_silence_does_not_overshoot() {
    let bar = drum_loop(BAR, -6.0, 23);
    let (lead_l, lead_r) = tiled_bars(&bar, 3);
    let gap = (10.0 * SR) as usize;
    let (tail_l, tail_r) = tiled_bars(&bar, 1);
    let with_gap = |lead: &[f32], tail: Vec<f32>| -> Vec<f32> {
        lead.iter()
            .copied()
            .chain(std::iter::repeat_n(0.0, gap))
            .chain(tail)
            .collect()
    };
    let l = with_gap(&lead_l, tail_l);
    let r = with_gap(&lead_r, tail_r);
    let resume = 3 * BAR + gap;
    let mut errors = Vec::new();
    for mode in Mode::ALL {
        for drive in ONSET_DRIVES {
            let (ol, or) = render(&onset_settings(mode, drive), &l, &r);
            let what = format!("{} drive {drive} after silence", mode.label());
            errors.extend(onset_error(&what, &ol, &or, resume, 2 * BAR, false));
        }
    }
    assert_no_onset_errors(errors);
}

/// A transport reset between two passes of the loop: the reset clears
/// the filters, not the match, so the second pass starts where the
/// first one settled.
#[test]
fn the_first_400_ms_after_a_reset_do_not_overshoot() {
    let bar = drum_loop(BAR, -6.0, 23);
    let (l, r) = tiled_bars(&bar, 3);
    let mut errors = Vec::new();
    for mode in Mode::ALL {
        for drive in ONSET_DRIVES {
            let mut plugin = ResonanceColor::new();
            apply_settings(&plugin.params, &onset_settings(mode, drive));
            plugin.initialize(SR, BLOCK as u32);
            render_with(&mut plugin, &l, &r);
            plugin.reset();
            let (ol, or) = render_with(&mut plugin, &l, &r);
            let what = format!("{} drive {drive} after a reset", mode.label());
            errors.extend(onset_error(&what, &ol, &or, 0, 2 * BAR, false));
        }
    }
    assert_no_onset_errors(errors);
}

/// A fresh instance has no match to start from. The rise limit holds the
/// gain under the one the first attack alone implies, so the start may
/// be quieter than steady state while the match climbs in (at most
/// `AUTO_GAIN_RISE_DB_PER_S`), but never louder.
#[test]
fn a_fresh_instance_never_starts_louder_than_steady_state() {
    let bar = drum_loop(BAR, -6.0, 23);
    let (l, r) = tiled_bars(&bar, 3);
    let mut errors = Vec::new();
    for mode in Mode::ALL {
        for drive in ONSET_DRIVES {
            let (ol, or) = render(&onset_settings(mode, drive), &l, &r);
            let what = format!("{} drive {drive} from a fresh instance", mode.label());
            errors.extend(onset_error(&what, &ol, &or, 0, 2 * BAR, true));
        }
    }
    assert_no_onset_errors(errors);
}
