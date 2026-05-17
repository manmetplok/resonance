use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::kit::{LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer};
use resonance_drums::params::DrumParams;
use resonance_drums::dsp::{DrumSampler, PortBuffers};

const NUM_PORTS: usize = 7;
const TOMS_PORT: usize = 3;
const OVERHEAD_PORT: usize = 6;

fn make_sampler() -> DrumSampler {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    DrumSampler::new(rx)
}

fn impulse_layer(value: f32) -> VelocityLayer {
    VelocityLayer {
        round_robins: vec![LoadedSample {
            data: vec![value; 64],
            frames: 32,
        }],
    }
}

fn make_pad_with_oh(mapping_index: usize) -> LoadedPad {
    let m = &PAD_MAPPINGS[mapping_index];
    let close_mics: Vec<LoadedMicBank> = m
        .close_mic_positions
        .iter()
        .map(|pos| LoadedMicBank {
            position: pos.to_string(),
            setup_key: String::new(),
            layers: vec![impulse_layer(0.5)],
        })
        .collect();
    LoadedPad {
        name: m.name.to_string(),
        choke_group: m.choke_group,
        output_group: m.output_group,
        close_mics,
        overhead: Some(LoadedMicBank {
            position: "OHsAB".to_string(),
            setup_key: String::new(),
            layers: vec![impulse_layer(0.25)],
        }),
    }
}

fn make_silent_pad(mapping_index: usize) -> LoadedPad {
    let m = &PAD_MAPPINGS[mapping_index];
    LoadedPad {
        name: m.name.to_string(),
        choke_group: m.choke_group,
        output_group: m.output_group,
        close_mics: Vec::new(),
        overhead: None,
    }
}

fn render_one_block(sampler: &mut DrumSampler, frames: usize) -> Vec<(Vec<f32>, Vec<f32>)> {
    let mut port_data: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_PORTS)
        .map(|_| (vec![0.0; frames], vec![0.0; frames]))
        .collect();
    {
        let mut ports: Vec<PortBuffers<'_>> = port_data
            .iter_mut()
            .map(|(l, r)| PortBuffers {
                left: l.as_mut_slice(),
                right: r.as_mut_slice(),
            })
            .collect();
        sampler.render_block(&mut ports, frames, &DrumParams::default());
    }
    port_data
}

#[test]
fn tom_high_hit_renders_to_toms_and_overhead_ports() {
    let mut sampler = make_sampler();
    sampler.pads = (0..PAD_MAPPINGS.len())
        .map(|i| {
            if i == 9 || i == 10 || i == 11 {
                make_pad_with_oh(i)
            } else {
                make_silent_pad(i)
            }
        })
        .collect();

    sampler.note_on(drum_map::TOM_HIGH, 0.8);
    let ports = render_one_block(&mut sampler, 16);

    let toms_sum: f32 = ports[TOMS_PORT].0.iter().map(|s| s.abs()).sum();
    let oh_sum: f32 = ports[OVERHEAD_PORT].0.iter().map(|s| s.abs()).sum();
    let kick_sum: f32 = ports[1].0.iter().map(|s| s.abs()).sum();
    let snare_sum: f32 = ports[2].0.iter().map(|s| s.abs()).sum();
    let hats_sum: f32 = ports[4].0.iter().map(|s| s.abs()).sum();

    assert!(
        toms_sum > 0.0,
        "Toms port (3) should have audio after a tom hit, got sum={toms_sum}"
    );
    assert!(
        oh_sum > 0.0,
        "Overhead port (6) should have audio, got sum={oh_sum}"
    );
    assert_eq!(kick_sum, 0.0, "Kick port should be silent");
    assert_eq!(snare_sum, 0.0, "Snare port should be silent");
    assert_eq!(hats_sum, 0.0, "Hats port should be silent");
}

#[test]
fn tom_mid_and_floor_also_route_to_toms_port() {
    let mut sampler = make_sampler();
    sampler.pads = (0..PAD_MAPPINGS.len())
        .map(|i| {
            if i == 9 || i == 10 || i == 11 {
                make_pad_with_oh(i)
            } else {
                make_silent_pad(i)
            }
        })
        .collect();

    for note in [drum_map::TOM_MID, drum_map::TOM_LOW] {
        sampler.reset();
        sampler.note_on(note, 0.8);
        let ports = render_one_block(&mut sampler, 16);
        let toms_sum: f32 = ports[TOMS_PORT].0.iter().map(|s| s.abs()).sum();
        let oh_sum: f32 = ports[OVERHEAD_PORT].0.iter().map(|s| s.abs()).sum();
        assert!(
            toms_sum > 0.0,
            "Toms port silent for note {note}, sum={toms_sum}"
        );
        assert!(oh_sum > 0.0, "OH port silent for note {note}, sum={oh_sum}");
    }
}
