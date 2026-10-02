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

/// Wait for the built-in kit's pads to be published: `play_builtin` on an
/// active plugin builds the built-in kit off-thread and publishes its pads
/// with the hand-off.
fn wait_for_builtin_pads(plugin: &ResonanceDrums) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while plugin.bridge.kit_pads.current().from_kit {
        assert!(Instant::now() < deadline, "the built-in pads never came back");
        std::thread::sleep(Duration::from_millis(2));
    }
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
    // The built-in kit pairs nothing: the parameter says so.
    assert_eq!(
        kick.display(ARTICULATION_ALT as f64),
        pad_map::NO_ALTERNATE_TEXT,
        "no kit yet"
    );

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

    // A pad the kit does not pair says it has no alternate, on both
    // values — and the text still parses back.
    let tom = &params.pads[TOM_HIGH_PAD].articulation;
    for value in [0.0, 1.0] {
        assert_eq!(tom.display(value), pad_map::NO_ALTERNATE_TEXT);
    }
    assert_eq!(tom.parse(pad_map::NO_ALTERNATE_TEXT), Some(0.0));
    assert_eq!(tom.parse("Alternate"), Some(1.0));

    // Back on the built-in kit, the kit's words go.
    resonance_drums::selection::play_builtin(&plugin.bridge);
    wait_for_builtin_pads(&plugin);
    assert_eq!(kick.display(ARTICULATION_ALT as f64), pad_map::NO_ALTERNATE_TEXT);
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
    wait_for_builtin_pads(&plugin);
}

/// The pads are published with the hand-off, not looked up by `kit_path`:
/// a state load points `kit_path` at kit B before B decodes, and the
/// editor must keep showing the kit that plays — and keep showing it when
/// B fails.
#[test]
fn the_pads_stay_the_playing_kits_while_another_loads_and_when_it_fails() {
    let plugin = booted();
    load(&plugin, "it_techno");
    let bridge = &plugin.bridge;
    let missing = fixture("it_techno").with_file_name("no_such_manifest.json");
    // What a preset switch does first.
    *bridge.kit_path.lock() = Some(missing.clone());
    let pads = bridge.kit_pads.current();
    assert!(pads.from_kit, "the editor flipped to the built-in view");
    assert_eq!(pads.pads[COUNT_STICK_PAD].name, "Perc Conga");

    spawn_loader(
        missing,
        RATE,
        bridge,
        DEFAULT_OVERHEAD_SETUP.to_string(),
        no_choices(),
        bridge.articulations(),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while !matches!(*bridge.kit_status.lock(), KitStatus::Error { .. })
        || bridge.pending_kit.lock().is_some()
    {
        assert!(Instant::now() < deadline, "the load never failed");
        std::thread::sleep(Duration::from_millis(2));
    }
    let pads = bridge.kit_pads.current();
    assert!(pads.from_kit, "a failed load left the built-in view");
    assert_eq!(pads.pads[COUNT_STICK_PAD].name, "Perc Conga");
}

/// A load whose articulation labels differ from the kit before it asks the
/// host for a **text** rescan (the values did not move); one whose labels
/// match does not.
#[test]
fn a_load_that_changes_the_articulation_labels_asks_for_a_text_rescan() {
    let plugin = booted();
    let asks = &plugin.bridge.host_asks;
    let texts = || asks.text_rescans.load(std::sync::atomic::Ordering::Relaxed);
    let before = texts();
    load(&plugin, "it_techno");
    let after_first = texts();
    assert!(after_first > before, "punch/deep came in without a text rescan");

    // The same kit again: the same labels.
    load(&plugin, "it_techno");
    assert_eq!(texts(), after_first, "an unchanged label set asked again");

    // Another kit pairs other pads under other words.
    load(&plugin, "drummica_like");
    assert!(texts() > after_first);
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
    // The kit's name is on screen in its cell (the grid shows all 30 pads
    // at once; there is no filter to find one with).
    assert!(frame.shows("Perc Conga"), "the conga's cell is not visible");

    // Tom pads are dimmed, the kick is not.
    for pad in TOM_PADS {
        assert!(
            frame.widget(&format!("pad_cell.{pad}.absent")).is_some(),
            "tom pad {pad} is not dimmed"
        );
    }
    assert!(frame.widget("pad_cell.0.absent").is_none(), "the kick is dimmed");

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

// ---------------------------------------------------------------------------
// `KitPads::resolve` edge cases (pure: no files)
// ---------------------------------------------------------------------------

fn meta(json: serde_json::Value) -> KitMeta {
    KitMeta::from_value(&json)
}

#[test]
fn a_piece_that_loses_a_note_collision_still_plays_on_its_table_slot() {
    // Both name the Tom High note; "SD Count Stick" sorts first and wins
    // it. The kick loses — and must still play on the kick pad, not
    // nowhere.
    let pieces = ["SD Count Stick", "SD Kick mit Teppich"];
    let meta = meta(serde_json::json!({
        "pads": {
            "SD Count Stick": { "note": drum_map::TOM_HIGH },
            "SD Kick mit Teppich": { "note": drum_map::TOM_HIGH },
        }
    }));
    let pads = KitPads::resolve(|p| pieces.contains(&p), &meta);
    assert_eq!(pads.pads[TOM_HIGH_PAD].piece.as_deref(), Some("SD Count Stick"));
    assert_eq!(
        pads.pads[KICK_PAD].piece.as_deref(),
        Some("SD Kick mit Teppich"),
        "the losing piece plays nowhere"
    );
    // The winner plays only where it was placed.
    assert!(!pads.is_present(COUNT_STICK_PAD));
}

#[test]
fn a_drummica_piece_moved_by_meta_pads_keeps_its_alternate() {
    // A kit without `_meta.articulations` that moves Tom01 onto the Tom
    // Low note: the Drummica pair is the piece's, wherever it plays.
    let pieces = ["SD Tom01 mit Teppich", "SD Tom01 ohne Teppich"];
    let meta = meta(serde_json::json!({
        "pads": { "SD Tom01 mit Teppich": { "note": drum_map::TOM_LOW } }
    }));
    let pads = KitPads::resolve(|p| pieces.contains(&p), &meta);
    let tom_low = &pads.pads[TOM_PADS[2]];
    assert_eq!(tom_low.piece.as_deref(), Some("SD Tom01 mit Teppich"));
    let articulation = tom_low.articulation.as_ref().expect("Tom01's alternate");
    assert_eq!(articulation.alt, "SD Tom01 ohne Teppich");
    assert_eq!(pads.piece_for(TOM_PADS[2], true), Some("SD Tom01 ohne Teppich"));
    assert!(!pads.is_present(TOM_HIGH_PAD));
}

// ---------------------------------------------------------------------------
// The watcher: masked articulations, one load per tick, `acting()`
// ---------------------------------------------------------------------------

fn generation(plugin: &ResonanceDrums) -> u64 {
    plugin
        .bridge
        .load_generation
        .load(std::sync::atomic::Ordering::Acquire)
}

/// Load `kit` and let the audio thread take it, so the progress is
/// complete and the mailbox empty.
fn load_and_take(plugin: &mut ResonanceDrums, kit: &str) {
    load(plugin, kit);
    for _ in 0..4 {
        render(plugin, &[]);
    }
    assert!(plugin.bridge.load_progress.is_complete());
}

/// Moving the articulation of a pad the kit has no alternate for loads
/// nothing: a reload would fade every voice, restart the round robins and
/// drop the progress to 0 to build the same kit.
#[test]
fn an_unpaired_pads_articulation_reloads_nothing() {
    let mut plugin = booted();
    load_and_take(&mut plugin, "drummica_like");
    assert!(plugin.bridge.kit_pads.current().pads[SNARE_PAD].articulation.is_none());
    let before = generation(&plugin);

    plugin.bridge.params.pads[SNARE_PAD]
        .articulation
        .set_value(ARTICULATION_ALT);
    assert!(!articulation::apply_pending(&plugin.bridge));
    resonance_drums::selection::watch(&plugin.bridge);
    assert_eq!(generation(&plugin), before, "a load started");
    assert!(plugin.bridge.load_progress.is_complete());

    // A paired pad still reloads.
    plugin.bridge.params.pads[KICK_PAD]
        .articulation
        .set_value(ARTICULATION_ALT);
    assert!(articulation::apply_pending(&plugin.bridge));
    settle(&plugin);
}

/// A reload that rebuilds no pad of the kit the sampler holds hands
/// nothing off: the load is complete at once, with no block run.
#[test]
fn a_reload_that_rebuilds_nothing_hands_nothing_off() {
    let mut plugin = booted();
    load_and_take(&mut plugin, "drummica_like");
    let taken = plugin.bridge.load_progress.kits_taken();
    assert!(resonance_drums::reload::reload_kit(&plugin.bridge));
    settle(&plugin);
    assert_eq!(plugin.bridge.load_stats.lock().rebuilt_pads, 0);
    assert!(
        plugin.bridge.load_progress.is_complete(),
        "the reload waits for a kit the audio thread is never sent"
    );
    for _ in 0..4 {
        render(&mut plugin, &[]);
    }
    assert_eq!(plugin.bridge.load_progress.kits_taken(), taken, "a kit was swapped in");
}

/// One watcher tick starts at most one load: a moved preload and a moved
/// articulation are one reload, and a moved `kit_select` takes both with
/// it.
#[test]
fn one_watcher_tick_starts_one_load() {
    let mut plugin = booted();
    load_and_take(&mut plugin, "drummica_like");
    let before = generation(&plugin);
    {
        // Held so the instance's own watcher thread cannot act between
        // the writes.
        let _acting = plugin.bridge.params.selection.acting();
        plugin.bridge.params.stream_preload.set_value(0);
        plugin.bridge.params.pads[KICK_PAD]
            .articulation
            .set_value(ARTICULATION_ALT);
    }
    resonance_drums::selection::watch(&plugin.bridge);
    settle(&plugin);
    assert_eq!(generation(&plugin), before + 1, "one tick, one load");
    assert_eq!(
        plugin
            .bridge
            .stream_preload
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert_eq!(*plugin.bridge.loaded_articulations.lock(), plugin.bridge.articulations());

    // A `kit_select` that loads a kit, with the preload and an
    // articulation moved in the same tick: its load is the only one, and
    // it is built with both.
    use resonance_drums::selection::{self, NO_KIT, PARKED_KIT};
    selection::load_unslotted_now(&plugin.bridge, fixture("it_techno"));
    settle(&plugin);
    plugin.bridge.params.kit_select.set_value(NO_KIT);
    selection::watch(&plugin.bridge);
    wait_for_builtin_pads(&plugin);
    let before = generation(&plugin);
    {
        let _acting = plugin.bridge.params.selection.acting();
        plugin.bridge.params.stream_preload.set_value(1);
        plugin.bridge.params.pads[KICK_PAD]
            .articulation
            .set_value(ARTICULATION_PRIMARY);
        plugin.bridge.params.kit_select.set_value(PARKED_KIT);
    }
    selection::watch(&plugin.bridge);
    settle(&plugin);
    assert_eq!(generation(&plugin), before + 1, "one tick, one load");
    let handed = plugin.bridge.handed_off.lock().clone().expect("it_techno handed off");
    assert_eq!(handed.request.path, fixture("it_techno"));
    assert_eq!(handed.request.preload, resonance_drums::stream::DEFAULT_PRELOAD);
    assert_eq!(handed.request.articulations, plugin.bridge.articulations());
}

/// The articulation step waits for whoever holds `acting()` — a state
/// load, a `kit_select` act — rather than reload in the middle of it.
#[test]
fn an_articulation_reload_waits_for_acting() {
    let mut plugin = booted();
    load_and_take(&mut plugin, "drummica_like");
    let before = generation(&plugin);
    let bridge = plugin.bridge.clone();
    let acting = plugin.bridge.params.selection.acting();
    plugin.bridge.params.pads[KICK_PAD]
        .articulation
        .set_value(ARTICULATION_ALT);
    let worker = std::thread::spawn(move || articulation::apply_pending(&bridge));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(generation(&plugin), before, "reloaded under another act");
    drop(acting);
    // Either this call or the instance's watcher started it.
    let _ = worker.join().unwrap();
    assert_eq!(generation(&plugin), before + 1);
    settle(&plugin);
}

/// A kit none of whose pieces lands on a pad (no `_meta.pads`, no Drummica
/// names) fails its load with a reason, rather than "loading" as 30 silent
/// pads.
#[test]
fn a_kit_with_no_mappable_piece_fails_to_load() {
    let dir = std::env::temp_dir().join(format!("drums-unmappable-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let manifest = dir.join("drum_samples.json");
    let wav = fixture("it_techno")
        .parent()
        .unwrap()
        .join("../wavs/kick.wav");
    std::fs::write(
        &manifest,
        serde_json::json!({
            "Mystery Drum": { "01_KickIn_T": {
                "brand": "Test", "channel": "1", "mic": "M1", "position": "KickIn",
                "rounds": { "RR01": { "Vel01": wav } }
            } }
        })
        .to_string(),
    )
    .unwrap();
    let err = match load_kit_from_manifest(
        &manifest,
        RATE,
        DEFAULT_OVERHEAD_SETUP,
        &no_choices(),
        &[false; NUM_PADS],
    ) {
        Ok(_) => panic!("a kit with no mappable piece loaded"),
        Err(e) => e,
    };
    assert_eq!(err, resonance_drums::kit_loader::NO_MAPPABLE_PADS);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A chip click writes the pad's articulation param (announced to the host
/// as one undoable edit) and the watcher reloads the kit from it.
#[test]
fn an_articulation_chip_click_moves_the_param_and_reloads() {
    use resonance_drums::TestEditor;

    let plugin = booted();
    load(&plugin, "it_techno");
    let before = plugin
        .bridge
        .load_generation
        .load(std::sync::atomic::Ordering::Acquire);
    let mut editor = TestEditor::new(&plugin, resonance_drums::library::shared(), (960.0, 640.0));
    editor.select_pad(KICK_PAD);
    editor.frame(Vec::new());
    let frame = editor.frame(Vec::new());
    let chip = frame.widget("articulation.1").expect("the deep chip").rect;
    editor.click(chip.center());
    assert_eq!(
        plugin.bridge.params.pads[KICK_PAD].articulation.value(),
        ARTICULATION_ALT
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while plugin
        .bridge
        .load_generation
        .load(std::sync::atomic::Ordering::Acquire)
        == before
    {
        assert!(Instant::now() < deadline, "the click reloaded nothing");
        std::thread::sleep(Duration::from_millis(2));
    }
    settle(&plugin);
}
