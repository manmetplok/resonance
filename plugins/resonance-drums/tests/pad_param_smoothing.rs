//! Per-pad parameter declick: volume / pan / OH blend / balance are
//! snapshotted per block but must be linearly ramped across the block
//! (mirroring the master volume ramp), so a param jump between blocks
//! produces a continuous gain trajectory instead of a step.

use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};
use resonance_drums::params::DrumParams;

const NUM_PORTS: usize = 7;
const FRAMES: usize = 256;

fn make_sampler() -> DrumSampler {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    DrumSampler::new(rx)
}

/// A long constant-1.0 stereo sample, so the rendered output *is* the
/// effective gain trajectory.
fn dc_layer(frames: usize) -> VelocityLayer {
    VelocityLayer {
        round_robins: vec![LoadedSample {
            data: vec![1.0; frames * 2],
            frames,
        }],
    }
}

/// Build a kit where every pad has exactly one close mic playing DC 1.0
/// (BalanceSide::None) and no overhead, so each hit renders one voice.
fn dc_pads(sample_frames: usize) -> Vec<LoadedPad> {
    PAD_MAPPINGS
        .iter()
        .map(|m| LoadedPad {
            name: m.name.to_string(),
            choke_group: None, // avoid choking the pad under test
            output_group: m.output_group,
            close_mics: vec![LoadedMicBank {
                position: "test".to_string(),
                setup_key: String::new(),
                layers: vec![dc_layer(sample_frames)],
            }],
            overhead: None,
        })
        .collect()
}

fn render_block(sampler: &mut DrumSampler, params: &DrumParams) -> Vec<(Vec<f32>, Vec<f32>)> {
    let mut port_data: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_PORTS)
        .map(|_| (vec![0.0; FRAMES], vec![0.0; FRAMES]))
        .collect();
    {
        let mut ports: Vec<PortBuffers<'_>> = port_data
            .iter_mut()
            .map(|(l, r)| PortBuffers {
                left: l.as_mut_slice(),
                right: r.as_mut_slice(),
            })
            .collect();
        sampler.render_block(&mut ports, FRAMES, params);
    }
    port_data
}

/// The pad under test: the low tom (single close mic, BalanceSide::None).
fn tom_setup() -> (DrumSampler, DrumParams, usize, usize) {
    let mut sampler = make_sampler();
    sampler.pads = dc_pads(FRAMES * 8);
    let params = DrumParams::default();
    let pad_index = drum_map::pad_index_for_note(drum_map::TOM_LOW).unwrap();
    let port = PAD_MAPPINGS[pad_index].output_group.index();
    // Pin master + pad volume to unity so the rendered DC *is* the
    // pad-param gain trajectory (defaults are 0.8 / 0.8).
    params.master_volume.set_value(1.0);
    params.pads[pad_index].volume.set_value(1.0);
    sampler.note_on(drum_map::TOM_LOW, 1.0);
    (sampler, params, pad_index, port)
}

#[test]
fn pad_volume_jump_ramps_without_step_discontinuity() {
    let (mut sampler, params, pad_index, port) = tom_setup();

    // Block 1 at the default volume (1.0): flat DC at 1.0.
    let block1 = render_block(&mut sampler, &params);
    let last_before = block1[port].0[FRAMES - 1];
    assert!(
        (last_before - 1.0).abs() < 1e-6,
        "expected unity DC before the jump, got {last_before}"
    );

    // Jump the pad volume, then render block 2: the output must ramp
    // from 1.0 toward 0.25 with no step bigger than one ramp increment.
    let target = 0.25_f32;
    params.pads[pad_index].volume.set_value(target);
    let block2 = render_block(&mut sampler, &params);
    let out = &block2[port].0;
    let step = (target - 1.0) / FRAMES as f32;
    let tol = step.abs() + 1e-5;

    // No discontinuity across the block boundary...
    assert!(
        (out[0] - last_before).abs() <= tol,
        "step across block boundary: {last_before} -> {}",
        out[0]
    );
    // ...nor anywhere inside the block...
    for i in 1..FRAMES {
        assert!(
            (out[i] - out[i - 1]).abs() <= tol,
            "step inside block at {i}: {} -> {}",
            out[i - 1],
            out[i]
        );
    }
    // ...and the ramp lands at the target by block end.
    assert!(
        (out[FRAMES - 1] - target).abs() <= tol,
        "ramp end = {}, want ~{target}",
        out[FRAMES - 1]
    );

    // Block 3 sits flat at the target.
    let block3 = render_block(&mut sampler, &params);
    for (i, &v) in block3[port].0.iter().enumerate() {
        assert!(
            (v - target).abs() < 1e-5,
            "post-ramp block not settled at {i}: {v}"
        );
    }
}

#[test]
fn pad_mute_toggle_ramps_to_silence() {
    let (mut sampler, params, pad_index, port) = tom_setup();
    let _ = render_block(&mut sampler, &params);

    params.pads[pad_index].mute.set_value(true);
    let block = render_block(&mut sampler, &params);
    let out = &block[port].0;
    let tol = 1.0 / FRAMES as f32 + 1e-5;

    assert!((out[0] - 1.0).abs() <= tol, "mute clicked: out[0] = {}", out[0]);
    for i in 1..FRAMES {
        assert!(
            (out[i] - out[i - 1]).abs() <= tol,
            "mute step at {i}: {} -> {}",
            out[i - 1],
            out[i]
        );
    }
    assert!(
        out[FRAMES - 1].abs() <= tol,
        "mute ramp end = {}",
        out[FRAMES - 1]
    );
}

#[test]
fn pad_pan_jump_ramps_both_channels() {
    let (mut sampler, params, pad_index, port) = tom_setup();
    let _ = render_block(&mut sampler, &params);

    // Hard-right: left gain 1.0 -> 0.0, right stays 1.0.
    params.pads[pad_index].pan.set_value(1.0);
    let block = render_block(&mut sampler, &params);
    let (l, r) = &block[port];
    let tol = 1.0 / FRAMES as f32 + 1e-5;

    assert!((l[0] - 1.0).abs() <= tol, "pan clicked left: {}", l[0]);
    for i in 1..FRAMES {
        assert!(
            (l[i] - l[i - 1]).abs() <= tol,
            "pan left step at {i}: {} -> {}",
            l[i - 1],
            l[i]
        );
    }
    assert!(l[FRAMES - 1].abs() <= tol, "pan left end = {}", l[FRAMES - 1]);
    // Right channel stays at unity throughout (stereo_balance keeps the
    // boosted side at 1.0).
    for (i, &v) in r.iter().enumerate() {
        assert!((v - 1.0).abs() <= tol, "pan right moved at {i}: {v}");
    }
}

#[test]
fn first_block_does_not_ramp_in_from_defaults() {
    // Setting a non-default volume before the very first render must
    // produce a flat block at that volume — the prev snapshot is seeded
    // from the first block, not from constructor defaults.
    let mut sampler = make_sampler();
    sampler.pads = dc_pads(FRAMES * 8);
    let params = DrumParams::default();
    let pad_index = drum_map::pad_index_for_note(drum_map::TOM_LOW).unwrap();
    let port = PAD_MAPPINGS[pad_index].output_group.index();
    params.master_volume.set_value(1.0);
    params.pads[pad_index].volume.set_value(0.5);
    sampler.note_on(drum_map::TOM_LOW, 1.0);

    let block = render_block(&mut sampler, &params);
    for (i, &v) in block[port].0.iter().enumerate() {
        assert!(
            (v - 0.5).abs() < 1e-6,
            "first block not flat at 0.5 at {i}: {v}"
        );
    }
}
