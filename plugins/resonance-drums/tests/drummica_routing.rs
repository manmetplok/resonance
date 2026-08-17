//! Regression coverage for kit-load + per-port routing.
//!
//! Loads a non-built-in kit through the loader's public entry point,
//! swaps the resulting pads into a `DrumSampler`, triggers each pad
//! class via `note_on`, and asserts the expected stereo output ports
//! produce non-silent audio. Mirrors the audio-thread sequence in
//! `ResonanceDrums::process` (try_swap_kit -> note_on -> render_block)
//! end-to-end at the sampler level, so regressions in the per-pad
//! output_group / overhead routing fail this test loudly rather than
//! showing up as "the drum plugin is silent" reports.
//!
//! Gated on the Drummica manifest being present on disk. When it isn't
//! the test prints the expected path and exits cleanly so CI without
//! the samples still passes.

use std::path::PathBuf;

use resonance_drums::drum_map::{self, NUM_PADS};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{LoadedPad, NUM_OUTPUT_PORTS, OVERHEAD_PORT_INDEX};
use resonance_drums::kit_loader::{
    load_kit_from_manifest, PadMicChoices, DEFAULT_OVERHEAD_SETUP,
};
use resonance_drums::params::DrumParams;

fn drummica_manifest() -> Option<PathBuf> {
    std::env::var("RESONANCE_DRUMMICA_PATH").ok().map(PathBuf::from)
}

fn default_choices() -> [PadMicChoices; NUM_PADS] {
    std::array::from_fn(|_| PadMicChoices::default())
}

fn default_articulations() -> [bool; NUM_PADS] {
    [false; NUM_PADS]
}

/// Build a sampler with the channel disconnected. Tests poke `pads`
/// directly so the loader-thread plumbing isn't exercised here.
fn make_sampler() -> DrumSampler {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    DrumSampler::new(rx)
}

fn render_one_block(sampler: &mut DrumSampler, frames: usize) -> Vec<(Vec<f32>, Vec<f32>)> {
    let mut port_data: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
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

fn port_energy(port: &(Vec<f32>, Vec<f32>)) -> f32 {
    port.0.iter().chain(port.1.iter()).map(|s| s.abs()).sum()
}

fn load_drummica_pads() -> Option<Vec<LoadedPad>> {
    let Some(manifest) = drummica_manifest() else {
        eprintln!(
            "RESONANCE_DRUMMICA_PATH not set; skipping the real-library test"
        );
        return None;
    };
    assert!(
        manifest.exists(),
        "RESONANCE_DRUMMICA_PATH points at {}, which does not exist",
        manifest.display()
    );
    let kit = load_kit_from_manifest(
        &manifest,
        48000.0,
        DEFAULT_OVERHEAD_SETUP,
        &default_choices(),
        &default_articulations(),
    )
    .expect("drummica kit should load");
    Some(kit.pads)
}

/// A kick triggered on a non-built-in kit must produce audio on the
/// Kick port (1) **and** the shared Overhead port (6). Bug
/// pre-condition: the editor's "Installed kits" flow loads the kit,
/// audio goes silent because the per-pad output_group routing skips
/// the loaded pad. This test fails fast if any of the close-mic banks
/// or the overhead bank lose their voice destination during the
/// kit-load path.
#[test]
fn drummica_kick_routes_to_kick_and_overhead_ports() {
    let Some(pads) = load_drummica_pads() else {
        return;
    };
    let mut sampler = make_sampler();
    sampler.pads = pads;

    sampler.note_on(drum_map::KICK, 0.9);
    let ports = render_one_block(&mut sampler, 256);

    let kick_energy = port_energy(&ports[1]);
    let oh_energy = port_energy(&ports[OVERHEAD_PORT_INDEX]);
    assert!(
        kick_energy > 0.0,
        "Kick port (1) silent on non-built-in kit; energy={kick_energy}"
    );
    assert!(
        oh_energy > 0.0,
        "Overhead port (6) silent on non-built-in kit; energy={oh_energy}"
    );

    // Everything else should be silent — the kick must not bleed into
    // the snare / toms / hats / cymbals sub-tracks.
    for (i, port) in ports.iter().enumerate() {
        if i == 1 || i == OVERHEAD_PORT_INDEX {
            continue;
        }
        let e = port_energy(port);
        assert_eq!(
            e, 0.0,
            "port {i} bled audio on a kick hit (non-built-in kit); energy={e}"
        );
    }
}

/// Snare -> port 2 + Overhead.
#[test]
fn drummica_snare_routes_to_snare_and_overhead_ports() {
    let Some(pads) = load_drummica_pads() else {
        return;
    };
    let mut sampler = make_sampler();
    sampler.pads = pads;

    sampler.note_on(drum_map::SNARE, 0.9);
    let ports = render_one_block(&mut sampler, 256);
    assert!(
        port_energy(&ports[2]) > 0.0,
        "Snare port (2) silent on non-built-in kit"
    );
    assert!(
        port_energy(&ports[OVERHEAD_PORT_INDEX]) > 0.0,
        "Overhead port silent on non-built-in kit snare"
    );
}

/// Tom (high) -> port 3 + Overhead.
#[test]
fn drummica_tom_routes_to_toms_and_overhead_ports() {
    let Some(pads) = load_drummica_pads() else {
        return;
    };
    let mut sampler = make_sampler();
    sampler.pads = pads;

    sampler.note_on(drum_map::TOM_HIGH, 0.9);
    let ports = render_one_block(&mut sampler, 256);
    assert!(
        port_energy(&ports[3]) > 0.0,
        "Toms port (3) silent on non-built-in kit"
    );
    assert!(
        port_energy(&ports[OVERHEAD_PORT_INDEX]) > 0.0,
        "Overhead port silent on non-built-in kit tom"
    );
}

/// Hi-hat -> port 4 + Overhead.
#[test]
fn drummica_hat_routes_to_hats_and_overhead_ports() {
    let Some(pads) = load_drummica_pads() else {
        return;
    };
    let mut sampler = make_sampler();
    sampler.pads = pads;

    sampler.note_on(drum_map::HIHAT_CLOSED, 0.9);
    let ports = render_one_block(&mut sampler, 256);
    assert!(
        port_energy(&ports[4]) > 0.0,
        "Hats port (4) silent on non-built-in kit"
    );
    assert!(
        port_energy(&ports[OVERHEAD_PORT_INDEX]) > 0.0,
        "Overhead port silent on non-built-in kit hat"
    );
}

/// Cymbal pads in Drummica have **no close mic** — they're recorded from
/// the overheads only. The overhead take is therefore the pad's entire
/// sound, so it is summed into the pad's own group port (Cymbals, 5),
/// exactly like the built-in fallback kit does.
///
/// This assertion used to run the other way: the cymbal was sent to the
/// shared Overhead port and the Cymbals port was asserted *silent*. That
/// was incidental — the test was added alongside the multi-packet WAV
/// decode fix to catch a `close_mic_count == 0` short-circuit dropping the
/// cymbal voice entirely, and it pinned whatever port the voice happened to
/// land on. The cost only showed up in the field (ba todo #1232): the host
/// derives one sub-track per output port, so the "Cymbals" sub-track was
/// digital silence for the flagship kit and a client reported "the kit has
/// no cymbals". Writing more cymbal notes changed nothing on that track.
///
/// The point being protected is unchanged — a cymbal hit must make sound —
/// so the `close_mic_count == 0` short-circuit this test was written for
/// still fails it.
#[test]
fn drummica_cymbal_routes_to_the_cymbals_port() {
    let Some(pads) = load_drummica_pads() else {
        return;
    };
    let mut sampler = make_sampler();
    sampler.pads = pads;

    sampler.note_on(drum_map::CRASH_16_EDGE, 0.9);
    let ports = render_one_block(&mut sampler, 256);
    assert!(
        port_energy(&ports[5]) > 0.0,
        "Cymbals port (5) silent on Drummica cymbal hit — the Cymbals \
         sub-track would carry nothing"
    );
    // And it must not also double into the shared Overhead port.
    assert_eq!(
        port_energy(&ports[OVERHEAD_PORT_INDEX]),
        0.0,
        "cymbal must not be summed into Overhead as well as Cymbals"
    );
}

/// Full sequence: kit-swap -> note_on -> render. Mirrors what
/// `process()` does each audio block (try_swap_kit at the top, then
/// drain MIDI, then render). Verifies the swap path itself doesn't
/// leave the sampler in a state where the next note_on quietly drops
/// voices.
#[test]
fn kit_swap_then_kick_produces_audio() {
    let Some(pads) = load_drummica_pads() else {
        return;
    };
    let (tx, rx) = crossbeam_channel::bounded::<Vec<LoadedPad>>(1);
    let mut sampler = DrumSampler::new(rx);
    // Boot with embedded fallback so we have something to swap *from*.
    sampler.load_defaults(48000.0);

    // Confirm the fallback path produces audio first (regression
    // protection against the test framework rather than the SUT).
    sampler.note_on(drum_map::KICK, 0.9);
    let pre = render_one_block(&mut sampler, 256);
    assert!(
        port_energy(&pre[1]) > 0.0,
        "built-in kick should produce audio on Kick port"
    );
    sampler.reset();

    // Swap to the non-built-in kit and trigger again.
    tx.send(pads).unwrap();
    sampler.try_swap_kit();
    sampler.note_on(drum_map::KICK, 0.9);
    let post = render_one_block(&mut sampler, 256);
    assert!(
        port_energy(&post[1]) > 0.0,
        "non-built-in kit kick must produce audio on Kick port after swap"
    );
    assert!(
        port_energy(&post[OVERHEAD_PORT_INDEX]) > 0.0,
        "non-built-in kit kick must produce overhead audio after swap"
    );
}
