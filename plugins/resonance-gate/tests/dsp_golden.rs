//! Bit-exact DSP golden for the gate (ba todo #1373).
//!
//! `tests/gate.rs` asserts *behaviour* — a loud signal passes, a quiet
//! one is gated, hold bridges a gap, the key replaces the detector. All
//! of that stays true if someone changes the attack curve, the detector
//! filter or the dB conversion; the plugin would sound different and the
//! suite would stay green. This test pins the samples themselves, so a
//! refactor that moves the sound has to say so out loud.
//!
//! # The signal, and why it is the revealing one
//!
//! A gate is a *threshold-crossing* device: everything interesting about
//! it happens at the edges. A steady tone pins the open state and
//! nothing else; silence pins the closed state and nothing else. So the
//! driving material here is a **burst train** — 22 ms of a 220 Hz tone
//! at −6 dBFS, then 28 ms of a −66 dBFS deterministic noise floor —
//! which puts a rising and a falling threshold crossing into every
//! 50 ms of the render. That exercises, in order and repeatedly:
//! attack, the open state, hold, release, the closed floor, and the
//! hysteresis offset that decides *where* the falling edge lands.
//!
//! One scenario instead drives a slow crescendo through the threshold,
//! because a burst train opens and closes so fast that a wrong
//! hysteresis *sign* still looks plausible; a slow ramp separates the
//! open point from the close point far enough to pin both. Another
//! drives a snare-ish tone sitting on 55 Hz rumble so the detector HPF
//! has something to reject, and another supplies an external key whose
//! content is nowhere in the main buffer, so the golden cannot be
//! matched by a build that quietly falls back to the internal detector.
//!
//! # Tolerance: bit-exact
//!
//! The whole audio path is scalar f32 — one-pole ballistics, a biquad
//! detector HPF, dB conversions and a gain multiply. No FFT, no runtime
//! SIMD dispatch, no RNG, no clock. Given the same input the output is
//! reproducible, so anything less than bit equality would let a real
//! change hide. (The one thing that can legitimately move a bit between
//! machines is libm's rounding of `log10`/`exp`/`sin`; that shows up as
//! thousands of samples differing at the 1e-7 level, never as a handful
//! differing visibly. The failure message spells out the difference.)
//!
//! To (re)generate the golden after an *intentional* audio change:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-gate --test dsp_golden
//!
//! A pure refactor must never need this.
//!
//! # Re-blessed once, deliberately (ba todo #1343)
//!
//! The fixture was regenerated when the swapped ballistics were fixed:
//! `attack` had been driving the closing ramp and `release` the opening
//! one, so every scenario here rendered its edges the wrong way round.
//! 65 180 of 65 184 samples moved, peak delta 0.50 — a real sound change,
//! signed off as such, not a rounding drift.
//!
//! The scenario table below was NOT retuned. Its settings span the useful
//! range of both controls and stay meaningful under either reading, so
//! leaving them alone keeps the fixture comparable across the change:
//! the diff is purely what the fix did, with nothing else moving at the
//! same time.
//!
//! `tests/ballistics.rs` is the test that says which way round the two
//! controls belong. This file only pins that the samples do not move
//! unnoticed; it would have been just as happy with the inverted build,
//! which is exactly why the bug survived here for as long as it did.

use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_gate::params::GateParams;
use resonance_gate::ResonanceGate;
use resonance_plugin::{EventIterator, KeyBuffer, OutputBuffer, ResonancePlugin};

const SR: f32 = 48_000.0;
/// Host block sizes are cycled per scenario so the per-block
/// `prepare_block` coefficient refresh lands at several offsets relative
/// to the burst edges. `MAX_BLOCK` is what the plugin is activated with.
const MAX_BLOCK: usize = 320;
/// Blocks rendered per scenario. At the block sizes used below this is
/// ~130 ms per scenario — two and a half burst periods, which is all a
/// gate needs to show every state, and keeps the fixture small.
const BLOCKS: usize = 24;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "dsp_golden.f32")
}

/// `RESONANCE_BLESS=1` is the workspace-wide convention (CLAUDE.md); the
/// narrower name exists so blessing one plugin's DSP golden inside a
/// wider run is possible without re-blessing everything.
fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_DSP_GOLDEN"])
}

/// Deterministic pseudo-noise from the absolute sample index, so the
/// stream is identical however the blocks are chopped. Used only as a
/// *floor* — a gate's closed state is never digital silence in practice,
/// and a noise floor is what actually reveals the range parameter.
fn noise(n: u64) -> f32 {
    let mut s = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) as u32;
    s ^= s >> 16;
    s = s.wrapping_mul(2_246_822_519);
    s ^= s >> 13;
    (s >> 8) as f32 * (2.0 / (1 << 23) as f32) - 1.0
}

const TAU: f32 = std::f32::consts::TAU;

/// The material each scenario is driven with.
#[derive(Clone, Copy)]
enum Signal {
    /// 22 ms of a −6 dBFS 220 Hz tone, then 28 ms of a −66 dBFS floor.
    /// Two threshold crossings per 50 ms: the gate's whole job.
    Bursts,
    /// The same train, but the tone sits on top of a −14 dBFS 55 Hz
    /// rumble that never stops. With the detector HPF off the rumble
    /// holds the gate open forever; with it on the bursts still gate.
    BurstsOnRumble,
    /// A 330 Hz tone whose level crawls from −72 dBFS up to −6 dBFS and
    /// back over the run. Separates the opening threshold from the
    /// closing one, which a fast burst train cannot.
    Crescendo,
    /// A sustained −12 dBFS pad. On its own it never closes the gate —
    /// only useful paired with a key.
    Pad,
}

impl Signal {
    fn sample(self, n: u64) -> (f32, f32) {
        let t = n as f32 / SR;
        match self {
            Signal::Bursts => {
                let v = burst_env(t) * 0.5 * (220.0 * t * TAU).sin() + 0.0005 * noise(n);
                // A small L/R level difference so a channel swap or a
                // mono-summed detector shows up in the golden.
                (v, v * 0.85)
            }
            Signal::BurstsOnRumble => {
                let rumble = 0.2 * (55.0 * t * TAU).sin();
                let v = burst_env(t) * 0.5 * (330.0 * t * TAU).sin() + rumble + 0.0005 * noise(n);
                (v, v * 0.85)
            }
            Signal::Crescendo => {
                // Triangle in dB from −72 to −6 and back across the run.
                let period = 0.12;
                let phase = (t / period) % 2.0;
                let up = if phase < 1.0 { phase } else { 2.0 - phase };
                let db = -72.0 + 66.0 * up;
                let amp = 10.0f32.powf(db / 20.0);
                let v = amp * (330.0 * t * TAU).sin();
                (v, v * 0.85)
            }
            Signal::Pad => {
                let v = 0.25 * ((196.0 * t * TAU).sin() + 0.6 * (294.0 * t * TAU).sin());
                (v, v * 0.9)
            }
        }
    }
}

/// 22 ms open / 28 ms closed, with 1 ms raised-cosine edges so the
/// *source* has no discontinuity of its own — every edge in the output
/// is the gate's doing, not the test signal's.
fn burst_env(t: f32) -> f32 {
    let period = 0.05;
    let open = 0.022;
    let x = t % period;
    let edge = 0.001;
    if x < edge {
        0.5 - 0.5 * (std::f32::consts::PI * x / edge).cos()
    } else if x < open - edge {
        1.0
    } else if x < open {
        0.5 + 0.5 * (std::f32::consts::PI * (x - (open - edge)) / edge).cos()
    } else {
        0.0
    }
}

/// A kick-shaped external key: a 10 Hz train of 60 Hz bursts with a fast
/// exponential decay. Deliberately absent from the main buffer, so the
/// key scenario can only match if the key really drives the detector.
fn key_sample(n: u64) -> f32 {
    let t = n as f32 / SR;
    let x = t % 0.1;
    let env = (-70.0 * x).exp();
    0.7 * env * (60.0 * t * TAU).sin()
}

/// A parameter edit applied *between* blocks, so the next block's
/// `settings()` snapshot picks it up. `None` for static scenarios.
type MidRunEdit = fn(&GateParams, usize);

struct Scenario {
    name: &'static str,
    signal: Signal,
    /// Host block sizes, cycled across the run.
    blocks: &'static [usize],
    /// Whether to feed an external key through `process_with_key`.
    key: bool,
    setup: fn(&GateParams),
    edit: Option<MidRunEdit>,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // 1. Factory defaults on the burst train. The path a user hears
        //    first: −40 dB threshold, 8:1, 1 ms attack, 20 ms hold,
        //    100 ms release, 60 dB range, 6 dB hysteresis.
        Scenario {
            name: "factory_defaults_bursts",
            signal: Signal::Bursts,
            blocks: &[256],
            key: false,
            setup: |_| {},
            edit: None,
        },
        // 2. Ballistics at their fastest and the ratio at its hardest,
        //    hysteresis off: the closing edge lands exactly on the
        //    threshold crossing, so the attack/release one-poles are
        //    pinned with nothing smoothing over them.
        Scenario {
            name: "hard_fast_no_hysteresis",
            signal: Signal::Bursts,
            blocks: &[64, 197, 320],
            key: false,
            setup: |p| {
                p.threshold.set_value(-30.0);
                p.ratio.set_value(20.0);
                p.attack.set_value(0.05);
                p.hold.set_value(0.0);
                p.release.set_value(5.0);
                p.range.set_value(80.0);
                p.hysteresis.set_value(0.0);
            },
            edit: None,
        },
        // 3. The same train as a gentle downward expander: low ratio,
        //    shallow range. This is the setting where "gate" and
        //    "no-op" are only 12 dB apart, so it catches a range or
        //    slope change that the hard-gate scenario would saturate.
        Scenario {
            name: "gentle_expander",
            signal: Signal::Bursts,
            blocks: &[128, 96],
            key: false,
            setup: |p| {
                p.threshold.set_value(-24.0);
                p.ratio.set_value(2.0);
                p.range.set_value(12.0);
                p.attack.set_value(20.0);
                p.release.set_value(400.0);
                p.hold.set_value(0.0);
                p.hysteresis.set_value(0.0);
            },
            edit: None,
        },
        // 4. Slow crescendo with the hysteresis at maximum. The gate
        //    must open 24 dB higher than it closes; a sign error or a
        //    dropped `max(0.0)` moves one of those two points and
        //    nothing else, which is invisible on a burst train.
        Scenario {
            name: "hysteresis_crescendo",
            signal: Signal::Crescendo,
            blocks: &[192],
            key: false,
            setup: |p| {
                p.threshold.set_value(-36.0);
                p.hysteresis.set_value(24.0);
                p.ratio.set_value(10.0);
                p.attack.set_value(2.0);
                p.release.set_value(80.0);
                p.hold.set_value(10.0);
            },
            edit: None,
        },
        // 5. Detector HPF doing the job it exists for: rejecting low
        //    rumble that would otherwise hold the gate open. Without
        //    the filter this scenario's output is the input.
        Scenario {
            name: "key_hpf_rejects_rumble",
            signal: Signal::BurstsOnRumble,
            blocks: &[256, 160],
            key: false,
            setup: |p| {
                p.threshold.set_value(-20.0);
                p.key_hpf.set_value(250.0);
                p.hold.set_value(5.0);
                p.release.set_value(40.0);
                p.range.set_value(48.0);
            },
            edit: None,
        },
        // 6. External key. The main buffer is a sustained pad that
        //    never crosses anything; every edge in the output comes
        //    from the 60 Hz kick on the key port. A build that ignored
        //    the key would render a flat open pad and miss by a mile.
        Scenario {
            name: "external_key_ducks_pad",
            signal: Signal::Pad,
            blocks: &[256, 320, 101],
            key: true,
            setup: |p| {
                p.threshold.set_value(-18.0);
                p.ratio.set_value(12.0);
                p.attack.set_value(0.5);
                p.hold.set_value(30.0);
                p.release.set_value(150.0);
                p.range.set_value(40.0);
                p.hysteresis.set_value(3.0);
            },
            edit: None,
        },
        // 7. Every control swept between blocks, so each block re-reads
        //    the params and recomputes its coefficients mid-burst. This
        //    is the scenario that pins the block-rate refresh itself.
        Scenario {
            name: "param_sweeps_between_blocks",
            signal: Signal::Bursts,
            blocks: &[128, 320, 64],
            key: false,
            setup: |p| {
                p.threshold.set_value(-30.0);
            },
            edit: Some(|p, block| {
                let t = block as f32 / BLOCKS as f32;
                p.threshold.set_value(-54.0 + 42.0 * t);
                p.ratio.set_value(1.5 + 16.0 * t);
                p.attack.set_value(0.05 + 30.0 * t);
                p.release.set_value(8.0 + 500.0 * t);
                p.range.set_value(6.0 + 70.0 * t);
                p.hysteresis.set_value(if block % 16 >= 8 { 18.0 } else { 0.0 });
                p.key_hpf.set_value(if block % 24 >= 12 { 400.0 } else { 0.0 });
                // Hold crosses zero, so the hold counter both engages
                // and disengages inside one run.
                p.hold.set_value(if block % 20 >= 10 { 60.0 } else { 0.0 });
            }),
        },
    ]
}

/// Deterministic render of one scenario into the golden sample stream.
fn render_scenario(s: &Scenario) -> Vec<f32> {
    let mut plugin = ResonanceGate::new();
    (s.setup)(&plugin.params);
    plugin.initialize(SR, MAX_BLOCK as u32);
    // A fresh plugin is already clean, but resetting makes the render
    // independent of anything `new()`/`initialize()` might start doing.
    plugin.reset();

    let mut out = Vec::new();
    let mut left = vec![0.0f32; MAX_BLOCK];
    let mut right = vec![0.0f32; MAX_BLOCK];
    let mut key_l = vec![0.0f32; MAX_BLOCK];
    let mut key_r = vec![0.0f32; MAX_BLOCK];
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
            if s.key {
                let k = key_sample(n + i as u64);
                key_l[i] = k;
                // Slightly different right key so a detector that only
                // reads one channel is distinguishable from one that
                // sums both.
                key_r[i] = k * 0.75;
            }
        }
        {
            let mut outs = [OutputBuffer {
                left: &mut left[..frames],
                right: &mut right[..frames],
            }];
            let mut ev = EventIterator::empty();
            let key = s.key.then(|| KeyBuffer {
                left: &key_l[..frames],
                right: &key_r[..frames],
            });
            plugin.process_with_key(&mut outs, key, frames, &mut ev, None);
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
fn gate_output_is_bit_exact() {
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
            "gate DSP output changed: {}/{} samples differ, peak delta \
             {:.3e}; first at sample {i} (got {got:?} / {:#010x}, want \
             {want:?} / {:#010x}).\nA refactor of the gate path must be bit-exact. \
             If the change was intended, re-bless with RESONANCE_BLESS=1.\nA peak \
             delta at ~1e-7 spread over most of the render is libm rounding \
             (log10/exp/sin), not a DSP change.",
            diff.diff_count,
            rendered.len(),
            diff.max_abs,
            got.to_bits(),
            want.to_bits(),
        );
    }
}

/// Guards the scenario table itself: every scenario must actually gate
/// — the output has to differ from the input somewhere, and it has to
/// both open (pass something near full level) and close (drop far
/// below it). A scenario that silently degenerated into a bypass would
/// otherwise sit in the golden forever pinning nothing.
#[test]
fn every_scenario_opens_and_closes() {
    for s in scenarios() {
        let out = render_scenario(&s);
        assert!(
            out.iter().all(|x| x.is_finite()),
            "scenario `{}` rendered non-finite samples",
            s.name
        );

        // Peak over the second half (past the initial open) versus the
        // quietest 512-sample window in the same span.
        let half = out.len() / 2;
        let tail = &out[half..];
        let peak = tail.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let quietest = tail
            .chunks(512)
            .map(|w| w.iter().fold(0.0f32, |m, x| m.max(x.abs())))
            .fold(f32::INFINITY, f32::min);

        assert!(
            peak > 1e-3,
            "scenario `{}` rendered near-silence — the gate never opened",
            s.name
        );
        assert!(
            quietest < peak * 0.5,
            "scenario `{}` never closed: quietest window {quietest:.3e} vs peak \
             {peak:.3e}. A gate golden over a permanently-open gate pins nothing.",
            s.name
        );
    }
}
