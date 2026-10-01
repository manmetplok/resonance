//! Output mode and per-pad output (drums-plugin-rework.md §7 E11, D5).
//!
//! - **Stereo**, the default for a fresh instance: the whole kit — close
//!   mics and overheads — on Main, every other port silent. Port 0 is
//!   the full kit.
//! - **Multi**: each pad's close mics on its `pad_N_output` port (by
//!   default the Drummica table), and every pad's overhead take on the
//!   Overhead port — the cymbals' included.

use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{
    LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, MAIN_PORT_INDEX, NUM_OUTPUT_PORTS,
    OUTPUT_PORT_NAMES, OVERHEAD_PORT_INDEX,
};
use resonance_drums::params::{DrumParams, OUTPUT_MODE_MULTI, OUTPUT_MODE_STEREO};
use resonance_plugin::Param;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;
const BLOCKS: usize = 8;

/// A decaying tone, so every pad and bank is a different, never-silent
/// signal.
fn tone(seed: usize, frames: usize) -> LoadedSample {
    let data = (0..frames)
        .flat_map(|i| {
            let t = i as f32;
            let s = (t * (0.01 + seed as f32 * 0.002)).sin() * (1.0 - t / frames as f32) * 0.3;
            [s, s * 0.9]
        })
        .collect();
    LoadedSample::from_data(data)
}

fn bank(position: &str, seed: usize) -> LoadedMicBank {
    LoadedMicBank {
        position: position.to_string(),
        setup_key: String::new(),
        layers: vec![VelocityLayer {
            round_robins: vec![tone(seed, BLOCK * BLOCKS)],
        }],
    }
}

/// Shaped the way the Drummica loader builds a kit: a close bank per
/// mapped close-mic position (none for the cymbals) and an overhead bank
/// on every pad.
fn drummica_shaped_kit() -> Vec<LoadedPad> {
    PAD_MAPPINGS
        .iter()
        .enumerate()
        .map(|(i, m)| LoadedPad {
            name: m.name.to_string(),
            choke_group: m.choke_group,
            output_group: m.output_group,
            close_mics: m
                .close_mic_positions
                .iter()
                .enumerate()
                .map(|(b, pos)| bank(pos, i * 3 + b))
                .collect(),
            overhead: Some(bank("OHsAB", i * 3 + 2)),
        })
        .collect()
}

/// Render `notes` (all on frame 0) with `params` applied; every port.
fn render(params: &DrumParams, notes: &[u8]) -> Vec<(Vec<f32>, Vec<f32>)> {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.set_sample_rate(SR);
    sampler.pads = drummica_shaped_kit();
    sampler.update_global_settings(params);
    for &note in notes {
        sampler.note_on(note, 0.9);
    }
    let mut out: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (Vec::new(), Vec::new()))
        .collect();
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
        .collect();
    for _ in 0..BLOCKS {
        {
            let mut ports: Vec<PortBuffers<'_>> = bufs
                .iter_mut()
                .map(|(l, r)| PortBuffers {
                    left: l.as_mut_slice(),
                    right: r.as_mut_slice(),
                })
                .collect();
            sampler.render_block(&mut ports, BLOCK, params, &[]);
        }
        for (o, b) in out.iter_mut().zip(&bufs) {
            o.0.extend_from_slice(&b.0);
            o.1.extend_from_slice(&b.1);
        }
    }
    out
}

fn rms(port: &(Vec<f32>, Vec<f32>)) -> f32 {
    let n = (port.0.len() + port.1.len()) as f32;
    (port.0.iter().chain(&port.1).map(|s| s * s).sum::<f32>() / n).sqrt()
}

/// One hit on every pad.
fn every_pad() -> Vec<u8> {
    PAD_MAPPINGS.iter().map(|m| m.note).collect()
}

#[test]
fn a_fresh_instance_is_stereo_and_its_ports_default_to_the_drummica_table() {
    let p = DrumParams::default();
    assert_eq!(
        p.output_mode.value(),
        OUTPUT_MODE_STEREO,
        "D5: Stereo by default"
    );
    assert_eq!(p.output_mode.display(0.0), "Stereo");
    assert_eq!(p.output_mode.display(1.0), "Multi");
    assert!(!p.output_mode.is_automatable());
    for (i, m) in PAD_MAPPINGS.iter().enumerate() {
        let out = &p.pads[i].output;
        assert_eq!(out.id(), format!("pad_{i}_output"));
        assert_eq!(out.value(), m.output_group.index() as i32, "pad {i}");
        assert_eq!(out.max_plain() as usize, NUM_OUTPUT_PORTS - 1);
        assert_eq!(
            out.display(out.value() as f64),
            OUTPUT_PORT_NAMES[m.output_group.index()]
        );
        assert!(!out.is_automatable());
    }
    assert_eq!(
        p.pads[0].output.parse("Overhead"),
        Some(OVERHEAD_PORT_INDEX as f64)
    );
}

#[test]
fn stereo_puts_the_whole_kit_on_main() {
    let stereo = DrumParams::default();
    let multi = DrumParams::default();
    multi.output_mode.set_value(OUTPUT_MODE_MULTI);
    let notes = every_pad();
    let s = render(&stereo, &notes);
    let m = render(&multi, &notes);

    // Nothing anywhere but Main.
    for (port, data) in s.iter().enumerate().skip(1) {
        assert_eq!(rms(data), 0.0, "Stereo leaked onto port {port}");
    }
    // Main carries the full kit: what Multi spreads over all seven ports.
    let frames = s[0].0.len();
    let mut multi_sum = (vec![0.0f32; frames], vec![0.0f32; frames]);
    for (l, r) in &m {
        for i in 0..frames {
            multi_sum.0[i] += l[i];
            multi_sum.1[i] += r[i];
        }
    }
    let main = rms(&s[MAIN_PORT_INDEX]);
    let full = rms(&multi_sum);
    assert!(full > 0.01, "the kit sounds ({full})");
    assert!(
        (main - full).abs() / full < 1e-4,
        "port 0 in Stereo ({main}) is the full kit ({full})"
    );
    for i in 0..frames {
        assert!(
            (s[0].0[i] - multi_sum.0[i]).abs() < 1e-5,
            "frame {i} differs"
        );
    }
}

#[test]
fn multi_puts_the_cymbals_overheads_on_the_overhead_port() {
    let params = DrumParams::default();
    params.output_mode.set_value(OUTPUT_MODE_MULTI);
    let ports = render(&params, &[drum_map::CRASH_16_EDGE, drum_map::RIDE_TIP]);
    assert!(
        rms(&ports[OVERHEAD_PORT_INDEX]) > 0.01,
        "the cymbal is on Overhead"
    );
    for (port, data) in ports.iter().enumerate() {
        if port != OVERHEAD_PORT_INDEX {
            assert_eq!(rms(data), 0.0, "the cymbal leaked onto port {port}");
        }
    }
}

#[test]
fn multi_routes_close_mics_by_pad_output_and_overheads_to_overhead() {
    let params = DrumParams::default();
    params.output_mode.set_value(OUTPUT_MODE_MULTI);
    // Default: the kick's close mics on Kick, its overhead on Overhead.
    let ports = render(&params, &[drum_map::KICK]);
    assert!(rms(&ports[1]) > 0.01, "the kick is on Kick");
    assert!(rms(&ports[OVERHEAD_PORT_INDEX]) > 0.01);
    assert_eq!(rms(&ports[MAIN_PORT_INDEX]), 0.0);

    // Re-routed to the Toms port: the close mics follow, the overhead not.
    let kick = drum_map::pad_index_for_note(drum_map::KICK).unwrap();
    params.pads[kick].output.set_value(3);
    let ports = render(&params, &[drum_map::KICK]);
    assert_eq!(rms(&ports[1]), 0.0, "nothing left on Kick");
    assert!(rms(&ports[3]) > 0.01, "the kick's close mics are on Toms");
    assert!(
        rms(&ports[OVERHEAD_PORT_INDEX]) > 0.01,
        "its overhead stays"
    );

    // To Main.
    params.pads[kick].output.set_value(0);
    let ports = render(&params, &[drum_map::KICK]);
    assert!(rms(&ports[MAIN_PORT_INDEX]) > 0.01);
    // pad_N_output does nothing in Stereo: everything is on Main anyway.
    params.output_mode.set_value(OUTPUT_MODE_STEREO);
    params.pads[kick].output.set_value(5);
    let ports = render(&params, &[drum_map::KICK]);
    assert_eq!(rms(&ports[5]), 0.0);
    assert!(rms(&ports[MAIN_PORT_INDEX]) > 0.01);
}
