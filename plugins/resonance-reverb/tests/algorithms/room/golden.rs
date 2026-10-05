//! Bit-exact goldens for Room, Chamber and Ambience (§5.2 "Goldens"):
//! per engine an impulse (L at 0, R at 37) at a factory voicing and at a
//! corner of the parameter space, plus a noise burst for the modulated
//! engines (Room, Chamber) with a size glide or Freeze landing mid-tail.
//! Every scenario pins every setter and carries its own silence guard.
//!
//! Re-bless after an intended change only:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-reverb --test algorithms room::golden
//!
//! (`RESONANCE_BLESS_ROOM=1` blesses only these three files.)

use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_reverb::dsp::{Algorithm, ReverbDsp};

use super::common::*;

type Edit = (usize, fn(&mut ReverbDsp));

struct Scenario {
    name: &'static str,
    setup: Setup,
    predelay_ms: f32,
    frames: usize,
    input: fn(usize) -> (f32, f32),
    edit: Option<Edit>,
}

fn impulse_lr(n: usize) -> (f32, f32) {
    (
        if n == 0 { 1.0 } else { 0.0 },
        if n == 37 { 1.0 } else { 0.0 },
    )
}

/// A 15 ms Hann-windowed noise burst, decorrelated L/R.
fn burst(n: usize) -> (f32, f32) {
    if n < 720 {
        let w = 0.5 - 0.5 * (std::f32::consts::TAU * n as f32 / 720.0).cos();
        (0.8 * w * noise(n), 0.8 * w * noise(n + 9_973))
    } else {
        (0.0, 0.0)
    }
}

fn room_scenarios() -> Vec<Scenario> {
    vec![
        // The Drum Room voicing: ER-forward kit ambience.
        Scenario {
            name: "room_impulse_drum_room",
            setup: Setup {
                size: 0.35,
                decay: 0.7,
                damping: 7_000.0,
                diffusion: 0.75,
                er_level: 0.7,
                er_time: 0.45,
                mod_rate: 0.8,
                mod_depth: 0.15,
                low_mult: 1.0,
                low_xover: 250.0,
                high_mult: 0.5,
                width: 1.0,
                ..Setup::new(Algorithm::Room, 0.35, 0.7)
            },
            predelay_ms: 0.0,
            frames: 12_000,
            input: impulse_lr,
            edit: None,
        },
        // The small, dark, undiffused corner: the smallest box, the
        // diffusers as plain delays, every shelf deep.
        Scenario {
            name: "room_impulse_small_dark_undiffused",
            setup: Setup {
                size: 0.0,
                decay: 0.4,
                damping: 900.0,
                diffusion: 0.0,
                er_level: 1.0,
                er_time: 0.1,
                mod_rate: 0.3,
                mod_depth: 0.0,
                low_mult: 0.5,
                low_xover: 150.0,
                high_mult: 0.2,
                width: 1.0,
                ..Setup::new(Algorithm::Room, 0.0, 0.4)
            },
            predelay_ms: 0.0,
            frames: 7_200,
            input: impulse_lr,
            edit: None,
        },
        // A noise burst into a big, fully modulated room with a shaped
        // decay, a pre-delay and a narrowed width; at 150 ms the size
        // drops, so the line glide and the ER crossfade are pinned too.
        Scenario {
            name: "room_burst_modulated_glide",
            setup: Setup {
                size: 0.8,
                decay: 2.5,
                damping: 6_000.0,
                diffusion: 0.95,
                er_level: 0.4,
                er_time: 0.7,
                mod_rate: 3.0,
                mod_depth: 1.0,
                low_mult: 1.3,
                low_xover: 400.0,
                high_mult: 0.6,
                width: 0.8,
                ..Setup::new(Algorithm::Room, 0.8, 2.5)
            },
            predelay_ms: 10.0,
            frames: 14_400,
            input: burst,
            edit: Some((7_200, |d| d.set_size(0.4))),
        },
    ]
}

fn chamber_scenarios() -> Vec<Scenario> {
    vec![
        // The Vocal Chamber voicing.
        Scenario {
            name: "chamber_impulse_vocal_chamber",
            setup: Setup::vocal_chamber(),
            predelay_ms: 0.0,
            frames: 14_400,
            input: impulse_lr,
            edit: None,
        },
        // A burst into the largest chamber, modulation wide open, Freeze
        // engaged at 200 ms: the lossless loop and the muted input.
        Scenario {
            name: "chamber_burst_modulated_freeze",
            setup: Setup {
                size: 1.0,
                decay: 3.0,
                damping: 12_000.0,
                diffusion: 1.0,
                er_level: 0.5,
                er_time: 0.9,
                mod_rate: 4.5,
                mod_depth: 1.0,
                low_mult: 2.0,
                low_xover: 600.0,
                high_mult: 0.3,
                width: 1.0,
                ..Setup::new(Algorithm::Chamber, 1.0, 3.0)
            },
            predelay_ms: 0.0,
            frames: 14_400,
            input: burst,
            edit: Some((9_600, |d| d.set_freeze(true))),
        },
    ]
}

fn ambience_scenarios() -> Vec<Scenario> {
    vec![
        // The Mix Glue voicing.
        Scenario {
            name: "ambience_impulse_mix_glue",
            setup: Setup {
                size: 0.35,
                decay: 0.5,
                damping: 9_000.0,
                diffusion: 0.8,
                er_level: 0.6,
                er_time: 0.4,
                mod_rate: 0.5,
                mod_depth: 0.0,
                low_mult: 0.8,
                low_xover: 250.0,
                high_mult: 0.6,
                width: 1.0,
                ..Setup::new(Algorithm::Ambience, 0.35, 0.5)
            },
            predelay_ms: 0.0,
            frames: 9_600,
            input: impulse_lr,
            edit: None,
        },
        // The largest box at the widest spacing, decay far above the
        // 1 s clamp, modulation asked for (Ambience has none).
        Scenario {
            name: "ambience_impulse_large_clamped",
            setup: Setup {
                size: 1.0,
                decay: 6.0,
                damping: 20_000.0,
                diffusion: 1.0,
                er_level: 1.0,
                er_time: 1.0,
                mod_rate: 5.0,
                mod_depth: 1.0,
                low_mult: 1.5,
                low_xover: 300.0,
                high_mult: 1.0,
                width: 0.6,
                ..Setup::new(Algorithm::Ambience, 1.0, 6.0)
            },
            predelay_ms: 5.0,
            frames: 9_600,
            input: impulse_lr,
            edit: None,
        },
    ]
}

/// The scenario's output and its input energy.
fn render(s: &Scenario) -> (Vec<f32>, Vec<f32>, f64) {
    let v = s.setup;
    let mut d = v.dsp();
    d.set_predelay(s.predelay_ms);
    let (mut l, mut r) = (Vec::with_capacity(s.frames), Vec::with_capacity(s.frames));
    let mut e_in = 0.0f64;
    for n in 0..s.frames {
        if let Some((at, edit)) = s.edit {
            if n == at {
                edit(&mut d);
            }
        }
        let (x, y) = (s.input)(n);
        e_in += 0.5 * ((x as f64).powi(2) + (y as f64).powi(2));
        let (a, b) = d.process(x, y, v.diffusion, v.width);
        l.push(a);
        r.push(b);
    }
    (l, r, e_in)
}

fn check(file: &str, scenarios: Vec<Scenario>) {
    let mut rendered = Vec::new();
    for s in &scenarios {
        let (l, r, e_in) = render(s);
        assert!(
            l.iter().chain(&r).all(|x| x.is_finite()),
            "{}: non-finite",
            s.name
        );
        // Silence guard, re the scenario's own input energy.
        let e = energy_db(&l, &r) - 10.0 * e_in.log10();
        assert!(
            e > -40.0,
            "{}: {e:.1} dB re its input (silence guard)",
            s.name
        );
        let tail = s.frames * 3 / 4;
        let tail_db = energy_db(&l[tail..], &r[tail..]) - 10.0 * e_in.log10();
        assert!(
            tail_db > -60.0,
            "{}: no tail in the last quarter ({tail_db:.1} dB)",
            s.name
        );
        println!(
            "{}: {e:.1} dB, last quarter {tail_db:.1} dB re input",
            s.name
        );
        rendered.extend(l);
        rendered.extend(r);
    }

    let path: PathBuf = golden::golden_path(env!("CARGO_MANIFEST_DIR"), file);
    if golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_ROOM"]) {
        golden::bless_f32(&path, &rendered);
        return;
    }
    let want = golden::load_golden_f32(&path, rendered.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&rendered, &want);
    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "{file}: output changed: {}/{} samples differ, peak delta {:.3e}; first at \
             sample {i} (got {got:?}, want {want:?}). Re-bless with RESONANCE_BLESS=1 \
             only for an intended change.",
            diff.diff_count,
            rendered.len(),
            diff.max_abs,
        );
    }
}

#[test]
fn room_golden_is_bit_exact() {
    check("room_golden.f32", room_scenarios());
}

#[test]
fn chamber_golden_is_bit_exact() {
    check("chamber_golden.f32", chamber_scenarios());
}

#[test]
fn ambience_golden_is_bit_exact() {
    check("ambience_golden.f32", ambience_scenarios());
}
