//! Bit-exact regression guard for the granular-delay DSP core
//! (ba todo #1264).
//!
//! Every rendered sample must match the golden by `f32::to_bits`, and
//! every block's published viz state (packed grain slots + coarse peak
//! bins) must match word for word. The test exists to pin the audible
//! and editor-visible output of `GranularDsp::process_block` /
//! `publish_viz` across structural refactors of the DSP path — splitting
//! the core into a module tree and grouping its state into sub-structs
//! must not move a single bit.
//!
//! The scenarios walk every seam of that decomposition: the buffer write
//! and its freeze crossfade, the peak-bin mip, all three time modes
//! (including mid-run Time changes, so the Fade swap and the Repitch
//! slew both land inside a block), the pitch-sync PSOLA path and its
//! voice crossfade, all three feedback routes, the decorrelated stereo
//! engine, the un-transposed feedback tap, per-grain pitch quantization,
//! all three quality tiers and the tempo-sync delay resolution. Host
//! block sizes vary per scenario so the `fb_len` prefix logic and the
//! per-slice loop are exercised at several lengths.
//!
//! To (re)generate the golden after an *intentional* audio change:
//!
//!     RESONANCE_BLESS_GRANULAR_DSP=1 cargo test -p resonance-granular-delay \
//!         --test dsp_regression
//!
//! A pure refactor must never need this.

use std::path::PathBuf;

use resonance_granular_delay::params::GranularDelayParams;
use resonance_granular_delay::viz::{pack_grain, GrainSnapshot, GRAIN_SLOTS, PEAK_BINS};
use resonance_granular_delay::ResonanceGranularDelay;
use resonance_plugin::{EventIterator, OutputBuffer, ResonancePlugin, TempoInfo};

const SR: f32 = 48_000.0;
/// Blocks rendered per scenario.
const BLOCKS: usize = 96;
/// Blocks rendered before the audio capture starts. The grain tap reads
/// `time_ms` behind the write head, so until the ring holds at least one
/// delay of material the wet bus is silent and the output is pure dry —
/// every scenario keeps its delay well under `PRIME_BLOCKS` × 64 samples
/// (~64 ms) so the captured half is genuinely granulated. The viz digest
/// is captured from the first block (it is two words per block).
const PRIME_BLOCKS: usize = 48;
/// Largest block a scenario may ask for (the activation buffer size).
const MAX_BLOCK: usize = 96;

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/dsp_regression.u32")
}

/// FNV-1a over a word sequence — used to fold each block's published viz
/// state (grain slots + peak bins) into two golden words. A single bit
/// anywhere in the snapshot changes the digest.
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, word: u64) {
        for b in word.to_le_bytes() {
            self.0 ^= u64::from(b);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(self) -> [u32; 2] {
        [self.0 as u32, (self.0 >> 32) as u32]
    }
}

/// Input material. Each variant is a deterministic, closed-form signal
/// so the golden never depends on a random source.
#[derive(Clone, Copy)]
enum Signal {
    /// Two detuned partials plus a slow tremolo — broadband-ish, unvoiced
    /// enough that the pitch tracker stays out of the way.
    Mixed,
    /// Band-limited sawtooth at 110 Hz: strongly voiced, so the
    /// pitch-synchronous scheduler locks.
    Saw110,
    /// Deterministic pseudo-noise (LCG): the tracker never locks, so the
    /// PSOLA path stays in its fallback.
    Noise,
}

impl Signal {
    fn sample(self, n: u64) -> (f32, f32) {
        let t = n as f32 / SR;
        match self {
            Signal::Mixed => {
                let trem = 0.75 + 0.25 * (3.0 * t * std::f32::consts::TAU).sin();
                let l = 0.4
                    * trem
                    * ((220.0 * t * std::f32::consts::TAU).sin()
                        + 0.3 * (587.0 * t * std::f32::consts::TAU).sin());
                let r = 0.4
                    * trem
                    * ((221.5 * t * std::f32::consts::TAU).sin()
                        + 0.3 * (593.0 * t * std::f32::consts::TAU).sin());
                (l, r)
            }
            Signal::Saw110 => {
                let mut v = 0.0f32;
                for h in 1..=12 {
                    let f = 110.0 * h as f32;
                    v += (1.0 / h as f32) * (f * t * std::f32::consts::TAU).sin();
                }
                let v = 0.3 * v;
                (v, v)
            }
            Signal::Noise => {
                // 32-bit LCG driven by the absolute sample index, so the
                // stream is identical however the blocks are chopped.
                let mut s = n.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1) as u32;
                s ^= s >> 16;
                let l = (s >> 8) as f32 * (1.0 / (1 << 23) as f32) - 1.0;
                let s2 = s.wrapping_mul(2_654_435_761);
                let r = (s2 >> 8) as f32 * (1.0 / (1 << 23) as f32) - 1.0;
                (0.35 * l, 0.35 * r)
            }
        }
    }
}

/// A parameter edit applied *between* blocks, so the next block picks it
/// up. `None` for scenarios with static parameters.
type MidRunEdit = fn(&GranularDelayParams, usize);

struct Scenario {
    name: &'static str,
    signal: Signal,
    /// Host block sizes, cycled across the run (varying block size is
    /// what shrinks the feedback prefix and re-slices the render loop).
    blocks: &'static [usize],
    tempo: Option<TempoInfo>,
    setup: fn(&GranularDelayParams),
    edit: Option<MidRunEdit>,
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

/// Wet-forward baseline every scenario starts from: free-running time,
/// audible wet, deterministic jitter-free cloud unless overridden.
fn base(p: &GranularDelayParams) {
    p.sync.set_value(false);
    p.time_ms.set_value(55.0);
    p.mix.set_value(0.8);
    p.feedback.set_value(0.0);
    p.density_hz.set_value(24.0);
    p.grain_size_ms.set_value(45.0);
    p.texture.set_value(0.5);
    p.spray_ms.set_value(0.0);
    p.size_jitter.set_value(0.0);
    p.level_jitter.set_value(0.0);
    p.reverse_prob.set_value(0.0);
    p.pan_spread.set_value(0.0);
    p.width.set_value(1.0);
    p.pitch.set_value(0.0);
    p.spread_cents.set_value(0.0);
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // 1. Factory defaults with only the delay shortened (500 ms of
        //    priming would not fit the run): the plainest path —
        //    Per-Grain time, Wet→Buffer feedback, Normal quality, no
        //    stereo work, factory density/size/mix/feedback.
        Scenario {
            name: "factory_defaults",
            signal: Signal::Mixed,
            blocks: &[64],
            tempo: None,
            setup: |p| p.time_ms.set_value(50.0),
            edit: None,
        },
        // 2. Wet→Buffer feedback with damping, jitters and reversal: the
        //    conditioning chain (filter → tanh → DC block) and every
        //    per-grain randomisation draw.
        Scenario {
            name: "wet_to_buffer_jitter",
            signal: Signal::Mixed,
            blocks: &[64, 37, 96],
            tempo: None,
            setup: |p| {
                base(p);
                p.fb_route.set_value(0);
                p.feedback.set_value(0.72);
                p.filter_type.set_value(0);
                p.filter_hz.set_value(3200.0);
                p.spray_ms.set_value(35.0);
                p.size_jitter.set_value(0.6);
                p.level_jitter.set_value(0.4);
                p.reverse_prob.set_value(0.35);
                p.texture.set_value(0.8);
                p.density_hz.set_value(40.0);
            },
            edit: None,
        },
        // 3. Output-only "clean repeats" with a highpass damper and a
        //    swept feedback amount: the dedicated recirculation ring.
        Scenario {
            name: "output_only_hp",
            signal: Signal::Mixed,
            blocks: &[48, 96],
            tempo: None,
            setup: |p| {
                base(p);
                p.fb_route.set_value(1);
                p.feedback.set_value(0.6);
                p.filter_type.set_value(1);
                p.filter_hz.set_value(900.0);
                p.time_ms.set_value(45.0);
            },
            edit: Some(|p, block| {
                let t = block as f32 / BLOCKS as f32;
                p.feedback.set_value(0.3 + 0.7 * t);
                p.filter_hz.set_value(300.0 + 4000.0 * t);
                p.mix.set_value(0.4 + 0.5 * t);
            }),
        },
        // 4. Ping-pong route with the decorrelated right engine engaged
        //    and M/S width swept: the stereo stage end to end.
        Scenario {
            name: "pingpong_stereo",
            signal: Signal::Mixed,
            blocks: &[64, 32],
            tempo: None,
            setup: |p| {
                base(p);
                p.fb_route.set_value(2);
                p.feedback.set_value(0.65);
                p.pan_spread.set_value(0.9);
                p.width.set_value(1.4);
                p.density_hz.set_value(35.0);
            },
            edit: Some(|p, block| {
                // Pan Spread crosses back to 0 mid-run, so the decor
                // engine drains and the blend settles.
                p.pan_spread
                    .set_value(if (BLOCKS / 3..2 * BLOCKS / 3).contains(&block) {
                        0.0
                    } else {
                        0.9
                    });
                p.width
                    .set_value(0.2 + 1.2 * (block as f32 / BLOCKS as f32));
            }),
        },
        // 5. Fade time mode with Time stepped between blocks: the
        //    SwapFader legs, the mid-block swap sample and the engine
        //    hard-reset that lands on it.
        Scenario {
            name: "fade_time_steps",
            signal: Signal::Mixed,
            blocks: &[64, 55],
            tempo: None,
            setup: |p| {
                base(p);
                p.time_mode.set_value(0);
                p.feedback.set_value(0.4);
                p.time_ms.set_value(50.0);
            },
            edit: Some(|p, block| {
                if block % 9 == 4 {
                    p.time_ms
                        .set_value(35.0 + 12.0 * ((block / 9) % 5) as f32);
                }
            }),
        },
        // 6. Repitch time mode swept continuously: the one-pole slew, the
        //    per-slice rate offset and the gliding Output-only read tap.
        Scenario {
            name: "repitch_glide",
            signal: Signal::Mixed,
            blocks: &[64],
            tempo: None,
            setup: |p| {
                base(p);
                p.time_mode.set_value(1);
                p.fb_route.set_value(1);
                p.feedback.set_value(0.5);
                p.time_ms.set_value(60.0);
            },
            edit: Some(|p, block| {
                let t = block as f32 / BLOCKS as f32;
                p.time_ms.set_value(60.0 - 35.0 * t);
            }),
        },
        // 7. Freeze engaged and released mid-run: the write-gain ramp,
        //    the stalled head, the held peak bins and the recirc clock
        //    that keeps ticking underneath.
        Scenario {
            name: "freeze_cycle",
            signal: Signal::Mixed,
            blocks: &[64, 96, 41],
            tempo: None,
            setup: |p| {
                base(p);
                p.feedback.set_value(0.55);
                p.fb_route.set_value(0);
            },
            edit: Some(|p, block| {
                p.freeze
                    .set_value((BLOCKS / 4..3 * BLOCKS / 4).contains(&block));
            }),
        },
        // 8. Shimmer: transposed cloud, FB Pitch off, so the separate
        //    un-transposed feedback-tap engine pair renders — plus the
        //    ghost generations in the published viz.
        Scenario {
            name: "shimmer_unity_tap",
            signal: Signal::Mixed,
            blocks: &[64, 96],
            tempo: None,
            setup: |p| {
                base(p);
                p.pitch.set_value(12.0);
                p.spread_cents.set_value(25.0);
                p.fb_pitch.set_value(false);
                p.fb_route.set_value(0);
                p.feedback.set_value(0.7);
            },
            edit: Some(|p, block| {
                // FB Pitch toggles, so the tap engine engages, drains and
                // re-engages within one run.
                p.fb_pitch.set_value(block % 16 >= 8);
                p.pitch.set_value(if block % 16 >= 8 { 7.0 } else { 12.0 });
            }),
        },
        // 9. Pitch quantization to a scale: the plugin-side draw, the
        //    alternating spread sign and the short render slices.
        Scenario {
            name: "quantized_scale",
            signal: Signal::Mixed,
            blocks: &[64, 96],
            tempo: None,
            setup: |p| {
                base(p);
                p.pitch.set_value(5.0);
                p.spread_cents.set_value(80.0);
                p.pitch_quantize.set_value(2);
                p.root.set_value(3);
                p.scale.set_value(1);
                p.density_hz.set_value(45.0);
                p.feedback.set_value(0.3);
            },
            edit: Some(|p, block| {
                p.pitch_quantize.set_value(match block % 12 {
                    0..=3 => 0,
                    4..=7 => 1,
                    _ => 2,
                });
            }),
        },
        // 10. Pitch-sync scheduler on voiced material: the tracker feed,
        //     marker-snapped PSOLA voices and the voice crossfade in both
        //     directions (the signal goes unvoiced for a stretch).
        Scenario {
            name: "pitch_sync_voiced",
            signal: Signal::Saw110,
            blocks: &[64, 96],
            tempo: None,
            setup: |p| {
                base(p);
                p.scheduler.set_value(2);
                p.pitch.set_value(3.0);
                p.time_ms.set_value(45.0);
                p.feedback.set_value(0.25);
            },
            edit: Some(|p, block| {
                // Toggling the scheduler drains the voice bus and hands
                // back to the async cloud through the crossfade.
                p.scheduler.set_value(if block % 20 >= 14 { 1 } else { 2 });
            }),
        },
        // 11. Pitch-sync on noise: the tracker never locks, so the whole
        //     block runs the unvoiced fallback.
        Scenario {
            name: "pitch_sync_noise",
            signal: Signal::Noise,
            blocks: &[64],
            tempo: None,
            setup: |p| {
                base(p);
                p.scheduler.set_value(2);
                p.feedback.set_value(0.4);
                p.fb_route.set_value(2);
            },
            edit: None,
        },
        // 12. Quality tiers cycled: linear/µ-law/reduced pool, Hermite,
        //     and the B-spline + anti-alias tier, all with a transpose so
        //     the resampling kernels actually differ.
        Scenario {
            name: "quality_tiers",
            signal: Signal::Mixed,
            blocks: &[64, 96],
            tempo: None,
            setup: |p| {
                base(p);
                p.pitch.set_value(7.0);
                p.density_hz.set_value(50.0);
                p.feedback.set_value(0.35);
            },
            edit: Some(|p, block| {
                p.quality.set_value((block / 6 % 3) as i32);
            }),
        },
        // 13. Tempo-synced delay with the division stepped: the sync
        //     resolution path in `lib.rs` feeding the Fade machine.
        Scenario {
            name: "tempo_sync_divisions",
            signal: Signal::Mixed,
            blocks: &[64, 32],
            tempo: Some(tempo(320.0)),
            setup: |p| {
                base(p);
                p.sync.set_value(true);
                p.time_mode.set_value(0);
                p.feedback.set_value(0.5);
            },
            edit: Some(|p, block| {
                // At 320 BPM: 1/8T = 62 ms, 1/16 = 47 ms, 1/16T = 31 ms —
                // all short enough to be primed inside the run.
                p.division.set_value(9 + (block / 7 % 3) as i32);
            }),
        },
        // 14. Tempo-locked grain rate with the density division stepped
        //     (ba todo #1322): the sync resolution feeding the grain
        //     scheduler, and the re-lock when the division changes. At
        //     320 BPM, 1/16 = 16 grains/s and 1/16T = 32 grains/s.
        Scenario {
            name: "density_sync_locked",
            signal: Signal::Mixed,
            blocks: &[64, 96],
            tempo: Some(tempo(320.0)),
            setup: |p| {
                base(p);
                p.density_sync.set_value(true);
                p.density_division.set_value(11);
                p.grain_size_ms.set_value(45.0);
                p.feedback.set_value(0.3);
            },
            edit: Some(|p, block| {
                p.density_division
                    .set_value(if block % 16 >= 8 { 10 } else { 11 });
            }),
        },
    ]
}

/// Deterministic render of one scenario into the golden word stream:
/// per frame the two output samples' bit patterns, and per block a
/// digest of the published viz state (grain-slot count, packed slots,
/// peak bins and their time base).
fn render_scenario(s: &Scenario) -> Vec<u32> {
    let mut plugin = ResonanceGranularDelay::new();
    (s.setup)(&plugin.params);
    plugin.initialize(SR, MAX_BLOCK as u32);

    let mut out = Vec::new();
    let mut left = vec![0.0f32; MAX_BLOCK];
    let mut right = vec![0.0f32; MAX_BLOCK];
    let mut grains = [GrainSnapshot::default(); GRAIN_SLOTS];
    let mut peaks = [0.0f32; PEAK_BINS];
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
            plugin.process(&mut outs, frames, &mut ev, s.tempo);
        }
        n += frames as u64;

        if block >= PRIME_BLOCKS {
            for i in 0..frames {
                out.push(left[i].to_bits());
                out.push(right[i].to_bits());
            }
        }

        let mut digest = Fnv::new();
        let count = plugin.viz().read_grains(&mut grains);
        digest.write(count as u64);
        for g in &grains[..count] {
            digest.write(pack_grain(g));
        }
        let bin_ms = plugin.viz().read_peaks(&mut peaks);
        digest.write(u64::from(bin_ms.to_bits()));
        for p in &peaks {
            digest.write(u64::from(p.to_bits()));
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
fn dsp_output_and_viz_are_bit_exact() {
    let rendered = render_all();
    let path = golden_path();

    if std::env::var("RESONANCE_BLESS_GRANULAR_DSP").as_deref() == Ok("1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bytes: Vec<u8> = rendered.iter().flat_map(|w| w.to_le_bytes()).collect();
        std::fs::write(&path, bytes).unwrap();
        eprintln!(
            "blessed golden: {} words -> {}",
            rendered.len(),
            path.display()
        );
        return;
    }

    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "missing golden {}: {e}\nregenerate with RESONANCE_BLESS_GRANULAR_DSP=1",
            path.display()
        )
    });
    assert_eq!(
        bytes.len(),
        rendered.len() * 4,
        "golden length mismatch — the scenario set changed"
    );

    let golden = bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()));

    let mut diff_count = 0usize;
    let mut first_diff = None;
    for (i, (a, b)) in rendered.iter().copied().zip(golden).enumerate() {
        if a != b {
            diff_count += 1;
            if first_diff.is_none() {
                first_diff = Some((i, a, b));
            }
        }
    }

    if let Some((i, got, want)) = first_diff {
        panic!(
            "granular DSP output changed: {diff_count}/{} words differ; first at word {i} \
             (got {got:#010x}, want {want:#010x}; as f32: {} vs {}). A refactor of the DSP \
             path must be bit-exact — if the change was intended, re-bless with \
             RESONANCE_BLESS_GRANULAR_DSP=1.",
            rendered.len(),
            f32::from_bits(got),
            f32::from_bits(want),
        );
    }
}

/// Guards the scenario table itself: over the captured half of every
/// scenario the *wet* path must be audible — the output has to depart
/// from the dry input it was handed — and grains must be published. A
/// scenario whose delay outran the run would otherwise render pure dry
/// and pin nothing but the dry/wet mix.
#[test]
fn every_scenario_renders_wet_audio_and_grains() {
    for s in scenarios() {
        let mut plugin = ResonanceGranularDelay::new();
        (s.setup)(&plugin.params);
        plugin.initialize(SR, MAX_BLOCK as u32);

        let mut left = vec![0.0f32; MAX_BLOCK];
        let mut right = vec![0.0f32; MAX_BLOCK];
        let mut dry_l = vec![0.0f32; MAX_BLOCK];
        let mut dry_r = vec![0.0f32; MAX_BLOCK];
        let mut grains = [GrainSnapshot::default(); GRAIN_SLOTS];
        let mut wet_departure = 0.0f32;
        let mut max_grains = 0usize;
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
                dry_l[i] = l;
                dry_r[i] = r;
            }
            {
                let mut outs = [OutputBuffer {
                    left: &mut left[..frames],
                    right: &mut right[..frames],
                }];
                let mut ev = EventIterator::empty();
                plugin.process(&mut outs, frames, &mut ev, s.tempo);
            }
            n += frames as u64;
            for i in 0..frames {
                assert!(
                    left[i].is_finite() && right[i].is_finite(),
                    "scenario `{}` rendered a non-finite sample",
                    s.name
                );
            }
            if block >= PRIME_BLOCKS {
                // The dry path is an equal-power scaling of the input, so
                // any departure beyond that scaling is wet content.
                for i in 0..frames {
                    let mix = plugin.params.mix.value();
                    let dry_gain = (1.0 - mix).sqrt();
                    wet_departure = wet_departure
                        .max((left[i] - dry_l[i] * dry_gain).abs())
                        .max((right[i] - dry_r[i] * dry_gain).abs());
                }
            }
            max_grains = max_grains.max(plugin.viz().read_grains(&mut grains));
        }

        assert!(
            wet_departure > 1e-3,
            "scenario `{}` rendered no wet content in the captured window \
             (delay longer than the run?)",
            s.name
        );
        assert!(
            max_grains > 0,
            "scenario `{}` published no grain snapshots",
            s.name
        );
    }
}
