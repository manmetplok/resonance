//! Kit swap declick: voices still sounding when a new kit is swapped in
//! must fade out over `RELEASE_SAMPLES` (reading the retired kit's
//! sample data) instead of being hard-cut, so the swap never clicks.

use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};
use resonance_drums::params::DrumParams;
use resonance_drums::voice::RELEASE_SAMPLES;

const NUM_PORTS: usize = 7;
const FRAMES: usize = 256;

/// A long constant-DC stereo sample, so the rendered output *is* the
/// effective gain trajectory.
fn dc_layer(level: f32, frames: usize) -> VelocityLayer {
    VelocityLayer {
        round_robins: vec![LoadedSample {
            data: vec![level; frames * 2],
            frames,
        }],
    }
}

/// Build a kit where every pad has exactly one close mic playing DC at
/// `level` (BalanceSide::None) and no overhead, so each hit renders one
/// voice.
fn dc_pads(level: f32, sample_frames: usize) -> Vec<LoadedPad> {
    PAD_MAPPINGS
        .iter()
        .map(|m| LoadedPad {
            name: m.name.to_string(),
            choke_group: None,
            output_group: m.output_group,
            close_mics: vec![LoadedMicBank {
                position: "test".to_string(),
                setup_key: String::new(),
                layers: vec![dc_layer(level, sample_frames)],
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

/// Sampler playing the low tom from an old DC-1.0 kit, plus the channel
/// sender used to push replacement kits.
fn swap_setup() -> (
    DrumSampler,
    DrumParams,
    crossbeam_channel::Sender<Vec<LoadedPad>>,
    usize,
) {
    let (tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.pads = dc_pads(1.0, FRAMES * 32);
    let params = DrumParams::default();
    let pad_index = drum_map::pad_index_for_note(drum_map::TOM_LOW).unwrap();
    let port = PAD_MAPPINGS[pad_index].output_group.index();
    // Pin master + pad volume to unity so the rendered DC *is* the
    // voice's envelope.
    params.master_volume.set_value(1.0);
    params.pads[pad_index].volume.set_value(1.0);
    sampler.note_on(drum_map::TOM_LOW, 1.0);
    (sampler, params, tx, port)
}

#[test]
fn kit_swap_fades_ringing_voices_without_click() {
    let (mut sampler, params, tx, port) = swap_setup();

    // Block 1 on the old kit: flat DC at 1.0.
    let block1 = render_block(&mut sampler, &params);
    let mut prev = block1[port].0[FRAMES - 1];
    assert!(
        (prev - 1.0).abs() < 1e-6,
        "expected unity DC before the swap, got {prev}"
    );

    // Swap in a new (silent) kit mid-ring.
    tx.send(dc_pads(0.0, FRAMES * 32)).unwrap();
    sampler.try_swap_kit();

    // The fade slope is 1.0 / RELEASE_SAMPLES per sample; no
    // sample-to-sample jump may exceed it (plus float slack).
    let tol = 1.0 / RELEASE_SAMPLES as f32 + 1e-5;
    let fade_blocks = RELEASE_SAMPLES / FRAMES;
    for block in 0..fade_blocks {
        let out = render_block(&mut sampler, &params);
        for (i, &v) in out[port].0[..FRAMES].iter().enumerate() {
            assert!(
                (v - prev).abs() <= tol,
                "click at block {block} sample {i}: {prev} -> {v}"
            );
            prev = v;
        }
    }
    // The fade must have reached silence by RELEASE_SAMPLES.
    assert!(
        prev.abs() <= tol,
        "fade should end at silence, got {prev}"
    );

    // The block after the fade is fully silent.
    let after = render_block(&mut sampler, &params);
    for (i, &v) in after[port].0.iter().enumerate() {
        assert!(v.abs() < 1e-6, "post-fade output not silent at {i}: {v}");
    }
}

#[test]
fn fading_voices_read_old_kit_data() {
    let (mut sampler, params, tx, port) = swap_setup();
    let _ = render_block(&mut sampler, &params);

    // The new kit plays DC 0.5 — clearly distinguishable from the old
    // kit's DC 1.0. No new note is triggered, so any output after the
    // swap can only come from the old kit's samples.
    tx.send(dc_pads(0.5, FRAMES * 32)).unwrap();
    sampler.try_swap_kit();

    let out = render_block(&mut sampler, &params);
    let first = out[port].0[0];
    assert!(
        first > 0.9,
        "fading voice should still read the old kit's DC 1.0, got {first}"
    );
}

#[test]
fn note_after_swap_plays_new_kit() {
    let (mut sampler, params, tx, port) = swap_setup();
    let _ = render_block(&mut sampler, &params);

    tx.send(dc_pads(0.5, FRAMES * 32)).unwrap();
    sampler.try_swap_kit();

    // Let the old voice fade out completely.
    for _ in 0..=RELEASE_SAMPLES / FRAMES {
        let _ = render_block(&mut sampler, &params);
    }

    // A fresh hit renders the new kit's DC 0.5, flat.
    sampler.note_on(drum_map::TOM_LOW, 1.0);
    let out = render_block(&mut sampler, &params);
    for (i, &v) in out[port].0.iter().enumerate() {
        assert!(
            (v - 0.5).abs() < 1e-5,
            "new-kit hit should be flat DC 0.5 at {i}: {v}"
        );
    }
}

#[test]
fn second_swap_mid_fade_cuts_retired_voices() {
    let (mut sampler, params, tx, port) = swap_setup();
    let _ = render_block(&mut sampler, &params);

    // First swap starts the fade; second swap lands before it ends, so
    // the retired voices lose their data and must be cut.
    tx.send(dc_pads(0.5, FRAMES * 32)).unwrap();
    sampler.try_swap_kit();
    let _ = render_block(&mut sampler, &params);
    tx.send(dc_pads(0.25, FRAMES * 32)).unwrap();
    sampler.try_swap_kit();

    // No voice was playing kit 2, so the output is silent — and the
    // cut voices from kit 1 must not render against kit 3's data.
    let out = render_block(&mut sampler, &params);
    for (i, &v) in out[port].0.iter().enumerate() {
        assert!(v.abs() < 1e-6, "expected silence after double swap at {i}: {v}");
    }

    // A fresh hit still works against kit 3.
    sampler.note_on(drum_map::TOM_LOW, 1.0);
    let after = render_block(&mut sampler, &params);
    assert!(
        (after[port].0[0] - 0.25).abs() < 1e-5,
        "hit after double swap should play kit 3's DC 0.25, got {}",
        after[port].0[0]
    );
}
