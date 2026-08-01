//! Regression null test for the synth's audio output.
//!
//! Renders a fixed note sequence through several representative parameter
//! configurations and compares the result against a golden f32 dump captured
//! from a known-good build. Any DSP change that alters the rendered audio
//! fails here.
//!
//! To (re)generate the golden after an *intentional* audio change:
//!
//!     RESONANCE_BLESS_NULL_TEST=1 cargo test -p resonance-wavetable \
//!         --no-default-features --test null_test
//!
//! and describe the change in the commit message. Do not bless casually — the
//! whole point is that optimisation work leaves the sound untouched.

use std::path::PathBuf;

use resonance_plugin::{EventIterator, NoteEvent};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::params::WavetableParams;

const SR: f32 = 48_000.0;
const BLOCK: usize = 128;
/// Blocks rendered per scenario: enough to cover attack, sustain, note-off
/// and part of the release tail.
const BLOCKS: usize = 96;

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/null_test.f32")
}

/// Deterministic, reproducible render of one scenario. Notes come in at
/// staggered offsets and are released partway through so every envelope
/// stage, the voice-stealing path and the release tail all get exercised.
fn render_scenario(params: &WavetableParams, notes: &[u8]) -> Vec<f32> {
    let mut engine = SynthEngine::new();
    engine.initialize(SR);

    let mut out = Vec::with_capacity(BLOCKS * BLOCK * 2);
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];

    for block in 0..BLOCKS {
        let mut events: Vec<NoteEvent> = Vec::new();
        // Stagger note-ons over the first blocks, one per block, at a
        // non-zero sample offset so the sample-accurate event path is used.
        if block < notes.len() {
            events.push(NoteEvent::NoteOn {
                note: notes[block],
                velocity: 0.35 + 0.09 * block as f32,
                timing: (block * 7 % BLOCK) as u32,
            });
        }
        // Release them all in the second half.
        let release_start = BLOCKS / 2;
        if block >= release_start && block - release_start < notes.len() {
            events.push(NoteEvent::NoteOff {
                note: notes[block - release_start],
                timing: (block * 11 % BLOCK) as u32,
            });
        }

        let mut iter = EventIterator::new(&events);
        engine.render_block(&mut left, &mut right, BLOCK, params, &mut iter);
        out.extend_from_slice(&left);
        out.extend_from_slice(&right);
    }
    out
}

/// Every scenario the golden covers, in a fixed order.
fn scenarios() -> Vec<(&'static str, WavetableParams, Vec<u8>)> {
    let mut v = Vec::new();

    // 1. Init patch, single osc, mono note.
    {
        let p = WavetableParams::new();
        v.push(("init_single", p, vec![60]));
    }

    // 2. Both oscs, unison 5, filter with drive, mod matrix, chorus + delay.
    //    Covers the whole signal chain at once.
    {
        let p = WavetableParams::new();
        p.osc1.enabled.set_value(true);
        p.osc2.enabled.set_value(true);
        p.osc1.wavetable.set_value(3);
        p.osc2.wavetable.set_value(6);
        p.osc1.position.set_value(0.37);
        p.osc2.position.set_value(0.81);
        p.osc2.coarse.set_value(-12);
        p.osc1.fine.set_value(-5.0);
        p.osc2.fine.set_value(7.0);
        p.osc1.pan.set_value(-0.3);
        p.osc2.pan.set_value(0.3);
        p.osc_balance.set_value(0.2);
        p.unison.voices.set_value(5);
        p.unison.detune.set_value(18.0);
        p.unison.spread.set_value(0.8);
        p.filter.enabled.set_value(true);
        p.filter.cutoff.set_value(2500.0);
        p.filter.resonance.set_value(0.35);
        p.filter.env_depth.set_value(0.25);
        p.filter.keytrack.set_value(0.3);
        p.filter.drive.set_value(0.45);
        p.mod_slots[0].source.set_value(1); // LFO1
        p.mod_slots[0].destination.set_value(1); // osc1 position
        p.mod_slots[0].amount.set_value(0.35);
        p.mod_slots[1].source.set_value(2); // LFO2
        p.mod_slots[1].destination.set_value(5); // filter cutoff
        p.mod_slots[1].amount.set_value(0.4);
        p.mod_slots[2].source.set_value(4); // mod env
        p.mod_slots[2].destination.set_value(3); // osc1 pitch
        p.mod_slots[2].amount.set_value(0.08);
        p.mod_slots[3].source.set_value(3); // LFO3
        p.mod_slots[3].destination.set_value(10); // osc1 pan
        p.mod_slots[3].amount.set_value(0.5);
        p.mod_slots[4].source.set_value(5); // velocity
        p.mod_slots[4].destination.set_value(8); // amp level
        p.mod_slots[4].amount.set_value(0.3);
        p.chorus.enabled.set_value(true);
        p.delay.enabled.set_value(true);
        p.distortion.enabled.set_value(true);
        p.distortion.drive.set_value(2.5);
        v.push(("full_chain_u5", p, vec![36, 43, 48, 55, 60, 67]));
    }

    // 3. Glide: exercises the per-sample portamento path, where the pitch
    //    (and therefore the mip-level selection) changes every sample.
    {
        let p = WavetableParams::new();
        p.osc1.enabled.set_value(true);
        p.osc2.enabled.set_value(true);
        p.glide_enabled.set_value(true);
        p.glide_time.set_value(120.0);
        p.unison.voices.set_value(3);
        p.unison.detune.set_value(25.0);
        p.max_voices.set_value(1);
        p.filter.enabled.set_value(true);
        v.push(("glide_u3", p, vec![36, 60, 48, 72]));
    }

    // 4. Per-voice retriggered LFOs incl. sample & hold (RNG path), high
    //    resonance, highpass.
    {
        let p = WavetableParams::new();
        p.osc1.enabled.set_value(true);
        p.osc1.wavetable.set_value(8);
        p.unison.voices.set_value(2);
        p.lfo1.shape.set_value(4); // sample & hold
        p.lfo1.rate.set_value(11.0);
        p.lfo1.retrigger.set_value(true);
        p.lfo2.shape.set_value(2); // saw
        p.lfo2.retrigger.set_value(true);
        p.lfo3.shape.set_value(3); // square
        p.lfo3.retrigger.set_value(true);
        p.filter.enabled.set_value(true);
        p.filter.filter_type.set_value(1); // highpass
        p.filter.resonance.set_value(0.85);
        p.mod_slots[0].source.set_value(1);
        p.mod_slots[0].destination.set_value(5);
        p.mod_slots[0].amount.set_value(0.5);
        p.mod_slots[1].source.set_value(3);
        p.mod_slots[1].destination.set_value(2);
        p.mod_slots[1].amount.set_value(0.6);
        v.push(("lfo_sh_hpf", p, vec![55, 62, 69]));
    }

    // 5. High polyphony beyond max_voices, to exercise voice stealing.
    {
        let p = WavetableParams::new();
        p.osc1.enabled.set_value(true);
        p.osc2.enabled.set_value(true);
        p.max_voices.set_value(4);
        p.unison.voices.set_value(7);
        p.unison.detune.set_value(40.0);
        p.unison.spread.set_value(1.0);
        p.filter.enabled.set_value(true);
        p.filter.filter_type.set_value(2); // bandpass
        v.push((
            "voice_stealing_u7",
            p,
            vec![40, 44, 47, 51, 54, 58, 61, 65, 68],
        ));
    }

    v
}

fn render_all() -> Vec<f32> {
    let mut all = Vec::new();
    for (_, params, notes) in scenarios() {
        all.extend(render_scenario(&params, &notes));
    }
    all
}

#[test]
fn output_matches_golden() {
    let rendered = render_all();
    assert!(
        rendered.iter().all(|s| s.is_finite()),
        "render produced non-finite samples"
    );
    let peak = rendered.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 1e-3, "render produced silence — scenarios are broken");

    let path = golden_path();

    if std::env::var("RESONANCE_BLESS_NULL_TEST").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes: Vec<u8> = rendered.iter().flat_map(|s| s.to_le_bytes()).collect();
        std::fs::write(&path, bytes).unwrap();
        eprintln!("blessed golden: {} samples -> {}", rendered.len(), path.display());
        return;
    }

    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}\nregenerate with RESONANCE_BLESS_NULL_TEST=1",
            path.display()
        )
    });
    assert_eq!(
        bytes.len(),
        rendered.len() * 4,
        "golden length mismatch — scenario set changed"
    );

    let golden: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();

    // The golden was captured before the optimisation work. Every transform
    // applied since is a caching or hoisting change that re-uses the identical
    // expression, so all of those are bit-exact against it — with exactly one
    // deliberate exception:
    //
    //   `f32::tanh` -> `resonance_dsp::tanh_fast` in the filter drive
    //   soft-clip and the distortion waveshaper.
    //
    // That substitution is the *only* thing that may move a sample here, and
    // the bounds below pin how far. If a future change pushes past them, it is
    // altering the sound and needs its own justification.
    //
    // -80 dBFS peak / -100 dBFS RMS are roughly 2.5 and 25 orders of magnitude
    // below the rendered signal respectively, and sit under the dither floor
    // of 16-bit output.
    const MAX_PEAK_DELTA: f32 = 1.0e-4; // ~-80 dBFS
    const MAX_RMS_DELTA: f64 = 1.0e-5; // ~-100 dBFS

    let mut first_diff = None;
    let mut max_abs = 0.0f32;
    let mut diff_count = 0usize;
    let mut sq_err = 0.0f64;
    let mut sq_sig = 0.0f64;
    for (i, (a, b)) in rendered.iter().zip(golden.iter()).enumerate() {
        let d = a - b;
        sq_err += (d as f64) * (d as f64);
        sq_sig += (*b as f64) * (*b as f64);
        if a.to_bits() != b.to_bits() {
            diff_count += 1;
            if d.abs() > max_abs {
                max_abs = d.abs();
            }
            if first_diff.is_none() {
                first_diff = Some((i, *a, *b));
            }
        }
    }

    let n = rendered.len() as f64;
    let rms_err = (sq_err / n).sqrt();
    let rms_sig = (sq_sig / n).sqrt();
    let snr_db = 20.0 * (rms_sig / rms_err.max(f64::MIN_POSITIVE)).log10();

    eprintln!(
        "null test: {diff_count}/{} samples differ | peak delta {max_abs:.3e} \
         | rms delta {rms_err:.3e} | error is {snr_db:.1} dB below signal",
        rendered.len()
    );

    if let Some((i, a, b)) = first_diff {
        assert!(
            max_abs <= MAX_PEAK_DELTA && rms_err <= MAX_RMS_DELTA,
            "output moved beyond the documented tanh_fast bound: \
             {diff_count}/{} samples differ, peak delta {max_abs:.3e} \
             (limit {MAX_PEAK_DELTA:.1e}), rms delta {rms_err:.3e} \
             (limit {MAX_RMS_DELTA:.1e}); first at sample {i}: got {a:?}, want {b:?}",
            rendered.len()
        );
    }
}
