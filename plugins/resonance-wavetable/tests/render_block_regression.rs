//! Bit-exact regression guard for `SynthEngine::render_block`.
//!
//! Where `null_test.rs` allows a small documented tolerance (it predates the
//! `tanh_fast` substitution), this test is deliberately *bit*-exact: every
//! rendered sample must match the golden by `f32::to_bits`. It exists to pin
//! the output of `render_block` across structural refactors of the render
//! path — splitting the kernel into helpers must not move a single bit.
//!
//! The scenarios are chosen to hit each seam of that decomposition:
//! the parameter snapshot (including mid-run parameter edits between blocks),
//! voice allocation and stealing, the oscillator/unison inner kernel, the
//! control-rate filter-coefficient refresh and its `filter_dirty` force path,
//! the modulation matrix, the global and per-voice LFOs (including the
//! sample & hold RNG path), the block effects chain, and the smoother
//! fast-forward taken when an effect is disabled.
//!
//! Every scenario but `init_single` spells out the parameters it depends on
//! rather than leaning on `WavetableParams::new()`'s defaults, so a change to
//! a *default* cannot quietly change what this test renders. ba todo #1354
//! is why, and it caught this file out twice: moving `filter_cutoff`'s
//! default to 20 kHz turned `lfo_sh_hpf` (a highpass at the default cutoff)
//! into silence, and taking the `lfoN_depth` defaults to 0.0 left mod slots
//! 0, 1 and 3 of `full_chain_u5` — the scenario that exists to pin the
//! modulation matrix — permanently inert while it went on rendering a
//! healthy 0.83 peak. Only the second round of that fix made the golden
//! move for `init_single` alone.
//!
//! `init_single` is the deliberate exception: it exists to render the
//! default patch, so it does follow the declarations.
//!
//! To (re)generate the golden after an *intentional* audio change:
//!
//!     RESONANCE_BLESS_RENDER_BLOCK=1 cargo test -p resonance-wavetable \
//!         --no-default-features --test render_block_regression
//!
//! A pure refactor must never need this.

use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_plugin::{EventIterator, NoteEvent};
use resonance_wavetable::dsp::engine::SynthEngine;
use resonance_wavetable::params::WavetableParams;

const SR: f32 = 48_000.0;
/// Deliberately not a multiple of the control-rate interval, so blocks end
/// mid-way through the coefficient grid.
const BLOCK: usize = 100;
const BLOCKS: usize = 40;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "render_block_regression.f32")
}

/// A parameter edit applied *between* blocks, so the next block's snapshot
/// picks it up. `None` for scenarios with static parameters.
type MidRunEdit = fn(&WavetableParams, usize);

struct Scenario {
    name: &'static str,
    params: WavetableParams,
    notes: Vec<u8>,
    edit: Option<MidRunEdit>,
}

/// Deterministic render of one scenario. Note-ons are staggered one per block
/// at non-zero sample offsets so the sample-accurate event drain is used, and
/// released in the second half so the release tail and the
/// `Releasing -> Idle` transition are covered.
fn render_scenario(s: &Scenario) -> Vec<f32> {
    let mut engine = SynthEngine::new();
    engine.initialize(SR);

    let mut out = Vec::with_capacity(BLOCKS * BLOCK * 2);
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];

    for block in 0..BLOCKS {
        if let Some(edit) = s.edit {
            edit(&s.params, block);
        }

        let mut events: Vec<NoteEvent> = Vec::new();
        if block < s.notes.len() {
            events.push(NoteEvent::NoteOn {
                note: s.notes[block],
                velocity: 0.31 + 0.07 * block as f32,
                timing: (block * 13 % BLOCK) as u32,
            });
        }
        let release_start = BLOCKS / 2;
        if block >= release_start && block - release_start < s.notes.len() {
            events.push(NoteEvent::NoteOff {
                note: s.notes[block - release_start],
                timing: (block * 17 % BLOCK) as u32,
            });
        }

        let mut iter = EventIterator::new(&events);
        engine.render_block(&mut left, &mut right, BLOCK, &s.params, &mut iter, None);
        out.extend_from_slice(&left);
        out.extend_from_slice(&right);
    }
    out
}

fn scenarios() -> Vec<Scenario> {
    let mut v = Vec::new();

    // 1. Init patch: single oscillator, one note. The plainest path through
    //    the kernel — snapshot, one voice, no filter, no effects.
    v.push(Scenario {
        name: "init_single",
        params: WavetableParams::new(),
        notes: vec![60],
        edit: None,
    });

    // 2. Whole signal chain at once: both oscillators, unison, driven filter,
    //    five mod-matrix slots, distortion + chorus + delay.
    {
        let p = WavetableParams::new();
        p.osc1.enabled.set_value(true);
        p.osc2.enabled.set_value(true);
        p.osc1.wavetable.set_value(2);
        p.osc2.wavetable.set_value(5);
        p.osc1.position.set_value(0.29);
        p.osc2.position.set_value(0.73);
        p.osc2.coarse.set_value(-12);
        p.osc1.fine.set_value(-9.0);
        p.osc2.fine.set_value(4.0);
        p.osc1.pan.set_value(-0.4);
        p.osc2.pan.set_value(0.35);
        p.osc_balance.set_value(-0.15);
        p.unison.voices.set_value(5);
        p.unison.detune.set_value(22.0);
        p.unison.spread.set_value(0.7);
        p.filter.enabled.set_value(true);
        p.filter.cutoff.set_value(1800.0);
        p.filter.resonance.set_value(0.45);
        p.filter.env_depth.set_value(0.3);
        p.filter.keytrack.set_value(0.4);
        p.filter.drive.set_value(0.6);
        // Spelled out rather than left to the declared defaults, which ba
        // todo #1354 took to zero: mod slots 0, 1 and 3 below all scale by
        // `lfoN_depth`, so without these three lines the scenario that
        // exists to pin the modulation matrix stops driving it.
        p.lfo1.depth.set_value(0.5);
        p.lfo2.depth.set_value(0.3);
        p.lfo3.depth.set_value(0.3);
        p.mod_slots[0].source.set_value(1); // LFO1
        p.mod_slots[0].destination.set_value(1); // osc1 position
        p.mod_slots[0].amount.set_value(0.3);
        p.mod_slots[1].source.set_value(2); // LFO2
        p.mod_slots[1].destination.set_value(5); // filter cutoff
        p.mod_slots[1].amount.set_value(0.45);
        p.mod_slots[2].source.set_value(4); // mod env
        p.mod_slots[2].destination.set_value(3); // osc1 pitch
        p.mod_slots[2].amount.set_value(0.06);
        p.mod_slots[3].source.set_value(3); // LFO3
        p.mod_slots[3].destination.set_value(10); // osc1 pan
        p.mod_slots[3].amount.set_value(0.4);
        p.mod_slots[4].source.set_value(5); // velocity
        p.mod_slots[4].destination.set_value(8); // amp level
        p.mod_slots[4].amount.set_value(0.25);
        p.distortion.enabled.set_value(true);
        p.distortion.drive.set_value(3.0);
        p.chorus.enabled.set_value(true);
        p.delay.enabled.set_value(true);
        v.push(Scenario {
            name: "full_chain_u5",
            params: p,
            notes: vec![36, 43, 48, 55, 60, 67],
            edit: None,
        });
    }

    // 3. Glide: `current_pitch` moves every sample, so the cached per-unison
    //    `OscSetup` is rebuilt on every sample of the kernel.
    {
        let p = WavetableParams::new();
        p.osc1.enabled.set_value(true);
        p.osc2.enabled.set_value(true);
        p.glide_enabled.set_value(true);
        p.glide_time.set_value(90.0);
        p.unison.voices.set_value(3);
        p.unison.detune.set_value(30.0);
        p.max_voices.set_value(1);
        p.filter.enabled.set_value(true);
        p.filter.cutoff.set_value(8000.0);
        v.push(Scenario {
            name: "glide_u3",
            params: p,
            notes: vec![36, 60, 48, 72],
            edit: None,
        });
    }

    // 4. Per-voice retriggered LFOs including sample & hold (the RNG path),
    //    high resonance, highpass.
    {
        let p = WavetableParams::new();
        p.osc1.enabled.set_value(true);
        p.osc1.wavetable.set_value(7);
        p.unison.voices.set_value(2);
        p.lfo1.shape.set_value(4); // sample & hold
        p.lfo1.rate.set_value(9.5);
        p.lfo1.depth.set_value(0.5);
        p.lfo1.retrigger.set_value(true);
        p.lfo2.shape.set_value(2); // saw
        p.lfo2.depth.set_value(0.3);
        p.lfo2.retrigger.set_value(true);
        p.lfo3.shape.set_value(3); // square
        p.lfo3.depth.set_value(0.3);
        p.lfo3.retrigger.set_value(true);
        p.filter.enabled.set_value(true);
        p.filter.filter_type.set_value(1); // highpass
        // Low enough that the notes' upper partials pass: at 8 kHz the
        // highpass left a 1.2e-3 peak (FU-G2c).
        p.filter.cutoff.set_value(500.0);
        p.filter.resonance.set_value(0.8);
        p.mod_slots[0].source.set_value(1);
        p.mod_slots[0].destination.set_value(5);
        p.mod_slots[0].amount.set_value(0.55);
        p.mod_slots[1].source.set_value(3);
        p.mod_slots[1].destination.set_value(2);
        p.mod_slots[1].amount.set_value(0.5);
        v.push(Scenario {
            name: "lfo_sh_hpf",
            params: p,
            notes: vec![55, 62, 69],
            edit: None,
        });
    }

    // 5. More notes than `max_voices`, exercising every branch of the voice
    //    stealing ladder.
    {
        let p = WavetableParams::new();
        p.osc1.enabled.set_value(true);
        p.osc2.enabled.set_value(true);
        p.max_voices.set_value(4);
        p.unison.voices.set_value(7);
        p.unison.detune.set_value(38.0);
        p.unison.spread.set_value(1.0);
        p.filter.enabled.set_value(true);
        p.filter.filter_type.set_value(2); // bandpass
        p.filter.cutoff.set_value(8000.0);
        v.push(Scenario {
            name: "voice_stealing_u7",
            params: p,
            notes: vec![40, 44, 47, 51, 54, 58, 61, 65, 68],
            edit: None,
        });
    }

    // 6. Parameters edited between blocks: the snapshot must be re-taken per
    //    block, the smoothers retargeted, and — when an effect is switched
    //    off — its smoothers fast-forwarded instead of replaying a stale ramp
    //    on re-enable. Also toggles the oscillators, covering the `oscs_active`
    //    skip and the wavetable-index resolution.
    {
        let p = WavetableParams::new();
        p.osc1.enabled.set_value(true);
        p.osc2.enabled.set_value(true);
        p.filter.enabled.set_value(true);
        p.unison.voices.set_value(3);
        p.unison.detune.set_value(15.0);
        p.distortion.enabled.set_value(true);
        p.chorus.enabled.set_value(true);
        p.delay.enabled.set_value(true);
        v.push(Scenario {
            name: "param_edits_between_blocks",
            params: p,
            notes: vec![48, 55, 60],
            edit: Some(|p, block| {
                // Continuous parameters sweep, so the per-sample smoothers
                // are always mid-ramp at a block boundary.
                let t = block as f32 / BLOCKS as f32;
                p.master_volume.set_value(0.2 + 0.6 * t);
                p.filter.cutoff.set_value(400.0 + 6000.0 * t);
                p.filter.resonance.set_value(0.1 + 0.7 * t);
                p.osc1.position.set_value(t);
                p.osc2.position.set_value(1.0 - t);
                p.distortion.drive.set_value(1.0 + 4.0 * t);
                p.delay.time_l.set_value(80.0 + 200.0 * t);
                p.delay.time_r.set_value(260.0 - 150.0 * t);
                p.chorus.depth.set_value(t);

                // Effects switch off for the middle third and back on, which
                // is the only path that reaches the `skip()` fast-forward.
                let fx_on = !(BLOCKS / 3..2 * BLOCKS / 3).contains(&block);
                p.distortion.enabled.set_value(fx_on);
                p.chorus.enabled.set_value(fx_on);
                p.delay.enabled.set_value(fx_on);

                // Oscillators go silent for a stretch; voice lifecycle,
                // LFO phases and filter ring-down must keep advancing.
                p.osc2.enabled.set_value(block % 7 != 3);
                p.osc1.enabled.set_value(!(BLOCKS / 4..BLOCKS / 4 + 5).contains(&block));

                // Swapping the wavetable re-resolves the table reference and
                // invalidates every cached per-unison `OscSetup`.
                p.osc1.wavetable.set_value(if block % 11 == 5 { 4 } else { 1 });
            }),
        });
    }

    v
}

fn render_all() -> Vec<f32> {
    let mut all = Vec::new();
    for s in scenarios() {
        all.extend(render_scenario(&s));
    }
    all
}

#[test]
fn render_block_output_is_bit_exact() {
    let rendered = render_all();
    assert!(
        rendered.iter().all(|s| s.is_finite()),
        "render produced non-finite samples"
    );
    let peak = rendered.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 1e-3, "render produced silence — scenarios are broken");

    let path = golden_path();

    if golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_RENDER_BLOCK"]) {
        golden::bless_f32(&path, &rendered);
        return;
    }

    let want = golden::load_golden_f32(&path, rendered.len(), "RESONANCE_BLESS_RENDER_BLOCK=1");
    let diff = golden::compare_f32(&rendered, &want);

    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "render_block output changed: {}/{} samples differ bitwise; \
             first at sample {i} (scenario block {}, frame {}): got {got:?} ({:#010x}), \
             want {want:?} ({:#010x}). A refactor of the render path must be bit-exact.",
            diff.diff_count,
            rendered.len(),
            i / (BLOCK * 2),
            i % (BLOCK * 2),
            got.to_bits(),
            want.to_bits(),
        );
    }
}

/// Per-scenario peak floor (−26 dBFS). "Not silent" is not enough: a
/// scenario idling at a 1e-3 peak (as `lfo_sh_hpf` did, its highpass
/// removing nearly everything) pins mostly rounding noise, so a real DSP
/// change can move it by less than it moves the loud ones (FU-G2c).
const MIN_SCENARIO_PEAK: f32 = 0.05;

/// Guards the scenario table itself: every scenario must contribute audio
/// at a meaningful level, so a future edit cannot silently turn one into
/// a (near-)no-op that the golden then happily matches.
#[test]
fn every_scenario_renders_audio() {
    for s in scenarios() {
        let out = render_scenario(&s);
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(
            peak > MIN_SCENARIO_PEAK,
            "scenario `{}` peaks at {peak:.2e}, below {MIN_SCENARIO_PEAK}",
            s.name
        );
        assert!(
            out.iter().all(|x| x.is_finite()),
            "scenario `{}` rendered non-finite samples",
            s.name
        );
    }
}
