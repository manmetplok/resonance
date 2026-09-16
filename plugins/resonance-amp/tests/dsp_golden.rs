//! Bit-exact DSP golden for the amp's audio-thread processor
//! (ba todo #1373).
//!
//! The `nam_*_reference_parity` suites already pin the *inference* maths
//! against the upstream C++ implementation, to a documented tolerance.
//! What nothing pinned before this is everything `AmpProcessor` does
//! around it: the L+R mono sum that feeds the model, the two logarithmic
//! gain smoothers, the DC blockers on the output, the 1024-sample
//! crossfade when a freshly-loaded model takes over mid-block, and the
//! peak metering. All of that can change without a parity test moving,
//! and all of it is audible.
//!
//! # The model, and why it is not a model-free golden
//!
//! An amp with no model loaded is a gain stage, so a golden over the
//! model-free path alone would prove almost nothing. This test loads
//! real `.nam` files — the small WaveNet, the LSTM and the A2 WaveNet
//! already committed under `tests/fixtures/`, which are the upstream
//! NeuralAmpModelerCore example models (MIT, see the fixture READMEs
//! for provenance). They are 2–12 kB each, they are in the repository
//! already, and they exercise all three inference back-ends.
//!
//! One scenario *does* cover the model-free path, because it is a real
//! state a user hits (plugin instantiated, nothing loaded yet) and
//! because its output — input gain × output gain with no DC blocking
//! and no fade — is a different branch of `process_block`.
//!
//! ## What this does not cover
//!
//! The plugin's own `process` in `lib.rs`: the file-select parameter,
//! the loader thread, the mailbox that hands a freshly-parsed model to
//! the processor, and the Tone3000 browser. Those need a model
//! directory on disk and a background thread; the *consequence* of that
//! handoff — `install_pending_model` and the crossfade it starts — is
//! covered here.
//!
//! # The signal, and why it is the revealing one
//!
//! A neural amp model is a **nonlinear, level-dependent** system: the
//! whole point of it is that a quiet note and a loud note come out
//! differently shaped, not just differently scaled. So the driver is a
//! guitar-DI-shaped pluck train whose level **swells from −34 dBFS to
//! 0 dBFS across the run**, sweeping the model through its clean range,
//! its breakup and its saturation in one render. A fixed-level signal
//! would pin one operating point of a curve. A second signal — a
//! sustained power chord — covers what a decaying pluck cannot: what
//! the model does with continuous energy, where the DC blocker's
//! high-pass and any accumulating recurrent state actually matter (the
//! LSTM is recurrent; the WaveNets are not).
//!
//! # Tolerance: bit-exact
//!
//! Inference is dense f32 matrix-vector arithmetic with tanh/sigmoid
//! activations; around it are two one-pole smoothers, two DC blockers
//! and a linear crossfade. No FFT, no runtime SIMD dispatch, no RNG, no
//! clock. Every render is reproducible, so anything short of bit
//! equality would let a real change hide. (Bits can move between
//! machines only through libm's rounding inside `tanh`/`exp`; that
//! shows up as most of the render shifting by ~1e-7, never as a handful
//! of samples differing visibly. The parity suites' tolerances exist
//! for a different reason — they compare against an f64 C++
//! implementation.)
//!
//! To (re)generate the golden after an *intentional* audio change:
//!
//!     RESONANCE_BLESS=1 cargo test -p resonance-amp --test dsp_golden
//!
//! A pure refactor must never need this.

use std::path::PathBuf;

use resonance_amp::dsp::AmpProcessor;
use resonance_amp::nam::parse::{load_model_from_file, LoadedModel};
use resonance_dsp_test_support as golden;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
/// Blocks per scenario — 16 × 256 = 4096 samples, ~85 ms. The models
/// are sample-serial and cheap, but they are also run in a debug build
/// here; this is enough to cover the level swell end to end and the
/// whole model-swap crossfade.
const BLOCKS: usize = 16;

fn golden_path() -> PathBuf {
    golden::golden_path(env!("CARGO_MANIFEST_DIR"), "dsp_golden.u32")
}

/// `RESONANCE_BLESS=1` is the workspace-wide convention (CLAUDE.md); the
/// narrower name blesses only this file inside a wider run.
fn blessing() -> bool {
    golden::blessed(&["RESONANCE_BLESS", "RESONANCE_BLESS_DSP_GOLDEN"])
}

const TAU: f32 = std::f32::consts::TAU;

/// The already-committed example models, by fixture directory and file.
#[derive(Clone, Copy, PartialEq)]
enum Model {
    /// `a1/wavenet.nam` — small A1 WaveNet, two tiny layer arrays.
    A1Wavenet,
    /// `lstm/lstm.nam` — the recurrent back-end, whose state carries
    /// across the whole render.
    Lstm,
    /// `a2/wavenet_condition_dsp.nam` — an A2 WaveNet, the newer
    /// architecture with its own activation set and conditioning.
    A2Wavenet,
}

impl Model {
    fn path(self) -> String {
        let (dir, file) = match self {
            Model::A1Wavenet => ("a1", "wavenet.nam"),
            Model::Lstm => ("lstm", "lstm.nam"),
            Model::A2Wavenet => ("a2", "wavenet_condition_dsp.nam"),
        };
        format!("{}/tests/fixtures/{dir}/{file}", env!("CARGO_MANIFEST_DIR"))
    }

    fn load(self) -> Box<dyn resonance_amp::nam::NamInference> {
        let path = self.path();
        let LoadedModel { model, .. } = load_model_from_file(&path)
            .unwrap_or_else(|e| panic!("fixture model {path} must load: {e}"));
        model
    }
}

#[derive(Clone, Copy)]
enum Signal {
    /// 12 Hz pluck train swelling from −34 dBFS to 0 dBFS across the
    /// render: the model is swept through clean, breakup and
    /// saturation in one pass.
    SwellingPlucks,
    /// A sustained power chord at −6 dBFS. Continuous energy, so the
    /// DC blocker's high-pass and the LSTM's recurrent state have
    /// something that does not decay away.
    PowerChord,
}

impl Signal {
    fn sample(self, n: u64, total: u64) -> (f32, f32) {
        let t = n as f32 / SR;
        match self {
            Signal::SwellingPlucks => {
                let progress = n as f32 / total.max(1) as f32;
                // −34 dBFS up to 0 dBFS, linear in dB.
                let level = 10.0f32.powf((-34.0 + 34.0 * progress) / 20.0);
                let x = t % (1.0 / 12.0);
                let env = (-18.0 * x).exp();
                let v = level
                    * env
                    * ((110.0 * t * TAU).sin()
                        + 0.6 * (220.0 * t * TAU).sin()
                        + 0.3 * (330.0 * t * TAU).sin()
                        + 0.15 * (880.0 * t * TAU).sin());
                // The right channel is 3 dB down and detuned. The model
                // is mono-by-design and the processor sums L+R before
                // driving it, so an asymmetric pair is what makes a
                // dropped channel or a changed sum weight visible.
                let vr = level
                    * env
                    * 0.7
                    * ((110.6 * t * TAU).sin() + 0.6 * (221.2 * t * TAU).sin());
                (v, vr)
            }
            Signal::PowerChord => {
                let v = 0.5
                    * ((82.4 * t * TAU).sin()
                        + 0.8 * (123.5 * t * TAU).sin()
                        + 0.5 * (164.8 * t * TAU).sin());
                let vr = 0.5
                    * ((82.4 * t * TAU).sin() + 0.8 * (123.9 * t * TAU).sin())
                    * 0.7;
                (v, vr)
            }
        }
    }
}

/// A gain edit applied *between* blocks, mirroring `lib.rs`, which
/// calls `set_gain_targets` from the current parameter values once per
/// block. Returns `(input_gain, output_gain)`.
type MidRunGains = fn(usize) -> (f32, f32);

struct Scenario {
    name: &'static str,
    signal: Signal,
    /// The model installed before the run; `None` renders the
    /// model-free branch.
    model: Option<Model>,
    /// If set, `install_pending_model` at the given block, starting the
    /// crossfade.
    swap_to: Option<(Model, usize)>,
    input_gain: f32,
    output_gain: f32,
    gains: Option<MidRunGains>,
}

fn scenarios() -> Vec<Scenario> {
    vec![
        // 1. No model loaded. Not silence and not a bypass: the
        //    processor applies input × output gain, skips the DC
        //    blockers entirely and leaves the fader idle. It is the
        //    state every instance starts in, and it is its own branch.
        Scenario {
            name: "no_model_gain_only",
            signal: Signal::SwellingPlucks,
            model: None,
            swap_to: None,
            input_gain: 2.0,
            output_gain: 0.6,
            gains: None,
        },
        // 2. The small A1 WaveNet on the swelling plucks: the
        //    everyday path, swept across the model's whole
        //    clean-to-saturated range in one render.
        Scenario {
            name: "a1_wavenet_swell",
            signal: Signal::SwellingPlucks,
            model: Some(Model::A1Wavenet),
            swap_to: None,
            input_gain: 1.0,
            output_gain: 1.0,
            gains: None,
        },
        // 3. The LSTM back-end. It is recurrent, so its state carries
        //    the entire render — which makes it the scenario where a
        //    reordering of the per-sample loop, or a missed reset,
        //    diverges fastest.
        Scenario {
            name: "lstm_swell",
            signal: Signal::SwellingPlucks,
            model: Some(Model::Lstm),
            swap_to: None,
            input_gain: 1.0,
            output_gain: 1.0,
            gains: None,
        },
        // 4. An A2 WaveNet, whose activations and conditioning differ
        //    from A1's. Both architectures in the golden means an
        //    architecture-specific regression cannot hide behind the
        //    other one passing.
        Scenario {
            name: "a2_wavenet_swell",
            signal: Signal::SwellingPlucks,
            model: Some(Model::A2Wavenet),
            swap_to: None,
            input_gain: 1.0,
            output_gain: 1.0,
            gains: None,
        },
        // 5. Sustained input with both gain smoothers sweeping across
        //    their range. The smoothers are logarithmic with a 50 ms
        //    time constant, so they are always mid-ramp at a block
        //    boundary; the DC blockers are meanwhile working on a
        //    signal that never lets up.
        Scenario {
            name: "a1_gain_ramps_on_sustain",
            signal: Signal::PowerChord,
            model: Some(Model::A1Wavenet),
            swap_to: None,
            input_gain: 0.2,
            output_gain: 0.2,
            gains: Some(|block| {
                let t = block as f32 / BLOCKS as f32;
                // Input gain drives the model harder as it rises, so
                // the ramp is a timbre change and not only a level
                // change — which is exactly why it has to be smoothed.
                (0.2 + 5.0 * t, 1.4 - 1.0 * t)
            }),
        },
        // 6. A model swap mid-render: the old model fades out over
        //    1024 samples, the new one takes over and fades in. The two
        //    models are different architectures, so a broken handover
        //    is a step in the waveform rather than a subtle level
        //    change. This is the path the loader thread's mailbox
        //    triggers in the real plugin.
        Scenario {
            name: "model_swap_crossfade",
            signal: Signal::PowerChord,
            model: Some(Model::A1Wavenet),
            swap_to: Some((Model::Lstm, BLOCKS / 3)),
            input_gain: 1.5,
            output_gain: 0.9,
            gains: None,
        },
    ]
}

/// Deterministic render of one scenario into the golden word stream:
/// per frame the two output samples' bit patterns, and per block the
/// four reported peak values, so the metering is pinned alongside the
/// audio.
fn render_scenario(s: &Scenario) -> Vec<u32> {
    let mut proc = AmpProcessor::new();
    proc.initialize(SR, s.input_gain, s.output_gain);
    if let Some(m) = s.model {
        proc.install_initial_model(m.load());
    }

    let total = (BLOCKS * BLOCK) as u64;
    let mut out = Vec::new();
    let mut left = vec![0.0f32; BLOCK];
    let mut right = vec![0.0f32; BLOCK];
    let mut n: u64 = 0;

    for block in 0..BLOCKS {
        if let Some((m, at)) = s.swap_to {
            if block == at {
                proc.install_pending_model(m.load());
            }
        }
        let (ig, og) = match s.gains {
            Some(f) => f(block),
            None => (s.input_gain, s.output_gain),
        };
        proc.set_gain_targets(ig, og);

        for i in 0..BLOCK {
            let (l, r) = s.signal.sample(n + i as u64, total);
            left[i] = l;
            right[i] = r;
        }
        let peaks = proc.process_block(&mut left, &mut right, BLOCK);
        n += BLOCK as u64;

        for i in 0..BLOCK {
            assert!(
                left[i].is_finite() && right[i].is_finite(),
                "scenario `{}` rendered a non-finite sample in block {block}",
                s.name
            );
            out.push(left[i].to_bits());
            out.push(right[i].to_bits());
        }
        // The metering the editor reads. Pinned here because it is
        // computed inside the same loop and a refactor that moves the
        // peak capture across the gain stage would otherwise be
        // invisible.
        out.push(peaks.in_l.to_bits());
        out.push(peaks.in_r.to_bits());
        out.push(peaks.out_l.to_bits());
        out.push(peaks.out_r.to_bits());
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
fn amp_output_is_bit_exact() {
    // Finiteness is asserted inside `render_scenario`, on the samples
    // themselves.
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
            "amp DSP output changed: {}/{} words differ; first at word \
             {i} (got {got:#010x}, want {want:#010x}; as f32: {} vs {}).\nA \
             refactor of the amp's audio path must be bit-exact. If the change \
             was intended, re-bless with RESONANCE_BLESS=1.\nA whole-render \
             shift at the ~1e-7 level is libm's tanh/exp rounding, not a DSP \
             change.",
            diff.diff_count,
            rendered.len(),
            f32::from_bits(got),
            f32::from_bits(want),
        );
    }
}

/// Guards the scenario table: every model must load, and every
/// model-driven scenario must actually be *nonlinear* — a model that
/// silently failed to install would leave `process_block` on its
/// gain-only branch, which is a perfectly finite, perfectly plausible
/// render that pins nothing about the amp.
#[test]
fn every_model_loads_and_shapes_the_signal() {
    for m in [Model::A1Wavenet, Model::Lstm, Model::A2Wavenet] {
        let _ = m.load();
    }

    for s in scenarios() {
        let out = render_scenario(&s);
        // Only the audio words, skipping the four peak words per block.
        let audio: Vec<f32> = out
            .chunks(BLOCK * 2 + 4)
            .flat_map(|c| c[..BLOCK * 2].iter().map(|w| f32::from_bits(*w)))
            .collect();
        let peak = audio.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!(peak > 1e-4, "scenario `{}` rendered silence", s.name);

        let Some(_) = s.model else {
            continue;
        };
        // Rebuild the same input and check the output is not a scalar
        // multiple of it. A neural amp is a waveshaper; a linear
        // best-fit must leave a substantial residual.
        let total = (BLOCKS * BLOCK) as u64;
        let mut dry = Vec::with_capacity(audio.len());
        for block in 0..BLOCKS {
            for i in 0..BLOCK {
                let n = (block * BLOCK + i) as u64;
                let (l, r) = s.signal.sample(n, total);
                dry.push(l);
                dry.push(r);
            }
        }
        // The processor sums L+R into a mono model, so the two output
        // channels carry the same shaped signal; comparing against the
        // interleaved input is still the right linearity probe.
        let dot: f64 = audio
            .iter()
            .zip(&dry)
            .map(|(o, d)| (*o as f64) * (*d as f64))
            .sum();
        let den: f64 = dry.iter().map(|d| (*d as f64) * (*d as f64)).sum();
        let k = dot / den.max(1e-30);
        let resid: f64 = audio
            .iter()
            .zip(&dry)
            .map(|(o, d)| {
                let e = *o as f64 - k * *d as f64;
                e * e
            })
            .sum();
        let energy: f64 = audio.iter().map(|o| (*o as f64) * (*o as f64)).sum();
        assert!(
            resid > 0.1 * energy,
            "scenario `{}` output is within 10% of a scaled copy of its input — \
             the model is not shaping anything, so the golden would pin a gain \
             stage",
            s.name
        );
    }
}
