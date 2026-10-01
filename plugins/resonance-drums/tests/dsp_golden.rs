//! Bit-exact DSP golden for the drum sampler (ba todo #1373).
//!
//! `tests/sampler.rs`, `tests/round_robin.rs` and `tests/group_balance.rs`
//! pin *decisions*: which velocity layer, which take, which output port,
//! how loud one group is relative to another. None of that notices a
//! change to the per-sample voice mixer, the pan law, the release fade,
//! the choke ramp or the per-block parameter interpolation — the kit
//! would sound different and every assertion would still hold. This test
//! pins the rendered samples.
//!
//! # The kit: the bundled fallback, not Drummica
//!
//! The flagship kit is an 8.5 GB third-party sample library that lives
//! behind `RESONANCE_DRUMMICA_PATH` (see `tests/kit_loader.rs`), so a
//! golden over it would skip on every machine that has not downloaded
//! it — and a test that quietly skips is worse than one that covers
//! less. This golden therefore drives the **bundled fallback kit**:
//! `DrumSampler::load_defaults` decodes the WAVs compiled into the
//! binary, so it is byte-identical everywhere and needs nothing on
//! disk.
//!
//! What that leaves uncovered, explicitly:
//!
//! - the multi-mic path. The fallback kit gives every pad a single
//!   close-mic bank and **no overhead bank**, so the Overhead port is
//!   silent here and the kick/snare second close-mic trim has no bank
//!   to scale. `tests/group_balance.rs` documents the same
//!   limitation.
//! - multiple velocity layers and multiple round-robin takes. The
//!   fallback has one layer with one take per pad, so `pick_rr` and
//!   `pick_velocity_layer` always return 0 — their *selection* logic is
//!   covered by `tests/round_robin.rs`, but the audible difference
//!   between takes is not pinned by any golden.
//! - `kit_loader`'s manifest parsing and the loader thread.
//!
//! Everything below the voice's sample choice — the mixer, the velocity
//! curve, panning, the ramps, choking, voice stealing and the release
//! fade — is covered.
//!
//! # The input, and why it is the revealing one
//!
//! The driver is a **drum pattern**, not a tone: this is a sampler, and
//! its input is MIDI. The pattern strikes every output group (kick,
//! snare, toms, hats, cymbals) across the full velocity range, with
//! hi-hat open/closed pairs so the choke group fires, and enough
//! simultaneous hits to reach the voice-stealing ladder in the
//! polyphony scenario. A pattern that only hit one pad at one velocity
//! would pin one sample playing back at unity and nothing else.
//!
//! # What is stored, and why it is not all seven ports
//!
//! The plugin renders seven stereo ports. Storing all fourteen channels
//! raw would make the fixture several megabytes for no extra coverage,
//! so the golden holds two things per block, in the same shape
//! `resonance-granular-delay`'s golden uses:
//!
//! - the **kit mix** (the sum of all seven ports) as raw f32, which is
//!   what a listener hears and what a diff is readable against, and
//!   - a **per-block FNV-1a digest of every port's samples**, so a
//!   routing change that moves a voice from one port to another — which
//!   the mix alone would not see — still fails.
//!
//! # Tolerance: bit-exact
//!
//! Sample playback is multiply-accumulate with linear parameter ramps
//! and a fixed-seed xorshift for the Random round-robin mode. No FFT,
//! no runtime SIMD dispatch, no unseeded RNG, no clock. (The bundled
//! WAVs are decoded and resampled at load; that is deterministic too.)
//!
//! To (re)generate the golden after an *intentional* audio change:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-drums --test dsp_golden
//!
//! A pure refactor must never need this.

use std::path::PathBuf;

use resonance_dsp_test_support as golden;
use resonance_drums::drum_map::{
    CRASH_16_EDGE, HIHAT_CLOSED, HIHAT_OPEN, HIHAT_PEDAL, KICK, PAD_MAPPINGS, RIDE_TIP, RIMSHOT,
    SNARE, TOM_HIGH, TOM_LOW, TOM_MID,
};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{LoadedPad, NUM_OUTPUT_PORTS};
use resonance_drums::level::gain_to_db;
use resonance_drums::params::{DrumParams, MicSlot, OUTPUT_MODE_MULTI, OUTPUT_MODE_STEREO};
use resonance_drums::voice::MAX_VOICES;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
/// Blocks per scenario — 32 × 256 = 8192 samples, ~170 ms. Long enough
/// for every hit in the pattern to sound and for the last ones to be
/// well into their decay.
const BLOCKS: usize = 32;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "dsp_golden.u32")
}

/// `RESONANCE_BLESS=1` is the workspace-wide convention (CLAUDE.md); the
/// narrower name blesses only this file inside a wider run.
fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_DSP_GOLDEN"])
}

/// FNV-1a over a word sequence — folds each block's full seven-port
/// output into two golden words. A single bit anywhere in any port
/// changes the digest.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, word: u32) {
        for b in word.to_le_bytes() {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(self) -> [u32; 2] {
        [self.0 as u32, (self.0 >> 32) as u32]
    }
}

/// One MIDI hit: which block it lands on, the note, and the velocity.
/// Block granularity is what the plugin itself uses — `lib.rs` drains
/// the whole event queue before rendering — so nothing is lost by
/// expressing the pattern this way.
#[derive(Clone, Copy)]
struct Hit {
    block: usize,
    note: u8,
    vel: f32,
}

const fn hit(block: usize, note: u8, vel: f32) -> Hit {
    Hit { block, note, vel }
}

/// A dense fill that reaches every output group: kick, snare, rimshot,
/// closed hats, three toms, a crash and a ride, with velocities from
/// 0.15 to 1.0 so the velocity curve has its full range to bend.
///
/// Hits land on block boundaries, which at 256 frames is one every
/// 5.3 ms — far faster than anyone plays. That is deliberate: at this
/// spacing every hit is still sounding when the next few arrive, so the
/// per-sample voice mixer is summing five to ten voices for most of the
/// render instead of one. A realistically spaced groove would leave the
/// mixer summing a single decaying voice nearly all the time and pin
/// much less.
const GROOVE: &[Hit] = &[
    hit(0, KICK, 1.0),
    hit(0, HIHAT_CLOSED, 0.55),
    hit(2, HIHAT_CLOSED, 0.28),
    hit(4, SNARE, 0.85),
    hit(4, HIHAT_CLOSED, 0.5),
    hit(6, HIHAT_CLOSED, 0.25),
    hit(7, KICK, 0.7),
    hit(8, KICK, 0.95),
    hit(8, HIHAT_CLOSED, 0.6),
    hit(10, HIHAT_CLOSED, 0.3),
    hit(11, RIMSHOT, 0.45),
    hit(12, SNARE, 1.0),
    hit(12, CRASH_16_EDGE, 0.8),
    hit(14, HIHAT_CLOSED, 0.35),
    hit(16, TOM_HIGH, 0.7),
    hit(18, TOM_MID, 0.75),
    hit(20, TOM_LOW, 0.8),
    hit(22, KICK, 0.9),
    hit(22, RIDE_TIP, 0.5),
    hit(24, SNARE, 0.15),
    hit(26, RIDE_TIP, 0.4),
    hit(28, KICK, 0.65),
    hit(28, CRASH_16_EDGE, 0.6),
];

/// Open hi-hat followed by a closed hat and a pedal, twice. Both
/// closers are in the same choke group as the open hat, so each one has
/// to cut the ringing voice — the fade that does it is a per-sample
/// ramp no other test renders.
const HIHAT_CHOKES: &[Hit] = &[
    hit(0, HIHAT_OPEN, 0.9),
    hit(6, HIHAT_CLOSED, 0.7),
    hit(10, HIHAT_OPEN, 0.85),
    hit(16, HIHAT_PEDAL, 0.6),
    hit(20, HIHAT_OPEN, 0.8),
    hit(26, HIHAT_CLOSED, 0.75),
];

/// Ten pads struck on the same block, twice over. With the polyphony
/// ceiling turned down this walks the whole voice-stealing ladder.
const PILE_UP: &[Hit] = &[
    hit(0, KICK, 0.9),
    hit(0, SNARE, 0.9),
    hit(0, TOM_HIGH, 0.9),
    hit(0, TOM_MID, 0.9),
    hit(0, TOM_LOW, 0.9),
    hit(1, HIHAT_CLOSED, 0.8),
    hit(1, RIDE_TIP, 0.8),
    hit(1, CRASH_16_EDGE, 0.8),
    hit(1, RIMSHOT, 0.8),
    hit(2, HIHAT_OPEN, 0.8),
    hit(12, KICK, 1.0),
    hit(12, SNARE, 1.0),
    hit(12, TOM_HIGH, 1.0),
    hit(12, TOM_MID, 1.0),
    hit(12, TOM_LOW, 1.0),
    hit(13, CRASH_16_EDGE, 1.0),
    hit(13, RIDE_TIP, 1.0),
];

/// A parameter edit applied *between* blocks, so the next block's
/// snapshot picks it up and ramps toward it. `None` for static
/// scenarios.
type MidRunEdit = fn(&DrumParams, usize);

struct Scenario {
    name: &'static str,
    pattern: &'static [Hit],
    /// Applied after [`pin`].
    setup: fn(&DrumParams),
    edit: Option<MidRunEdit>,
    /// Where the scenario must be heard — its own guard against
    /// rendering silence (or the wrong routing) into the golden.
    ports: Ports,
}

enum Ports {
    /// Sounds on at least this many ports.
    AtLeast(usize),
    /// Sounds on Main, and nowhere else.
    MainOnly,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // 1. The groove at factory settings. The everyday sound of the
        //    plugin: every group, the full velocity range, the default
        //    linear velocity curve and Cycle round robin.
        Scenario {
            name: "groove_factory_defaults",
            pattern: GROOVE,
            setup: |_| {},
            edit: None,
            ports: Ports::AtLeast(3),
        },
        // 2. Velocity curve pushed hard (soft end). The curve is an
        //    exact identity at 0, so it is the one global control that
        //    a refactor can neutralise without any behavioural test
        //    noticing — the pads still sound, just at the wrong levels.
        Scenario {
            name: "groove_soft_velocity_curve",
            pattern: GROOVE,
            setup: |p| p.velocity_curve.set_value(1.0),
            edit: None,
            ports: Ports::AtLeast(3),
        },
        // 3. …and the hard end, which bends the same hits the other
        //    way. Two scenarios rather than one because the curve is
        //    signed and a sign error is otherwise invisible.
        Scenario {
            name: "groove_hard_velocity_curve",
            pattern: GROOVE,
            setup: |p| p.velocity_curve.set_value(-1.0),
            edit: None,
            ports: Ports::AtLeast(3),
        },
        // 4. Hi-hat choke group. An open hat that is cut by a closed
        //    hat is a ramp applied mid-decay; get the ramp length or
        //    the trigger point wrong and it clicks.
        Scenario {
            name: "hihat_choke_group",
            pattern: HIHAT_CHOKES,
            setup: |_| {},
            edit: None,
            // Deliberately one group.
            ports: Ports::AtLeast(1),
        },
        // 5. Ten simultaneous hits against a polyphony ceiling of three.
        //    Every stolen voice is a fade-out on a sounding sample,
        //    which is exactly the kind of thing that sounds "fine" in a
        //    behavioural test and different to a listener.
        Scenario {
            name: "voice_stealing_polyphony_3",
            pattern: PILE_UP,
            setup: |p| p.polyphony.set_value(3),
            edit: None,
            ports: Ports::AtLeast(3),
        },
        // 6. Random round robin. The mode is driven by a fixed-seed
        //    xorshift precisely so a render is reproducible — bouncing
        //    the same project twice must give the same takes — and this
        //    golden is what pins that promise. (On the fallback kit
        //    there is only one take per pad, so what is pinned here is
        //    that the random path renders identically to the cycle
        //    path; on a multi-take kit it would diverge.)
        Scenario {
            name: "random_round_robin_is_seeded",
            pattern: GROOVE,
            setup: |p| p.round_robin_mode.set_value(1),
            edit: None,
            ports: Ports::AtLeast(3),
        },
        // 7. Master volume, per-pad volume, pan and mute all moving
        //    between blocks. Each is linearly interpolated across the
        //    block from the previous snapshot; that ramp is a declick
        //    measure with no behavioural signature at all — it is
        //    audible and otherwise untested.
        Scenario {
            name: "param_ramps_between_blocks",
            pattern: GROOVE,
            setup: |_| {},
            edit: Some(|p, block| {
                let t = block as f32 / BLOCKS as f32;
                // Levels are dB (E9): the same gain sweeps as before,
                // given in dB, so the ramps still run between gains.
                p.master_volume.set_value(gain_to_db(0.2 + 0.7 * t));
                // Sweep the kick's and snare's pads in opposite
                // directions so the pan law is exercised across its
                // whole range in one run.
                for (pad, dir) in [(0usize, 1.0f32), (1usize, -1.0f32)] {
                    let pp = &p.pads[pad];
                    pp.volume.set_value(gain_to_db(0.3 + 0.7 * t));
                    pp.pan.set_value(dir * (2.0 * t - 1.0));
                    // The fallback kit's one close bank is mic 1.
                    pp.trim(MicSlot::Close1).set_value(-6.0 + 6.0 * t);
                }
                // A pad muting and unmuting mid-run: mute folds into
                // the volume snapshot, so it ramps rather than cutting.
                p.pads[4].mute.set_value(block % 10 >= 5);
            }),
            ports: Ports::AtLeast(3),
        },
        // 8. Stereo output mode (E11, the plugin's default): the whole
        //    groove summed to Main, every other port silent. The seven
        //    scenarios above run in Multi so they keep pinning the ports.
        Scenario {
            name: "groove_stereo_output",
            pattern: GROOVE,
            setup: |p| p.output_mode.set_value(OUTPUT_MODE_STEREO),
            edit: None,
            ports: Ports::MainOnly,
        },
    ]
}

/// Every param a scenario does not set itself, pinned to a stated value
/// rather than left to whatever the defaults are next year (a default
/// that moves must not silently re-shape a scenario): unity levels,
/// linear velocity, Cycle, full polyphony, the Drummica choke groups and
/// ports, and **Multi** output so the per-port digests see the routing.
fn pin(p: &DrumParams) {
    p.master_volume.set_value(0.0);
    p.polyphony.set_value(MAX_VOICES as i32);
    p.velocity_curve.set_value(0.0);
    p.round_robin_mode.set_value(0);
    p.output_mode.set_value(OUTPUT_MODE_MULTI);
    for (pad, mapping) in p.pads.iter().zip(PAD_MAPPINGS.iter()) {
        pad.volume.set_value(0.0);
        pad.pan.set_value(0.0);
        pad.mute.set_value(false);
        for trim in &pad.trims {
            trim.set_value(0.0);
        }
        pad.choke.set_value(mapping.choke_group.map_or(0, i32::from));
        pad.output.set_value(mapping.output_group.index() as i32);
    }
}

/// Deterministic render of one scenario into the golden word stream:
/// per frame the two summed-mix samples' bit patterns, and per block a
/// digest of every output port.
fn render_scenario(s: &Scenario) -> Vec<u32> {
    let params = DrumParams::default();
    pin(&params);
    (s.setup)(&params);

    // The kit receiver never receives anything: this scenario runs on
    // the bundled fallback kit alone, so there is no loader thread and
    // nothing to swap in.
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.load_defaults(SR);
    sampler.reset();

    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]))
        .collect();

    let mut out = Vec::new();
    for block in 0..BLOCKS {
        if let Some(edit) = s.edit {
            edit(&params, block);
        }
        sampler.update_global_settings(&params);
        for h in s.pattern.iter().filter(|h| h.block == block) {
            sampler.note_on(h.note, h.vel);
        }
        {
            let mut ports: Vec<PortBuffers<'_>> = bufs
                .iter_mut()
                .map(|(l, r)| PortBuffers {
                    left: l.as_mut_slice(),
                    right: r.as_mut_slice(),
                })
                .collect();
            sampler.render_block(&mut ports, BLOCK, &params, &[]);
        }

        // The kit mix: what a host summing every port would hear.
        for i in 0..BLOCK {
            let mut l = 0.0f32;
            let mut r = 0.0f32;
            for (pl, pr) in &bufs {
                l += pl[i];
                r += pr[i];
            }
            // Checked here rather than over the word stream: the digest
            // words that follow are hashes, and a hash is free to look
            // like a NaN.
            assert!(
                l.is_finite() && r.is_finite(),
                "scenario `{}` rendered a non-finite sample in block {block}",
                s.name
            );
            out.push(l.to_bits());
            out.push(r.to_bits());
        }

        // …and a digest over every port separately, so moving a voice
        // between ports (which the mix cannot see) still fails.
        let mut digest = Fnv::new();
        for (pl, pr) in &bufs {
            for i in 0..BLOCK {
                digest.write(pl[i].to_bits());
                digest.write(pr[i].to_bits());
            }
        }
        out.extend_from_slice(&digest.finish());
    }
    out
}

fn render_all() -> Vec<u32> {
    let mut all = Vec::new();
    for s in scenarios() {
        all.extend(render_scenario(&s));
    }
    all
}

#[test]
fn drums_output_is_bit_exact() {
    // Finiteness is asserted inside `render_scenario`, on the samples
    // themselves — the word stream also carries digest hashes, which
    // are allowed to look like anything.
    let rendered = render_all();

    let path = golden_path();
    if blessing() {
        golden::bless_words(&path, &rendered);
        return;
    }

    let want = golden::load_golden_words(&path, rendered.len(), "RESONANCE_BLESS=1");
    let diff = golden::compare_words(&rendered, &want);

    if let Some((i, got, want)) = diff.first_diff {
        panic!(
            "drum sampler output changed: {}/{} words differ; first at \
             word {i} (got {got:#010x}, want {want:#010x}; as f32: {} vs {}).\nA \
             refactor of the sampler must be bit-exact. If the change was \
             intended, re-bless with RESONANCE_BLESS=1.\nA difference confined to \
             the two digest words at the end of a block means a voice moved \
             between output ports without changing the mix.",
            diff.diff_count,
            rendered.len(),
            f32::from_bits(got),
            f32::from_bits(want),
        );
    }
}

/// Guards the scenario table and the kit behind it: every scenario must
/// render audio on more than one output port, so a fallback kit that
/// silently stopped decoding — or a routing change that funnelled
/// everything into one port — fails here rather than being blessed into
/// the golden.
#[test]
fn every_scenario_sounds_on_several_ports() {
    for s in scenarios() {
        let params = DrumParams::default();
        pin(&params);
        (s.setup)(&params);
        let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
        let mut sampler = DrumSampler::new(rx);
        sampler.load_defaults(SR);
        sampler.reset();

        let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
            .map(|_| (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]))
            .collect();
        let mut port_peak = [0.0f32; NUM_OUTPUT_PORTS];

        for block in 0..BLOCKS {
            if let Some(edit) = s.edit {
                edit(&params, block);
            }
            sampler.update_global_settings(&params);
            for h in s.pattern.iter().filter(|h| h.block == block) {
                sampler.note_on(h.note, h.vel);
            }
            {
                let mut ports: Vec<PortBuffers<'_>> = bufs
                    .iter_mut()
                    .map(|(l, r)| PortBuffers {
                        left: l.as_mut_slice(),
                        right: r.as_mut_slice(),
                    })
                    .collect();
                sampler.render_block(&mut ports, BLOCK, &params, &[]);
            }
            for (p, (l, r)) in bufs.iter().enumerate() {
                for (a, b) in l.iter().zip(r.iter()) {
                    assert!(
                        a.is_finite() && b.is_finite(),
                        "scenario `{}` rendered a non-finite sample on port {p}",
                        s.name
                    );
                    port_peak[p] = port_peak[p].max(a.abs()).max(b.abs());
                }
            }
        }

        let sounding = port_peak.iter().filter(|p| **p > 1e-4).count();
        match s.ports {
            Ports::AtLeast(want) => assert!(
                sounding >= want,
                "scenario `{}` sounded on only {sounding} of {NUM_OUTPUT_PORTS} ports \
                 (peaks {port_peak:?}) — the fallback kit or the port routing is \
                 broken, and the golden would pin the broken version",
                s.name
            ),
            Ports::MainOnly => {
                assert!(
                    port_peak[0] > 1e-4,
                    "scenario `{}` is silent on Main (peaks {port_peak:?})",
                    s.name
                );
                assert_eq!(
                    sounding, 1,
                    "scenario `{}` must sound on Main only (peaks {port_peak:?})",
                    s.name
                );
            }
        }
    }
}
