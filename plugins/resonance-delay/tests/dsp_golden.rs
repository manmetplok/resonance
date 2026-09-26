//! Bit-exact DSP golden for the stereo delay (ba todo #1373).
//!
//! The rest of this crate's suite pins *arithmetic and routing*: the
//! division table resolves to the right millisecond count, the echo taps
//! land where they should, the stereo offset is applied to the right
//! line, the gate's duty cycle is what the parameter says. None of that
//! notices a change to the interpolator, the feedback filters, the
//! analog-mode saturation or the modulation LFO — the delay would sound
//! different and every assertion would still hold. This test pins the
//! rendered samples.
//!
//! # The signal, and why it is the revealing one
//!
//! A delay is only visible on material with a **transient you can
//! follow**, so the driver here is a 24 Hz train of short plucks: a
//! 4 ms exponential burst of a 700 Hz tone, peaking near full scale.
//! Against the ~22 ms delay times the scenarios use, each pluck's
//! repeats land in the gaps between the next plucks, so the golden
//! contains four or five clearly separated generations of every echo.
//! That is what makes the read-tap interpolation, the feedback gain and
//! the tone of each successive repeat observable; a sustained tone would
//! smear all of them into one steady level and pin almost nothing.
//!
//! A second signal — a sustained two-note chord — exists for the
//! scenarios where the *repeat* is not the point: modulation depth and
//! the analog mode's saturation both need continuous excitation to be
//! readable, and the ducking detector needs something to duck.
//!
//! # Tolerance: bit-exact
//!
//! The audio path is scalar f32: a fractional-delay read, one-pole
//! filters, a `tanh` waveshaper, an LFO and a dry/wet blend. No FFT, no
//! runtime SIMD dispatch, no RNG, no clock. (Bits can only move between
//! machines through libm's rounding of `sin`/`tanh`/`exp`, which shows
//! up as most of the render shifting by ~1e-7, never as a handful of
//! samples differing visibly.)
//!
//! To (re)generate the golden after an *intentional* audio change:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-delay --test dsp_golden
//!
//! A pure refactor must never need this.

use std::path::PathBuf;

use resonance_delay::params::DelayParams;
use resonance_delay::ResonanceDelay;
use resonance_dsp_test_support as golden;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin, TempoInfo};

const SR: f32 = 48_000.0;
const MAX_BLOCK: usize = 192;
/// Blocks per scenario — ~92 ms at the block sizes below, which is four
/// or five generations of every echo at the delay times used.
const BLOCKS: usize = 32;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "dsp_golden.f32")
}

/// `RESONANCE_BLESS=1` is the workspace-wide convention (CLAUDE.md); the
/// narrower name blesses only this file inside a wider run.
fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_DSP_GOLDEN"])
}

const TAU: f32 = std::f32::consts::TAU;

#[derive(Clone, Copy)]
enum Signal {
    /// 24 Hz train of 4 ms plucks — the transient whose repeats the
    /// golden is really about.
    Plucks,
    /// Sustained two-note chord: continuous excitation for the
    /// modulation, saturation and ducking scenarios.
    Chord,
}

impl Signal {
    fn sample(self, n: u64) -> (f32, f32) {
        let t = n as f32 / SR;
        match self {
            Signal::Plucks => {
                let x = t % (1.0 / 24.0);
                let env = (-500.0 * x).exp();
                let v = 0.9 * env * ((700.0 * t * TAU).sin() + 0.4 * (1900.0 * t * TAU).sin());
                // The right channel is a different pluck rate, so the
                // stereo routing modes are distinguishable: in Stereo
                // the two lines stay independent, in Ping-Pong and Dual
                // they are summed to mono first.
                let xr = (t + 0.021) % (1.0 / 24.0);
                let envr = (-500.0 * xr).exp();
                let vr = 0.9 * envr * ((520.0 * t * TAU).sin() + 0.4 * (1450.0 * t * TAU).sin());
                (v, vr)
            }
            Signal::Chord => {
                let v = 0.35 * ((196.0 * t * TAU).sin() + 0.7 * (294.0 * t * TAU).sin());
                let vr = 0.35 * ((233.0 * t * TAU).sin() + 0.7 * (349.0 * t * TAU).sin());
                (v, vr)
            }
        }
    }
}

fn tempo(bpm: f32) -> TempoInfo {
    TempoInfo {
        bpm,
        time_sig_num: 4,
        time_sig_den: 4,
        playing: true,
        song_pos_beats: 0.0,
    }
}

/// A parameter edit applied *between* blocks. `None` for static
/// scenarios.
type MidRunEdit = fn(&DelayParams, usize);

struct Scenario {
    name: &'static str,
    signal: Signal,
    blocks: &'static [usize],
    tempo: Option<TempoInfo>,
    setup: fn(&DelayParams),
    edit: Option<MidRunEdit>,
}

/// Free-running short-delay baseline every scenario starts from. The
/// factory default is a 375 ms tempo-synced quarter note, which would
/// not repeat once inside a 92 ms render.
fn base(p: &DelayParams) {
    p.sync.set_value(false);
    p.time_ms.set_value(22.0);
    p.feedback.set_value(0.6);
    p.mix.set_value(0.5);
    p.character.set_value(0);
    p.routing.set_value(0);
    p.stereo_offset.set_value(0.0);
    p.hi_cut.set_value(20000.0);
    p.lo_cut.set_value(20.0);
    p.drive.set_value(0.0);
    p.mod_rate.set_value(0.4);
    p.mod_depth.set_value(0.0);
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // 1. Plain digital stereo delay: two independent lines, filters
        //    wide open, no drive, no modulation. The clean reference —
        //    every echo generation is the previous one times the
        //    feedback gain, so any change to the read tap or the
        //    feedback path shows up immediately.
        Scenario {
            name: "digital_stereo_plucks",
            signal: Signal::Plucks,
            blocks: &[128],
            tempo: None,
            setup: base,
            edit: None,
        },
        // 2. Analog character with the drive up and both damping
        //    filters closed in: the repeats have to get darker and
        //    softer generation by generation. This is the whole
        //    difference between the two Character modes, and nothing
        //    else in the suite renders it.
        Scenario {
            name: "analog_damped_repeats",
            signal: Signal::Plucks,
            blocks: &[128, 192, 96],
            tempo: None,
            setup: |p| {
                base(p);
                p.character.set_value(1);
                p.drive.set_value(0.85);
                p.hi_cut.set_value(2500.0);
                p.lo_cut.set_value(300.0);
                p.feedback.set_value(0.75);
                p.time_ms.set_value(26.0);
            },
            edit: None,
        },
        // 3. Ping-pong: the input is summed to mono into the left line
        //    and the feedback crosses L↔R, so successive repeats
        //    alternate sides. A golden over plucks makes the
        //    alternation literal — a crossed-over pair of lines is a
        //    different waveform, not just a different level.
        Scenario {
            name: "pingpong_alternating",
            signal: Signal::Plucks,
            blocks: &[192, 64],
            tempo: None,
            setup: |p| {
                base(p);
                p.routing.set_value(1);
                p.feedback.set_value(0.72);
                p.time_ms.set_value(19.0);
                p.mix.set_value(0.7);
            },
            edit: None,
        },
        // 4. Dual lines with a stereo offset and the modulation at
        //    depth: both taps move continuously and by different
        //    amounts, so the interpolator is reading a fractional
        //    position that changes every sample. Sustained material,
        //    because a modulated delay's pitch wobble is only audible
        //    on something that lasts.
        Scenario {
            name: "dual_offset_modulated",
            signal: Signal::Chord,
            blocks: &[128, 192],
            tempo: None,
            setup: |p| {
                base(p);
                p.routing.set_value(2);
                p.stereo_offset.set_value(0.35);
                p.mod_rate.set_value(4.5);
                p.mod_depth.set_value(0.9);
                p.time_ms.set_value(24.0);
                p.feedback.set_value(0.55);
            },
            edit: None,
        },
        // 5. Tempo-synced with the division stepped between blocks. At
        //    400 BPM a 1/16 is 37.5 ms and a 1/16T is 25 ms, both of
        //    which repeat several times inside the run — so the golden
        //    covers the sync resolution *and* the smoothed glide the
        //    read tap takes when the target moves.
        Scenario {
            name: "tempo_sync_division_steps",
            signal: Signal::Plucks,
            blocks: &[128, 96],
            tempo: Some(tempo(400.0)),
            setup: |p| {
                base(p);
                p.sync.set_value(true);
                p.division.set_value(10); // 1/16
                p.feedback.set_value(0.65);
            },
            edit: Some(|p, block| {
                // 10 = 1/16, 11 = 1/16T: the two shortest divisions,
                // alternated so the glide runs in both directions.
                p.division.set_value(if block % 12 >= 6 { 11 } else { 10 });
            }),
        },
        // 6. Freeze engaged and released mid-run: the input stops
        //    reaching the line, the feedback goes to unity, and the
        //    captured loop must repeat verbatim until the release. A
        //    freeze that leaks input or decays is a different sound and
        //    only a sample-level pin catches it.
        Scenario {
            name: "freeze_cycle",
            signal: Signal::Plucks,
            blocks: &[128, 192, 77],
            tempo: None,
            setup: |p| {
                base(p);
                p.time_ms.set_value(28.0);
                p.mix.set_value(0.9);
            },
            edit: Some(|p, block| {
                p.freeze
                    .set_value((BLOCKS / 4..3 * BLOCKS / 4).contains(&block));
            }),
        },
        // 7. The wet gate chopping the repeats while ducking pulls them
        //    down under the dry input. Both are level/time shaping
        //    applied *after* the delay line, so they are invisible to
        //    any test that looks at tap positions — and both are the
        //    kind of thing a refactor reorders by accident.
        Scenario {
            name: "gated_and_ducked",
            signal: Signal::Plucks,
            blocks: &[192, 128],
            tempo: Some(tempo(400.0)),
            setup: |p| {
                base(p);
                p.gate_on.set_value(true);
                p.gate_rate.set_value(11); // 1/16T at 400 BPM = 25 ms
                p.gate_width.set_value(0.4);
                p.gate_shape.set_value(0.15);
                p.gate_depth.set_value(0.9);
                p.duck_amount.set_value(0.8);
                p.duck_threshold.set_value(-30.0);
                p.duck_release.set_value(60.0);
                p.mix.set_value(0.8);
            },
            edit: None,
        },
        // 8. Everything swept between blocks, so every smoother is
        //    mid-ramp at each block boundary and the routing/character
        //    branches all get taken in one run.
        Scenario {
            name: "param_sweeps_between_blocks",
            signal: Signal::Plucks,
            blocks: &[96, 192, 64],
            tempo: None,
            setup: base,
            edit: Some(|p, block| {
                let t = block as f32 / BLOCKS as f32;
                p.time_ms.set_value(12.0 + 26.0 * t);
                p.feedback.set_value(0.2 + 0.7 * t);
                p.mix.set_value(0.2 + 0.8 * t);
                p.stereo_offset.set_value(-0.5 + t);
                p.hi_cut.set_value(20000.0 - 18000.0 * t);
                p.lo_cut.set_value(20.0 + 800.0 * t);
                p.drive.set_value(t);
                p.mod_rate.set_value(0.1 + 5.0 * t);
                p.mod_depth.set_value(t);
                // Routing and character cycle, so the three feedback
                // topologies and both saturation modes are all in the
                // golden together with mid-run switches between them.
                p.routing.set_value((block / 5 % 3) as i32);
                p.character.set_value((block / 7 % 2) as i32);
            }),
        },
    ]
}

/// Deterministic render of one scenario into the golden sample stream.
fn render_scenario(s: &Scenario) -> Vec<f32> {
    let mut plugin = ResonanceDelay::new();
    (s.setup)(&plugin.params);
    plugin.initialize(SR, MAX_BLOCK as u32);
    plugin.reset();

    let mut out = Vec::new();
    let mut left = vec![0.0f32; MAX_BLOCK];
    let mut right = vec![0.0f32; MAX_BLOCK];
    let mut n: u64 = 0;

    for block in 0..BLOCKS {
        if let Some(edit) = s.edit {
            edit(&plugin.params, block);
        }
        let frames = s.blocks[block % s.blocks.len()];
        for i in 0..frames {
            let (l, r) = s.signal.sample(n + i as u64);
            left[i] = l;
            right[i] = r;
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..frames],
                right: &mut right[..frames],
            }];
            let mut ev = EventIterator::empty();
            // The transport rolls with the render, as a host's would: the
            // wet gate locks its phase to `song_pos_beats` (LIB-03).
            let tempo = s.tempo.map(|t| TempoInfo {
                song_pos_beats: t.song_pos_beats + n as f64 * t.bpm as f64 / (60.0 * SR as f64),
                ..t
            });
            plugin.process(&mut outs, frames, &mut ev, tempo);
        }
        n += frames as u64;
        out.extend_from_slice(&left[..frames]);
        out.extend_from_slice(&right[..frames]);
    }
    out
}

fn render_all() -> Vec<f32> {
    let mut all = Vec::new();
    for s in scenarios() {
        all.extend(render_scenario(&s));
    }
    all
}

#[test]
fn delay_output_is_bit_exact() {
    let rendered = render_all();
    assert!(
        rendered.iter().all(|s| s.is_finite()),
        "render produced non-finite samples"
    );

    let path = golden_path();
    if blessing() {
        golden::bless_f32(&path, &rendered);
        return;
    }

    let want = golden::load_golden_f32(&path, rendered.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_f32(&rendered, &want);

    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "delay DSP output changed: {}/{} samples differ, peak delta \
             {:.3e}; first at sample {i} (got {got:?} / {:#010x}, want \
             {want:?} / {:#010x}).\nA refactor of the delay path must be \
             bit-exact. If the change was intended, re-bless with \
             RESONANCE_BLESS=1.\nA peak delta at ~1e-7 spread over most of the \
             render is libm rounding, not a DSP change.",
            diff.diff_count,
            rendered.len(),
            diff.max_abs,
            got.to_bits(),
            want.to_bits(),
        );
    }
}

/// Guards the scenario table: every scenario must render audible wet
/// content — energy that arrives *after* the dry input has gone quiet.
/// A scenario whose delay time outran the render, or whose mix collapsed
/// to dry, would pin the dry signal and nothing else.
#[test]
fn every_scenario_produces_repeats() {
    for s in scenarios() {
        let out = render_scenario(&s);
        assert!(
            out.iter().all(|x| x.is_finite()),
            "scenario `{}` rendered non-finite samples",
            s.name
        );
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-3, "scenario `{}` rendered silence", s.name);

        // Render the same input dry and require the output to depart
        // from every scalar multiple of it: the wet path is by
        // definition delayed, so it cannot be a gain change.
        let dry = render_dry(&s);
        assert!(
            golden::residual_fraction(&out, &dry) > 0.1,
            "scenario `{}` output is within 10% of a scaled copy of its input — \
             no repeats reached the render window",
            s.name
        );
    }
}

/// The same input stream a scenario feeds the plugin, in the same
/// per-block L-then-R layout, untouched.
fn render_dry(s: &Scenario) -> Vec<f32> {
    let mut out = Vec::new();
    let mut n: u64 = 0;
    for block in 0..BLOCKS {
        let frames = s.blocks[block % s.blocks.len()];
        for i in 0..frames {
            out.push(s.signal.sample(n + i as u64).0);
        }
        for i in 0..frames {
            out.push(s.signal.sample(n + i as u64).1);
        }
        n += frames as u64;
    }
    out
}
