//! Kit-driven pads (drums-plugin-rework.md §7 E10, D7): piece names and
//! articulation labels from `_meta`, the `_meta.pads` overrides of the
//! Drummica table, and pads the kit lacks — silent and dimmed, never
//! filled from the built-in samples.
//!
//! The kits are the checked-in fixtures under `tests/fixtures/kit_pads`
//! (regenerate them by hand if their shape changes; never point a default
//! test at a real kit):
//!
//! - `it_techno`: shaped like IT Techno — "SD Count Stick" is "Perc
//!   Conga", "SD Snare Handtuch" is "Clap", the kick and snare pair as
//!   "punch/deep" and "snap/body", and there are no toms.
//! - `overrides`: the same pieces, with `_meta.pads` moving the conga onto
//!   the Tom High note (with a port and choke hint).
//! - `drummica_like`: no `_meta` at all; kick, snare and Tom01 with the
//!   Drummica alternates of the kick and the tom.
//!
//! Each take is a constant level that differs per piece, so which piece
//! a pad played is audible in the rendered block.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use resonance_common::drumkit_library::KitMeta;
use resonance_drums::articulation::{self, ARTICULATION_ALT, ARTICULATION_PRIMARY};
use resonance_drums::drum_map::{self, NUM_PADS, PAD_MAPPINGS};
use resonance_drums::kit::{OutputGroup, NUM_OUTPUT_PORTS};
use resonance_drums::kit_loader::{
    load_kit_from_manifest, spawn_loader, KitStatus, PadMicChoices, DEFAULT_OVERHEAD_SETUP,
};
use resonance_drums::pad_map::{self, KitPads};
use resonance_drums::ResonanceDrums;
use resonance_plugin::param::Param;
use resonance_plugin::{EventIterator, NoteEvent, OutputBuffer, ResonancePlugin};

const RATE: f32 = 48_000.0;
const BLOCK: usize = 128;

const TOM_PADS: [usize; 3] = [9, 10, 11];
const TOM_NOTES: [u8; 3] = [drum_map::TOM_HIGH, drum_map::TOM_MID, drum_map::TOM_LOW];
const KICK_PAD: usize = 0;
const SNARE_PAD: usize = 1;
const HAT_PAD: usize = 2;
const TOM_HIGH_PAD: usize = 9;
const CLAP_PAD: usize = 28;
const COUNT_STICK_PAD: usize = 29;

fn fixture(kit: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/kit_pads")
        .join(kit)
        .join("drum_samples.json")
}

fn no_choices() -> [PadMicChoices; NUM_PADS] {
    std::array::from_fn(|_| PadMicChoices::default())
}

/// The loader's pad map for a fixture kit, as a load builds it.
fn loaded(kit: &str) -> resonance_drums::kit_loader::LoadedKit {
    load_kit_from_manifest(
        &fixture(kit),
        RATE,
        DEFAULT_OVERHEAD_SETUP,
        &no_choices(),
        &[false; NUM_PADS],
    )
    .unwrap_or_else(|e| panic!("{kit} should load: {e}"))
}

// ---------------------------------------------------------------------------
// Plugin helpers
// ---------------------------------------------------------------------------

fn booted() -> ResonanceDrums {
    let mut plugin = ResonanceDrums::new();
    assert!(plugin.initialize(RATE, BLOCK as u32));
    plugin
}

/// Load `kit` into `plugin` the way the kit picker does, and wait for it.
fn load(plugin: &ResonanceDrums, kit: &str) {
    spawn_loader(
        fixture(kit),
        RATE,
        &plugin.bridge,
        DEFAULT_OVERHEAD_SETUP.to_string(),
        no_choices(),
        plugin.bridge.articulations(),
    );
    settle(plugin);
}

fn settle(plugin: &ResonanceDrums) {
    let bridge = &plugin.bridge;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let pending = bridge.pending_kit.lock().is_some();
        let status = bridge.kit_status.lock().clone();
        match status {
            KitStatus::Loaded { .. } if !pending => return,
            KitStatus::Error { message } if !pending => panic!("kit failed to load: {message}"),
            _ => {}
        }
        assert!(Instant::now() < deadline, "load never settled");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn render(plugin: &mut ResonanceDrums, events: &[NoteEvent]) -> Vec<(Vec<f32>, Vec<f32>)> {
    let mut buffers: Vec<(Vec<f32>, Vec<f32>)> = (0..NUM_OUTPUT_PORTS)
        .map(|_| (vec![0.0; BLOCK], vec![0.0; BLOCK]))
        .collect();
    {
        let mut ports: Vec<OutputBuffer<'_>> = buffers
            .iter_mut()
            .map(|(l, r)| OutputBuffer {
                left: l.as_mut_slice(),
                right: r.as_mut_slice(),
            })
            .collect();
        let mut iter = EventIterator::new(events);
        plugin.process(&mut ports, BLOCK, &mut iter, None);
    }
    buffers
}

/// Peak over every port of a block that strikes `note`, after letting
/// a kit swap land and silencing whatever still sounds.
fn strike_peak(plugin: &mut ResonanceDrums, note: u8) -> f32 {
    // A block for the audio thread to take a kit waiting in the mailbox,
    // and a few for the swap fade to finish.
    for _ in 0..8 {
        render(plugin, &[]);
    }
    plugin.reset();
    let hit = [NoteEvent::NoteOn {
        note,
        velocity: 1.0,
        timing: 0,
    }];
    render(plugin, &hit)
        .iter()
        .flat_map(|(l, r)| l.iter().chain(r))
        .fold(0.0f32, |m, s| m.max(s.abs()))
}

// ---------------------------------------------------------------------------
// Names and labels
// ---------------------------------------------------------------------------

#[test]
fn it_techno_names_its_pads_from_meta() {
    let kit = loaded("it_techno");
    let pads = &kit.kit_pads;
    assert!(pads.from_kit);
    assert_eq!(pads.pads[COUNT_STICK_PAD].name, "Perc Conga");
    assert_eq!(pads.pads[CLAP_PAD].name, "Clap");
    assert_eq!(pads.pads[KICK_PAD].name, "Kick");
    // The pads the sampler plays carry the same names.
    assert_eq!(kit.pads[COUNT_STICK_PAD].name, "Perc Conga");
    assert_eq!(kit.pads[CLAP_PAD].name, "Clap");
    assert_eq!(
        pads.pads[COUNT_STICK_PAD].piece.as_deref(),
        Some("SD Count Stick")
    );
}

#[test]
fn it_techno_labels_its_articulations_from_meta() {
    let kit = loaded("it_techno");
    let kick = kit.kit_pads.pads[KICK_PAD]
        .articulation
        .as_ref()
        .expect("the kick pairs");
    assert_eq!(kick.label, "punch/deep");
    assert_eq!(kick.primary_label, "punch");
    assert_eq!(kick.alt_label, "deep");
    assert_eq!(kick.alt, "SD Kick ohne Teppich");
    let snare = kit.kit_pads.pads[SNARE_PAD]
        .articulation
        .as_ref()
        .expect("the snare pairs");
    assert_eq!((snare.primary_label.as_str(), snare.alt_label.as_str()), ("snap", "body"));
    // The kit lists only these two pairs.
    for (slot, pad) in kit.kit_pads.pads.iter().enumerate() {
        if slot != KICK_PAD && slot != SNARE_PAD {
            assert!(pad.articulation.is_none(), "pad {slot} has an articulation");
        }
    }
}

#[test]
fn the_articulation_param_reads_the_kits_labels() {
    let plugin = booted();
    let params = plugin.bridge.params.clone();
    let kick = &params.pads[KICK_PAD].articulation;
    assert_eq!(kick.display(ARTICULATION_ALT as f64), "Alternate", "no kit yet");

    load(&plugin, "it_techno");
    assert_eq!(kick.display(ARTICULATION_PRIMARY as f64), "punch");
    assert_eq!(kick.display(ARTICULATION_ALT as f64), "deep");
    assert_eq!(kick.parse("Deep"), Some(ARTICULATION_ALT as f64));
    assert_eq!(kick.parse("1"), Some(ARTICULATION_ALT as f64));
    // The generic words still parse, so automation typed against them
    // keeps working.
    assert_eq!(kick.parse("Alternate"), Some(ARTICULATION_ALT as f64));
    // The CLAP bridge's text source while the plugin is active says the
    // same.
    let text = plugin.param_text_source().expect("a text source");
    let index = (0..plugin.param_count())
        .find(|&i| plugin.param(i).id() == "pad_0_articulation")
        .expect("pad_0_articulation");
    assert_eq!(text.display(index, 1.0).as_deref(), Some("deep"));

    // A pad the kit does not pair reads generically.
    assert_eq!(
        params.pads[TOM_HIGH_PAD].articulation.display(0.0),
        "Primary"
    );

    // Back on the built-in kit, the kit's words go.
    resonance_drums::selection::play_builtin(&plugin.bridge);
    assert_eq!(kick.display(ARTICULATION_ALT as f64), "Alternate");
}

#[test]
fn the_kits_alternate_piece_is_what_the_alt_value_plays() {
    let mut plugin = booted();
    load(&plugin, "it_techno");
    let primary = strike_peak(&mut plugin, drum_map::KICK);
    plugin.bridge.params.pads[KICK_PAD]
        .articulation
        .set_value(ARTICULATION_ALT);
    assert!(articulation::apply_pending(&plugin.bridge));
    settle(&plugin);
    let alt = strike_peak(&mut plugin, drum_map::KICK);
    // kick.wav is 0.50, kick_alt.wav 0.25.
    assert!(primary > 0.01, "the kick is silent");
    assert!(
        (primary / alt - 2.0).abs() < 0.01,
        "the alt kick should play at half the level: primary {primary}, alt {alt}"
    );
}

#[test]
fn chip_labels_split_at_the_slash() {
    let split = |l: &str| pad_map::split_label(l);
    assert_eq!(split("punch/deep"), ("punch".into(), "deep".into()));
    assert_eq!(split("snap/body"), ("snap".into(), "body".into()));
    assert_eq!(
        split("mit/ohne Teppich"),
        ("mit Teppich".into(), "ohne Teppich".into())
    );
    assert_eq!(split("open / closed"), ("open".into(), "closed".into()));
    assert_eq!(split("rimshot"), ("rimshot".into(), "rimshot (alt)".into()));
    assert_eq!(split(""), ("Primary".into(), "Alternate".into()));
}

// ---------------------------------------------------------------------------
// Pads the kit lacks (D7)
// ---------------------------------------------------------------------------

#[test]
fn a_kit_without_toms_leaves_the_tom_pads_absent_and_silent() {
    let kit = loaded("it_techno");
    for pad in TOM_PADS {
        assert!(!kit.kit_pads.pads[pad].present, "pad {pad} is present");
        assert_eq!(kit.kit_pads.pads[pad].name, PAD_MAPPINGS[pad].name);
        assert!(
            kit.pads[pad].close_mics.is_empty() && kit.pads[pad].overhead.is_none(),
            "pad {pad} has banks: the built-in sample must not fill it"
        );
    }

    let mut plugin = booted();
    // The built-in kit plays every pad, toms included.
    for note in TOM_NOTES {
        assert!(strike_peak(&mut plugin, note) > 0.01, "built-in tom {note} silent");
    }
    load(&plugin, "it_techno");
    assert!(strike_peak(&mut plugin, drum_map::KICK) > 0.01, "kick silent");
    for note in TOM_NOTES {
        assert_eq!(
            strike_peak(&mut plugin, note),
            0.0,
            "tom {note} must be exactly silent in a kit without toms"
        );
    }
}

#[test]
fn the_bridge_reports_the_loaded_kits_pads_and_the_built_in_ones_without_a_kit() {
    let plugin = booted();
    let builtin = plugin.bridge.kit_pads.current();
    assert!(!builtin.from_kit);
    assert!((0..NUM_PADS).all(|pad| builtin.is_present(pad)));

    load(&plugin, "it_techno");
    let pads = plugin.bridge.kit_pads.current();
    assert!(pads.from_kit);
    assert_eq!(pads.pads[COUNT_STICK_PAD].name, "Perc Conga");
    assert!(!pads.is_present(TOM_HIGH_PAD));

    resonance_drums::selection::play_builtin(&plugin.bridge);
    assert!(!plugin.bridge.kit_pads.current().from_kit);
}

// ---------------------------------------------------------------------------
// `_meta.pads` and the Drummica table
// ---------------------------------------------------------------------------

#[test]
fn meta_pads_override_the_drummica_table() {
    let kit = loaded("overrides");
    let pads = &kit.kit_pads;
    // The conga moved to the Tom High note, and only there.
    assert_eq!(pads.pads[TOM_HIGH_PAD].piece.as_deref(), Some("SD Count Stick"));
    assert_eq!(pads.pads[TOM_HIGH_PAD].name, "Perc Conga");
    assert!(!pads.pads[COUNT_STICK_PAD].present);
    // The kit's hints, as data for whoever applies defaults on a load…
    assert_eq!(pads.pads[TOM_HIGH_PAD].port, Some(3));
    assert_eq!(pads.pads[TOM_HIGH_PAD].choke, Some(3));
    assert_eq!(pads.pads[HAT_PAD].port, Some(0));
    assert_eq!(pads.pads[HAT_PAD].choke, None);
    // …and the pads are built with them.
    assert_eq!(kit.pads[TOM_HIGH_PAD].output_group, OutputGroup::Toms);
    assert_eq!(kit.pads[TOM_HIGH_PAD].choke_group, Some(3));
    assert_eq!(kit.pads[HAT_PAD].output_group, OutputGroup::Main);
    assert_eq!(kit.pads[HAT_PAD].choke_group, PAD_MAPPINGS[HAT_PAD].choke_group);
    assert!(kit.pads[TOM_HIGH_PAD].overhead.is_some(), "the conga's OH take");

    let mut plugin = booted();
    load(&plugin, "overrides");
    assert!(
        strike_peak(&mut plugin, drum_map::TOM_HIGH) > 0.01,
        "the Tom High note plays the conga"
    );
    assert_eq!(strike_peak(&mut plugin, drum_map::COUNT_STICK), 0.0);
}

#[test]
fn a_kit_without_meta_maps_through_the_drummica_table() {
    let kit = loaded("drummica_like");
    let pads = &kit.kit_pads;
    assert_eq!(pads.pads[KICK_PAD].piece.as_deref(), Some("SD Kick mit Teppich"));
    assert_eq!(pads.pads[TOM_HIGH_PAD].piece.as_deref(), Some("SD Tom01 mit Teppich"));
    // No `_meta.pieces`: the GM pad names.
    assert_eq!(pads.pads[KICK_PAD].name, "Kick");
    assert_eq!(pads.pads[TOM_HIGH_PAD].name, "Tom High");
    // The table's pairs, where the kit has the alternate piece.
    let tom = pads.pads[TOM_HIGH_PAD].articulation.as_ref().expect("tom pairs");
    assert_eq!(tom.alt, "SD Tom01 ohne Teppich");
    assert_eq!(
        (tom.primary_label.as_str(), tom.alt_label.as_str()),
        ("mit Teppich", "ohne Teppich")
    );
    assert!(pads.pads[KICK_PAD].articulation.is_some());
    assert!(
        pads.pads[SNARE_PAD].articulation.is_none(),
        "the kit has no ohne-Teppich snare"
    );
    assert!(!pads.pads[10].present && !pads.pads[11].present);

    let mut plugin = booted();
    load(&plugin, "drummica_like");
    assert!(strike_peak(&mut plugin, drum_map::TOM_HIGH) > 0.01);
    assert_eq!(strike_peak(&mut plugin, drum_map::TOM_MID), 0.0);
}

#[test]
fn a_piece_without_a_meta_name_drops_the_sd_prefix() {
    let meta: KitMeta = serde_json::from_str::<serde_json::Value>(
        r#"{"pads": {"SD Cowbell": {"note": 31}}}"#,
    )
    .map(|v| KitMeta::from_value(&v))
    .unwrap();
    let pads = KitPads::resolve(|piece| piece == "SD Cowbell", &meta);
    assert_eq!(pads.pads[COUNT_STICK_PAD].name, "Cowbell");
    assert!(pads.is_present(COUNT_STICK_PAD));
    assert_eq!((0..NUM_PADS).filter(|&p| pads.is_present(p)).count(), 1);
}

// ---------------------------------------------------------------------------
// Editor
// ---------------------------------------------------------------------------

#[cfg(feature = "editor")]
#[test]
fn the_editor_shows_the_kits_names_labels_and_absent_pads() {
    use resonance_drums::TestEditor;

    let plugin = booted();
    load(&plugin, "it_techno");
    let mut editor = TestEditor::new(&plugin, resonance_drums::library::shared(), (960.0, 640.0));
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    let strings = frame.strings();
    for name in ["Perc Conga", "Clap"] {
        assert!(strings.iter().any(|s| s == name), "{name:?} is not drawn");
    }
    assert!(
        !strings.iter().any(|s| s == "Count Stick" || s == "Snare Handtuch"),
        "the GM name of a named piece is drawn"
    );
    // The kit's names are what the filter matches, and a match is on screen.
    editor.filter_pads("conga");
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert!(frame.shows("Perc Conga"), "the filtered row is not visible");
    editor.filter_pads("");

    // Tom pads are dimmed, the kick is not.
    let frame = editor.frame(Vec::new());
    for pad in TOM_PADS {
        assert!(
            frame.widget(&format!("pad_row.{pad}.absent")).is_some(),
            "tom pad {pad} is not dimmed"
        );
    }
    assert!(frame.widget("pad_row.0.absent").is_none(), "the kick is dimmed");

    // The kick's inspector: the kit's articulation labels.
    editor.select_pad(KICK_PAD);
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    for label in ["punch", "deep", "punch/deep"] {
        assert!(frame.shows(label), "{label:?} is not visible");
    }
    assert!(frame.widget("articulation.1").is_some());
    assert!(frame.widget("inspector.not_in_kit").is_none());

    // A tom's inspector: not in this kit, no articulation chips.
    editor.select_pad(TOM_HIGH_PAD);
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    assert!(frame.shows("Not in this kit"), "the absent pad is not explained");
    assert!(frame.widget("inspector.not_in_kit").is_some());
    assert!(frame.widget("articulation.0").is_none());
}
