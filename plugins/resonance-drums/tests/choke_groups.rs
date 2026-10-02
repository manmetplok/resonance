//! Choke groups as params (drums-plugin-rework.md §7 E12): `pad_N_choke`
//! (Kit, 0 = none, 1–8) decides which pads cut each other. Every pad
//! defaults to "Kit" — the group the loaded kit gives it, which puts the
//! hats in group 1, so an open hat is still choked by the pedal hat; any
//! pads can be put in a group — the toms in group 2 choke each other.

use resonance_drums::drum_map::{self, NUM_PADS, PAD_MAPPINGS};
use resonance_drums::dsp::{DrumSampler, PortBuffers};
use resonance_drums::kit::{LoadedPad, NUM_OUTPUT_PORTS};
use resonance_drums::dsp::FROM_KIT;
use resonance_drums::params::{choke_from_label, choke_label, DrumParams, CHOKE_KIT};
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
    chokes_on(&mut sampler(), params, first, second)
}

/// [`chokes`] on the caller's sampler (a kit of its own).
fn chokes_on(s: &mut DrumSampler, params: &DrumParams, first: u8, second: u8) -> bool {
    s.update_global_settings(params);
    s.note_on(first, 0.9);
    render(s, params, 2);
    assert!(
        playing(s, pad(first)),
        "note {first} sounds before the second hit"
    );
    s.note_on(second, 0.9);
    !playing(s, pad(first))
}

/// Every pad defaults to "Kit": the group the loaded kit gives it — for
/// the built-in kit and Drummica the table, every hi-hat in group 1 and
/// nothing else choked.
#[test]
fn every_pad_defaults_to_the_kits_group_and_the_hats_are_group_one() {
    let params = DrumParams::default();
    let mut s = sampler();
    s.update_global_settings(&params);
    for (i, mapping) in PAD_MAPPINGS.iter().enumerate() {
        assert_eq!(params.pads[i].choke.value(), CHOKE_KIT, "pad {i}");
        assert_eq!(params.pads[i].choke.id(), format!("pad_{i}_choke"));
        let want = mapping.name.contains("Hi-Hat").then_some(1);
        assert_eq!(s.pads[i].choke_group, want, "pad {i} ({})", mapping.name);
        assert_eq!(s.pad_settings(i).choke, FROM_KIT);
    }
    assert!((0..NUM_PADS).any(|i| s.pads[i].choke_group == Some(1)));
}

/// A kit's choke hint (`_meta.pads`) is what "Kit" plays: here two toms
/// in group 3. An explicit value overrides it; the param never takes the
/// hint's value.
#[test]
fn kit_follows_the_kits_choke_hint_and_an_explicit_group_overrides_it() {
    let params = DrumParams::default();
    let hinted = || {
        let mut s = sampler();
        for note in [drum_map::TOM_HIGH, drum_map::TOM_MID] {
            s.pads[pad(note)].choke_group = Some(3);
        }
        s
    };
    assert!(chokes_on(&mut hinted(), &params, drum_map::TOM_HIGH, drum_map::TOM_MID));
    assert_eq!(params.pads[pad(drum_map::TOM_HIGH)].choke.value(), CHOKE_KIT);

    params.pads[pad(drum_map::TOM_MID)].choke.set_value(0);
    assert!(!chokes_on(&mut hinted(), &params, drum_map::TOM_HIGH, drum_map::TOM_MID));
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
    assert_eq!(c.min_plain(), CHOKE_KIT as f64);
    assert_eq!(c.max_plain(), 8.0);
    assert_eq!(c.display(-1.0), "Kit");
    assert_eq!(c.parse("kit"), Some(-1.0));
    assert_eq!(c.display(0.0), "None");
    assert_eq!(c.display(3.0), "Group 3");
    assert_eq!(c.parse("Group 2"), Some(2.0));
    assert_eq!(c.parse("none"), Some(0.0));
    assert_eq!(c.parse("5"), Some(5.0));
    assert_eq!(choke_label(8), "Group 8");
    assert_eq!(choke_from_label("group 12"), Some(8), "clamped");
    assert_eq!(choke_from_label("hats"), None);
}
