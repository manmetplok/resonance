//! `DrumSampler::render_block` honours hit offsets itself (FU-G1).
//!
//! `process()` got sample-accurate onsets (DSP-01) by splitting the block
//! into `begin_block` / `render_span` / `end_block` around each event, but
//! the one-call `render_block` had no way to take a hit inside the block:
//! whatever `note_on` preceded it sounded from frame 0. It now takes the
//! block's hits with their frame offsets and applies each between spans,
//! so the convenience entry point is not a timing trap for a caller that
//! has events.

use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, Hit, PortBuffers};
use resonance_drums::kit::{LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};
use resonance_drums::params::DrumParams;

const NUM_PORTS: usize = 7;
const BLOCK: usize = 512;

/// A sampler whose every pad plays a 32-frame DC burst on its close mics.
fn sampler() -> DrumSampler {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut s = DrumSampler::new(rx);
    s.pads = PAD_MAPPINGS
        .iter()
        .map(|m| LoadedPad {
            name: m.name.to_string(),
            choke_group: m.choke_group,
            output_group: m.output_group,
            close_mics: m
                .close_mic_positions
                .iter()
                .map(|pos| LoadedMicBank {
                    position: pos.to_string(),
                    setup_key: String::new(),
                    layers: vec![VelocityLayer {
                        round_robins: vec![LoadedSample {
                            data: vec![0.5; 64],
                            frames: 32,
                        }],
                    }],
                })
                .collect(),
            overhead: None,
        })
        .collect();
    s
}

/// Render one block with `hits` and return the mix of every port's left
/// channel (the kick's port is not the point here).
fn render(s: &mut DrumSampler, params: &DrumParams, hits: &[Hit]) -> Vec<f32> {
    let mut data: Vec<(Vec<f32>, Vec<f32>)> =
        (0..NUM_PORTS).map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK])).collect();
    {
        let mut ports: Vec<PortBuffers<'_>> = data
            .iter_mut()
            .map(|(l, r)| PortBuffers {
                left: l.as_mut_slice(),
                right: r.as_mut_slice(),
            })
            .collect();
        s.render_block(&mut ports, BLOCK, params, hits);
    }
    (0..BLOCK).map(|i| data.iter().map(|(l, _)| l[i]).sum()).collect()
}

fn kick(frame: usize) -> Hit {
    Hit {
        frame,
        note: drum_map::KICK,
        velocity: 1.0,
    }
}

fn onsets(x: &[f32]) -> Vec<usize> {
    (0..x.len())
        .filter(|&i| x[i] != 0.0 && (i == 0 || x[i - 1] == 0.0))
        .collect()
}

#[test]
fn render_block_starts_each_hit_at_its_frame() {
    let params = DrumParams::default();
    // Warm-up block: the first block ramps master volume in from unity.
    let mut s = sampler();
    render(&mut s, &params, &[]);
    let at_0 = render(&mut s, &params, &[kick(0)]);

    let mut s = sampler();
    render(&mut s, &params, &[]);
    let at_300 = render(&mut s, &params, &[kick(300)]);

    assert!(at_0.iter().any(|x| *x != 0.0), "hit at 0 rendered silence");
    assert_eq!(onsets(&at_300), vec![300], "hit timed at 300");
    assert_eq!(&at_300[300..332], &at_0[..32], "the hit at 300 is not the hit at 0, shifted");
}

#[test]
fn render_block_plays_two_hits_on_one_pad_as_two_onsets() {
    let params = DrumParams::default();
    let mut s = sampler();
    render(&mut s, &params, &[]);
    let out = render(&mut s, &params, &[kick(0), kick(200)]);
    assert_eq!(onsets(&out), vec![0, 200]);
}

/// Same clamping contract as `process()`: an offset past the block lands
/// on its last frame, and an out-of-order one at the frame already reached.
#[test]
fn render_block_clamps_late_and_unsorted_hits() {
    let params = DrumParams::default();
    let mut s = sampler();
    render(&mut s, &params, &[]);
    let out = render(&mut s, &params, &[kick(100), kick(50), kick(BLOCK + 99)]);
    // 100 and the reordered 50 both start at 100 (one onset, twice as loud);
    // the late one at the last frame.
    assert_eq!(onsets(&out), vec![100, BLOCK - 1]);
}
