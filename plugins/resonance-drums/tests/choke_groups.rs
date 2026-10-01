//! Choke groups as params (drums-plugin-rework.md §7 E12): `pad_N_choke`
//! (0 = none, 1–8) decides which pads cut each other. The hats default
//! to group 1, so an open hat is still choked by the pedal hat; any pads
//! can be put in a group — the toms in group 2 choke each other.

use resonance_drums::drum_map::{self, NUM_PADS, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{LoadedPad, NUM_OUTPUT_PORTS};
use resonance_drums::params::{choke_from_label, choke_label, DrumParams};
use resonance_drums::voice::VoiceState;
use resonance_plugin::Param;

const SR: f32 = 48_000.0;
const BLOCK: usize = 256;

fn sampler() -> DrumSampler {
    let (_tx, rx) = crossbeam_channel::unbounded::<Vec<LoadedPad>>();
    let mut s = DrumSampler::new(rx);
    s.load_defaults(SR);
    s
}

fn render(sampler: &mut DrumSampler, params: &DrumParams, blocks: usize) {
    let mut bufs: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
        .collect();
    for _ in 0..blocks {
        let mut ports: Vec<PortBuffers<'_>> = bufs
            .iter_mut()
            .map(|(l, r)| PortBuffers {
                left: l.as_mut_slice(),
                right: r.as_mut_slice(),
            })
            .collect();
        sampler.render_block(&mut ports, BLOCK, params, &[]);
    }
}

fn pad(note: u8) -> usize {
    drum_map::pad_index_for_note(note).unwrap()
}

/// Whether a voice of `pad` is sounding and not fading.
fn playing(sampler: &DrumSampler, pad: usize) -> bool {
    sampler
        .voices
        .iter()
        .any(|v| v.active && v.pad_index == pad && v.state == VoiceState::Playing)
}

/// Strike `first`, let it ring a little, strike `second`: is `first`
/// cut (fading or gone)?
fn chokes(params: &DrumParams, first: u8, second: u8) -> bool {
    let mut s = sampler();
    s.update_global_settings(params);
    s.note_on(first, 0.9);
    render(&mut s, params, 2);
    assert!(
        playing(&s, pad(first)),
        "note {first} sounds before the second hit"
    );
    s.note_on(second, 0.9);
    !playing(&s, pad(first))
}

#[test]
fn hats_default_to_group_one_and_nothing_else_is_choked() {
    let params = DrumParams::default();
    for (i, mapping) in PAD_MAPPINGS.iter().enumerate() {
        let want = if mapping.name.contains("Hi-Hat") {
            1
        } else {
            0
        };
        assert_eq!(
            params.pads[i].choke.value(),
            want,
            "pad {i} ({}) default choke group",
            mapping.name
        );
        assert_eq!(params.pads[i].choke.id(), format!("pad_{i}_choke"));
    }
    assert!((0..NUM_PADS).any(|i| params.pads[i].choke.value() == 1));
}

#[test]
fn an_open_hat_is_choked_by_the_pedal_hat_by_default() {
    let params = DrumParams::default();
    assert!(chokes(&params, drum_map::HIHAT_OPEN, drum_map::HIHAT_PEDAL));
    assert!(chokes(
        &params,
        drum_map::HIHAT_OPEN,
        drum_map::HIHAT_CLOSED
    ));
}

#[test]
fn a_hat_taken_out_of_its_group_rings_on() {
    let params = DrumParams::default();
    params.pads[pad(drum_map::HIHAT_OPEN)].choke.set_value(0);
    assert!(!chokes(
        &params,
        drum_map::HIHAT_OPEN,
        drum_map::HIHAT_PEDAL
    ));
}

#[test]
fn toms_in_group_two_choke_each_other() {
    let params = DrumParams::default();
    // Not by default.
    assert!(!chokes(&params, drum_map::TOM_HIGH, drum_map::TOM_MID));
    for note in [drum_map::TOM_HIGH, drum_map::TOM_MID, drum_map::TOM_LOW] {
        params.pads[pad(note)].choke.set_value(2);
    }
    assert!(chokes(&params, drum_map::TOM_HIGH, drum_map::TOM_MID));
    assert!(chokes(&params, drum_map::TOM_MID, drum_map::TOM_LOW));
    // Group 2 leaves group 1 alone, and the reverse.
    assert!(!chokes(&params, drum_map::TOM_HIGH, drum_map::HIHAT_PEDAL));
    assert!(!chokes(&params, drum_map::HIHAT_OPEN, drum_map::TOM_LOW));
}

#[test]
fn the_choke_choice_reads_and_parses() {
    let p = DrumParams::default();
    let c = &p.pads[0].choke;
    assert_eq!(c.min_plain(), 0.0);
    assert_eq!(c.max_plain(), 8.0);
    assert_eq!(c.display(0.0), "None");
    assert_eq!(c.display(3.0), "Group 3");
    assert_eq!(c.parse("Group 2"), Some(2.0));
    assert_eq!(c.parse("none"), Some(0.0));
    assert_eq!(c.parse("5"), Some(5.0));
    assert_eq!(choke_label(8), "Group 8");
    assert_eq!(choke_from_label("group 12"), Some(8), "clamped");
    assert_eq!(choke_from_label("hats"), None);
}
