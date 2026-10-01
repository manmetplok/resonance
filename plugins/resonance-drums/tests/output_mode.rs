//! Output mode and per-pad output (drums-plugin-rework.md §7 E11, D5).
//!
//! - **Stereo**, the default for a fresh instance: the whole kit — close
//!   mics and overheads — on Main, every other port silent. Port 0 is
//!   the full kit.
//! - **Multi**: each pad's close mics on its `pad_N_output` port (by
//!   default the Drummica table), and a close-miked pad's overhead take
//!   on the Overhead port. A pad with no close mic (the cymbals, recorded
//!   on the overheads only) keeps its overhead take — its whole sound —
//!   on its own port, so the Cymbals sub-track is not silent (ba #1232).

use resonance_drums::drum_map::{self, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{
    LoadedMicBank, LoadedPad, LoadedSample, VelocityLayer, MAIN_PORT_INDEX, NUM_OUTPUT_PORTS,
    OUTPUT_PORT_NAMES, OVERHEAD_PORT_INDEX,
};
use resonance_drums::params::{
    output_choice_for_port, DrumParams, OUTPUT_KIT, OUTPUT_MODE_MULTI, OUTPUT_MODE_STEREO,
};
use resonance_plugin::Param;

const SR: f32 = 48_000.0;
/// The Cymbals port (`OutputGroup::Cymbals`).
const CYMBALS_PORT: usize = 5;
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
        layers: vec![VelocityLayer::new(vec![tone(seed, BLOCK * BLOCKS)])],
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
    render_kit(params, drummica_shaped_kit(), notes)
}

/// [`render`] on `kit`.
fn render_kit(params: &DrumParams, kit: Vec<LoadedPad>, notes: &[u8]) -> Vec<(Vec<f32>, Vec<f32>)> {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut sampler = DrumSampler::new(rx);
    sampler.set_sample_rate(SR);
    sampler.pads = kit;
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
    for i in 0..PAD_MAPPINGS.len() {
        let out = &p.pads[i].output;
        assert_eq!(out.id(), format!("pad_{i}_output"));
        assert_eq!(out.value(), OUTPUT_KIT, "pad {i}: the kit's port by default");
        assert_eq!(out.display(OUTPUT_KIT as f64), "Kit");
        assert_eq!(out.max_plain() as usize, NUM_OUTPUT_PORTS);
        for (port, name) in OUTPUT_PORT_NAMES.iter().enumerate() {
            assert_eq!(out.display(output_choice_for_port(port) as f64), *name);
        }
        assert!(!out.is_automatable());
    }
    assert_eq!(
        p.pads[0].output.parse("Overhead"),
        Some(output_choice_for_port(OVERHEAD_PORT_INDEX) as f64)
    );
    assert_eq!(p.pads[0].output.parse("Kit"), Some(OUTPUT_KIT as f64));
}

/// "Kit" plays the port the loaded kit gives the pad (its `_meta.pads`
/// hint, here a kick routed to Toms); an explicit port overrides it. The
/// param never takes the hint's value.
#[test]
fn kit_follows_the_kits_port_hint_and_an_explicit_port_overrides_it() {
    let params = DrumParams::default();
    params.output_mode.set_value(OUTPUT_MODE_MULTI);
    let kick = drum_map::pad_index_for_note(drum_map::KICK).unwrap();
    let mut kit = drummica_shaped_kit();
    kit[kick].output_group = resonance_drums::kit::OutputGroup::Toms;
    let ports = render_kit(&params, kit.clone(), &[drum_map::KICK]);
    assert!(rms(&ports[3]) > 0.01, "the kit's hint: Toms");
    assert_eq!(rms(&ports[1]), 0.0);
    assert_eq!(params.pads[kick].output.value(), OUTPUT_KIT);

    params.pads[kick]
        .output
        .set_value(output_choice_for_port(MAIN_PORT_INDEX));
    let ports = render_kit(&params, kit, &[drum_map::KICK]);
    assert!(rms(&ports[MAIN_PORT_INDEX]) > 0.01, "the user's Main wins");
    assert_eq!(rms(&ports[3]), 0.0);
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

/// A cymbal has no close mic: its overhead take is its whole sound, so in
/// Multi it plays on the cymbal's own port (Cymbals) — the Cymbals
/// sub-track is not silent (ba #1232) — not on Overhead.
#[test]
fn multi_keeps_the_overhead_only_cymbals_on_their_own_port() {
    let params = DrumParams::default();
    params.output_mode.set_value(OUTPUT_MODE_MULTI);
    let ports = render(&params, &[drum_map::CRASH_16_EDGE, drum_map::RIDE_TIP]);
    assert!(rms(&ports[CYMBALS_PORT]) > 0.01, "the cymbals are on Cymbals");
    for (port, data) in ports.iter().enumerate() {
        if port != CYMBALS_PORT {
            assert_eq!(rms(data), 0.0, "the cymbal leaked onto port {port}");
        }
    }

    // It follows the pad's `pad_N_output`, like a close mic.
    let crash = drum_map::pad_index_for_note(drum_map::CRASH_16_EDGE).unwrap();
    params.pads[crash]
        .output
        .set_value(output_choice_for_port(MAIN_PORT_INDEX));
    let ports = render(&params, &[drum_map::CRASH_16_EDGE]);
    assert!(rms(&ports[MAIN_PORT_INDEX]) > 0.01);
    assert_eq!(rms(&ports[CYMBALS_PORT]), 0.0);
    assert_eq!(rms(&ports[OVERHEAD_PORT_INDEX]), 0.0);
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
    params.pads[kick].output.set_value(output_choice_for_port(3));
    let ports = render(&params, &[drum_map::KICK]);
    assert_eq!(rms(&ports[1]), 0.0, "nothing left on Kick");
    assert!(rms(&ports[3]) > 0.01, "the kick's close mics are on Toms");
    assert!(
        rms(&ports[OVERHEAD_PORT_INDEX]) > 0.01,
        "its overhead stays"
    );

    // To Main.
    params.pads[kick]
        .output
        .set_value(output_choice_for_port(MAIN_PORT_INDEX));
    let ports = render(&params, &[drum_map::KICK]);
    assert!(rms(&ports[MAIN_PORT_INDEX]) > 0.01);
    // pad_N_output does nothing in Stereo: everything is on Main anyway.
    params.output_mode.set_value(OUTPUT_MODE_STEREO);
    params.pads[kick].output.set_value(output_choice_for_port(5));
    let ports = render(&params, &[drum_map::KICK]);
    assert_eq!(rms(&ports[5]), 0.0);
    assert!(rms(&ports[MAIN_PORT_INDEX]) > 0.01);
}
