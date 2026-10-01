//! Release and choke fades are timed in milliseconds, follow an
//! equal-power curve, and last the same time at every sample rate
//! (drums-plugin-rework.md §7 E2).
//!
//! The fade used to be a linear ramp of exactly 1024 samples: 23 ms at
//! 44.1 kHz, 21 ms at 48 kHz and 11 ms at 96 kHz, so a hi-hat choke
//! sounded different depending on the project rate.
//!
//! Every pad here plays flat DC at 1.0 with unity gain throughout, so
//! the rendered mix *is* the fading voice's envelope.

use std::f32::consts::FRAC_PI_4;

use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, Hit, PortBuffers};
use resonance_drums::kit::{
    LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, NUM_OUTPUT_PORTS,
};
use resonance_drums::params::DrumParams;
use resonance_drums::voice::{RELEASE_FADE_MS, SWAP_FADE_MS};

const RATES: [f32; 3] = [44_100.0, 48_000.0, 96_000.0];
const BLOCK: usize = 64;

/// Every pad flat DC at `level`, except `silent_note`'s pad (if any),
/// which plays zeros — so a hit on it fires its choke group and adds
/// nothing to the output. Choke groups as in the real map.
fn dc_pads(level: f32, silent_note: Option<u8>) -> Vec<LoadedPad> {
    let silent = silent_note.and_then(drum_map::pad_index_for_note);
    let frames = 48_000;
    PAD_MAPPINGS
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let v = if Some(i) == silent { 0.0 } else { level };
            LoadedPad {
                name: m.name.to_string(),
                choke_group: m.choke_group,
                output_group: m.output_group,
                close_mics: vec![LoadedMicBank {
                    position: "test".to_string(),
                    setup_key: String::new(),
                    layers: vec![VelocityLayer {
                        round_robins: vec![LoadedSample::from_data(vec![v; frames * 2])],
                    }],
                }],
                overhead: None,
            }
        })
        .collect()
}

struct Rig {
    sampler: DrumSampler,
    params: DrumParams,
    kit_tx: crossbeam_channel::Sender<Vec<LoadedPad>>,
}

impl Rig {
    fn new(rate: f32, silent_note: Option<u8>) -> Self {
        let (kit_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
        let mut sampler = DrumSampler::new(rx);
        sampler.set_sample_rate(rate);
        sampler.pads = dc_pads(1.0, silent_note);
        let params = DrumParams::default();
        params.master_volume.set_value(1.0);
        for pad in &params.pads {
            pad.volume.set_value(1.0);
        }
        sampler.update_global_settings(&params);
        Self {
            sampler,
            params,
            kit_tx,
        }
    }

    /// One block of the kit mix (every port summed, left channel).
    fn block(&mut self, hits: &[Hit]) -> Vec<f32> {
        let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
            .map(|_| (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]))
            .collect();
        {
            let mut ports: Vec<PortBuffers<'_>> = bufs
                .iter_mut()
                .map(|(l, r)| PortBuffers {
                    left: l.as_mut_slice(),
                    right: r.as_mut_slice(),
                })
                .collect();
            self.sampler
                .render_block(&mut ports, BLOCK, &self.params, hits);
        }
        (0..BLOCK)
            .map(|i| bufs.iter().map(|(l, _)| l[i]).sum())
            .collect()
    }

    /// Render until silent, starting with `first` (the block that holds
    /// the event), and return the envelope from the event on.
    fn fade_from(&mut self, first: Vec<f32>) -> Vec<f32> {
        let mut env = first;
        for _ in 0..200 {
            if env.last().is_some_and(|s| *s == 0.0) {
                break;
            }
            env.extend(self.block(&[]));
        }
        env
    }
}

fn hit(note: u8) -> Hit {
    Hit {
        frame: 0,
        note,
        velocity: 1.0,
    }
}

/// Measure a fade that starts at frame 0 of `env` from unity: its length
/// in frames (the first frame at which the voice is gone) and its gain
/// halfway through.
fn measure(env: &[f32]) -> (usize, f32) {
    assert!(
        (env[0] - 1.0).abs() < 1e-6,
        "the fade should start at unity, got {}",
        env[0]
    );
    let len = env
        .iter()
        .position(|s| *s == 0.0)
        .expect("the fade never reached silence");
    for w in env[..len].windows(2) {
        assert!(
            w[1] <= w[0],
            "a fade-out must never rise: {} -> {}",
            w[0],
            w[1]
        );
    }
    (len, env[len / 2])
}

/// The same fade, in milliseconds and in shape, at every rate.
fn assert_fade_ms(what: &str, want_ms: f32, mut fade_at: impl FnMut(f32) -> Vec<f32>) {
    for rate in RATES {
        let (len, mid) = measure(&fade_at(rate));
        let ms = len as f32 * 1000.0 / rate;
        let frame_ms = 1000.0 / rate;
        assert!(
            (ms - want_ms).abs() <= frame_ms,
            "{what} at {rate} Hz lasted {ms:.3} ms ({len} frames), want {want_ms} ms"
        );
        // Equal power: cos(π/4) ≈ 0.707 halfway, where a linear ramp is 0.5.
        assert!(
            (mid - FRAC_PI_4.cos()).abs() < 0.01,
            "{what} at {rate} Hz is not equal-power: {mid} halfway through"
        );
    }
}

#[test]
fn hihat_choke_fade_is_the_same_in_ms_at_every_rate() {
    assert_fade_ms("hi-hat choke", RELEASE_FADE_MS, |rate| {
        // Closed hat plays silence, so the mix is the open hat alone.
        let mut rig = Rig::new(rate, Some(drum_map::HIHAT_CLOSED));
        rig.block(&[hit(drum_map::HIHAT_OPEN)]);
        let first = rig.block(&[hit(drum_map::HIHAT_CLOSED)]);
        rig.fade_from(first)
    });
}

#[test]
fn host_choke_fade_is_the_same_in_ms_at_every_rate() {
    assert_fade_ms("host choke", RELEASE_FADE_MS, |rate| {
        let mut rig = Rig::new(rate, None);
        rig.block(&[hit(drum_map::TOM_LOW)]);
        rig.sampler.choke_note(drum_map::TOM_LOW);
        let first = rig.block(&[]);
        rig.fade_from(first)
    });
}

#[test]
fn kit_swap_fade_is_the_same_in_ms_at_every_rate() {
    assert_fade_ms("kit-swap fade", SWAP_FADE_MS, |rate| {
        let mut rig = Rig::new(rate, None);
        rig.block(&[hit(drum_map::TOM_LOW)]);
        rig.kit_tx.send(dc_pads(0.5, None)).unwrap();
        rig.sampler.try_swap_kit();
        let first = rig.block(&[]);
        rig.fade_from(first)
    });
}

/// CLAP `reset` is the one deliberate hard cut (see
/// `DrumSampler::reset`): the host calls it while not processing, so the
/// next render — a bounce — starts from silence.
#[test]
fn reset_is_immediate() {
    let mut rig = Rig::new(48_000.0, None);
    rig.block(&[hit(drum_map::TOM_LOW)]);
    rig.sampler.reset();
    assert!(rig.block(&[]).iter().all(|s| *s == 0.0));
}
