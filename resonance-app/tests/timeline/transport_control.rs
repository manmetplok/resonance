//! Transport and playhead control commands (command-palette.md §5.1, §5.2,
//! §6, §10 P1): the Play / Stop toggle, every `SeekTarget` on a project
//! with a 4/4 → 7/8 meter change, the loop-point setter's crossing rules
//! and undo, and the new keys end to end.

use resonance_app::commands::{KeyChord, Mods, NamedKey};
use resonance_app::compose::{GenerateParams, SectionDefinitionState};
use resonance_app::message::{GlobalTrackMessage, Message, TransportMessage, UiMessage};
use resonance_app::state::{ArrangementMarker, ViewMode};
use resonance_app::update::shortcuts::TypingProbe;
use resonance_app::update::transport_nav::{LoopEdge, SeekTarget};
use resonance_app::Resonance;
use resonance_audio::types::AudioCommand;
use resonance_music_theory::MotifSource;

use crate::common::app_with_tempo_120;

/// Bars 0-1 are 4/4, bar 2 onwards 7/8 (0-based bars), at 120 BPM / 48 kHz.
fn meter_change_app() -> Resonance {
    let mut app = app_with_tempo_120();
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::AddSignatureEvent {
        bar: 2,
        numerator: 7,
        denominator: 8,
    }));
    assert_eq!(app.test_tempo_map().numerator_at_bar(2), 7);
    app
}

fn bar(app: &Resonance, n: u32) -> u64 {
    app.test_tempo_map().bar_to_sample(n)
}

fn beat(app: &Resonance, bar: u32, beat: u32) -> u64 {
    app.test_tempo_map()
        .beat_sample_in_bar(bar as usize, beat, 48_000)
        .expect("beat inside the bar table")
}

fn seek(app: &mut Resonance, target: SeekTarget) -> u64 {
    let _ = app.update(Message::Transport(TransportMessage::SeekTo(target)));
    app.test_playhead()
}

fn place(app: &mut Resonance, sample: u64) {
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(sample)));
}

pub(crate) fn section_def(id: u64, length_bars: u32) -> SectionDefinitionState {
    SectionDefinitionState {
        id,
        name: format!("S{id}"),
        color: [0, 0, 0],
        length_bars,
        chords: Vec::new(),
        scale: None,
        progression_seed: 0,
        generate_params: GenerateParams::default(),
        generator_spec: None,
        generator_seed: 0,
        generated_material: None,
        lane_generators: std::collections::HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: MotifSource::default(),
        arrangement: Vec::new(),
    }
}

fn press(app: &mut Resonance, chord: KeyChord, repeat: bool) {
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord,
        repeat,
        captured: false,
    }));
}

// ---------------------------------------------------------------------------
// Play / Stop
// ---------------------------------------------------------------------------

/// D1: Space stops and returns to where playback started.
#[test]
fn toggle_play_returns_the_playhead_to_where_playback_started() {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    place(&mut app, 12_345);
    while rx.try_recv().is_ok() {}

    let _ = app.update(Message::Transport(TransportMessage::TogglePlay));
    assert!(app.test_transport_playing());
    // Playback moves the playhead (the engine echo, stood in for here).
    place(&mut app, 90_000);
    let _ = app.update(Message::Transport(TransportMessage::TogglePlay));

    assert!(!app.test_transport_playing());
    assert_eq!(app.test_playhead(), 12_345);
    let sent: Vec<String> = rx.try_iter().map(|c| format!("{c:?}")).collect();
    let play = sent.iter().position(|c| c == &format!("{:?}", AudioCommand::Play));
    let stop = sent.iter().position(|c| c == &format!("{:?}", AudioCommand::Stop));
    let back = sent.iter().position(|c| c == &format!("{:?}", AudioCommand::SeekTo(12_345)));
    assert!(
        matches!((play, stop, back), (Some(p), Some(s), Some(b)) if p < s && s < b),
        "expected Play, Stop, SeekTo(12345) in order: {sent:?}"
    );
}

#[test]
fn play_from_loop_start_seeks_then_plays() {
    let mut app = app_with_tempo_120();
    let _ = app.update(Message::Transport(TransportMessage::SetLoopRange {
        loop_in: 48_000,
        loop_out: 96_000,
        enabled: None,
    }));
    place(&mut app, 200_000);
    let _ = app.update(Message::Transport(TransportMessage::PlayFromLoopStart));
    assert!(app.test_transport_playing());
    assert_eq!(app.test_playhead(), 48_000);
}

// ---------------------------------------------------------------------------
// Seek targets
// ---------------------------------------------------------------------------

#[test]
fn bar_nudges_land_on_meter_aware_bar_lines() {
    let mut app = meter_change_app();
    // Off-grid inside bar 1: the first press snaps to the line ahead.
    { let s = bar(&app, 1) + 100; place(&mut app, s); }
    assert_eq!(seek(&mut app, SeekTarget::NudgeBars(1)), bar(&app, 2));
    // Into and across the 7/8 bars.
    assert_eq!(seek(&mut app, SeekTarget::NudgeBars(1)), bar(&app, 3));
    assert_eq!(seek(&mut app, SeekTarget::NudgeBars(2)), bar(&app, 5));
    assert_eq!(seek(&mut app, SeekTarget::NudgeBars(-3)), bar(&app, 2));
    // Off-grid going back snaps to the line behind.
    { let s = bar(&app, 3) + 100; place(&mut app, s); }
    assert_eq!(seek(&mut app, SeekTarget::NudgeBars(-1)), bar(&app, 3));
    // Clamped at zero.
    place(&mut app, 0);
    assert_eq!(seek(&mut app, SeekTarget::NudgeBars(-1)), 0);
    // The 7/8 bars really are shorter.
    assert!(bar(&app, 3) - bar(&app, 2) < bar(&app, 2) - bar(&app, 1));
}

#[test]
fn beat_nudges_use_the_beat_unit_in_force() {
    let mut app = meter_change_app();
    { let s = bar(&app, 2); place(&mut app, s); }
    // Seven eighth-note beats in a 7/8 bar.
    for b in 1..7 {
        assert_eq!(seek(&mut app, SeekTarget::NudgeBeats(1)), beat(&app, 2, b));
    }
    assert_eq!(seek(&mut app, SeekTarget::NudgeBeats(1)), bar(&app, 3));
    // Back across the meter change: the last quarter-note beat of 4/4 bar 1.
    { let s = bar(&app, 2); place(&mut app, s); }
    assert_eq!(seek(&mut app, SeekTarget::NudgeBeats(-1)), beat(&app, 1, 3));
    // An eighth in 7/8 is half a quarter in 4/4.
    let eighth = beat(&app, 2, 1) - beat(&app, 2, 0);
    let quarter = beat(&app, 1, 1) - beat(&app, 1, 0);
    assert!(eighth.abs_diff(quarter / 2) <= 1);
}

#[test]
fn project_loop_section_and_bar_targets_resolve() {
    let mut app = meter_change_app();
    let _ = app.update(Message::Transport(TransportMessage::SetLoopRange {
        loop_in: bar(&app, 1),
        loop_out: bar(&app, 3),
        enabled: None,
    }));
    app.test_add_marker(ArrangementMarker::new_point(1, "Out".into(), [0; 3], bar(&app, 9)));
    app.test_push_section_definition(section_def(1, 4));
    app.test_place_section(1, 0);
    app.test_place_section(1, 4);

    place(&mut app, 777);
    assert_eq!(seek(&mut app, SeekTarget::ProjectStart), 0);
    assert_eq!(seek(&mut app, SeekTarget::ProjectEnd), bar(&app, 9));
    assert_eq!(seek(&mut app, SeekTarget::LoopStart), bar(&app, 1));
    assert_eq!(seek(&mut app, SeekTarget::LoopEnd), bar(&app, 3));

    { let s = bar(&app, 2); place(&mut app, s); }
    assert_eq!(seek(&mut app, SeekTarget::NextSection), bar(&app, 4));
    // No section starts after bar 4: the playhead stays put.
    assert_eq!(seek(&mut app, SeekTarget::NextSection), bar(&app, 4));
    assert_eq!(seek(&mut app, SeekTarget::PrevSection), 0);

    // 1-based bar / beat, as the palette's `:` mode types them.
    assert_eq!(seek(&mut app, SeekTarget::Bar { bar: 3, beat: 1 }), bar(&app, 2));
    assert_eq!(seek(&mut app, SeekTarget::Bar { bar: 3, beat: 4 }), beat(&app, 2, 3));
}

// ---------------------------------------------------------------------------
// Loop points
// ---------------------------------------------------------------------------

fn set_point(app: &mut Resonance, at: u64, edge: LoopEdge) -> (u64, u64) {
    place(app, at);
    let _ = app.update(Message::Transport(TransportMessage::SetLoopPoint { edge }));
    let (i, o, _) = app.test_loop_range();
    (i, o)
}

#[test]
fn loop_points_follow_the_playhead_with_the_crossing_rules() {
    let mut app = app_with_tempo_120();
    let b = |n| app_with_tempo_120().test_tempo_map().bar_to_sample(n);
    let _ = app.update(Message::Transport(TransportMessage::SetLoopRange {
        loop_in: b(2),
        loop_out: b(4),
        enabled: Some(false),
    }));

    assert_eq!(set_point(&mut app, b(3), LoopEdge::Start), (b(3), b(4)));
    // in ≥ out: the end is pushed one bar past the new start.
    assert_eq!(set_point(&mut app, b(6), LoopEdge::Start), (b(6), b(7)));
    assert_eq!(set_point(&mut app, b(9), LoopEdge::End), (b(6), b(9)));
    // out ≤ in: the start is pushed one bar before the new end.
    assert_eq!(set_point(&mut app, b(2), LoopEdge::End), (b(1), b(2)));
    // …clamped at zero, where the end moves out instead.
    assert_eq!(set_point(&mut app, 0, LoopEdge::End), (0, b(1)));
    // Setting a point never enables the loop.
    assert!(!app.test_loop_range().2);
}

#[test]
fn each_loop_point_is_one_undo_entry() {
    let mut app = app_with_tempo_120();
    // Undo only records for a project with a path.
    app.test_set_project_path(std::env::temp_dir().join("resonance_transport_control.rproj"));
    let b = |n| app_with_tempo_120().test_tempo_map().bar_to_sample(n);
    let _ = app.update(Message::Transport(TransportMessage::SetLoopRange {
        loop_in: b(2),
        loop_out: b(4),
        enabled: None,
    }));
    let before = app.test_undo_history().undo_len();
    set_point(&mut app, b(6), LoopEdge::Start);
    assert_eq!(app.test_undo_history().undo_len(), before + 1);
    let _ = app.update(Message::Undo);
    let (i, o, _) = app.test_loop_range();
    assert_eq!((i, o), (b(2), b(4)));
}

// ---------------------------------------------------------------------------
// Keys
// ---------------------------------------------------------------------------

fn keyed_app() -> Resonance {
    let mut app = app_with_tempo_120();
    app.test_set_view_mode(ViewMode::Arrange);
    app.test_set_typing_probe(TypingProbe::Assume { editing: false });
    app
}

#[test]
fn space_plays_and_stops_but_not_while_typing() {
    let space = KeyChord::named(NamedKey::Space, Mods::NONE);
    let mut app = keyed_app();
    app.test_set_typing_probe(TypingProbe::Assume { editing: true });
    press(&mut app, space, false);
    assert!(!app.test_transport_playing(), "Space typed into a field must not play");

    app.test_set_typing_probe(TypingProbe::Assume { editing: false });
    press(&mut app, space, false);
    assert!(app.test_transport_playing());
    press(&mut app, space, false);
    assert!(!app.test_transport_playing());
}

#[test]
fn a_held_arrow_repeats_the_bar_nudge() {
    let right = KeyChord::named(NamedKey::ArrowRight, Mods::NONE);
    let mut app = keyed_app();
    press(&mut app, right, false);
    press(&mut app, right, true);
    press(&mut app, right, true);
    assert_eq!(app.test_playhead(), bar(&app, 3));
    // A held L is one toggle.
    let l = KeyChord::char('l', Mods::NONE);
    press(&mut app, l, false);
    press(&mut app, l, true);
    assert!(app.test_loop_range().2);
}

#[test]
fn brackets_and_home_move_the_playhead() {
    let mut app = keyed_app();
    let _ = app.update(Message::Transport(TransportMessage::SetLoopRange {
        loop_in: bar(&app, 2),
        loop_out: bar(&app, 5),
        enabled: None,
    }));
    press(&mut app, KeyChord::char('[', Mods::NONE), false);
    assert_eq!(app.test_playhead(), bar(&app, 2));
    press(&mut app, KeyChord::char(']', Mods::NONE), false);
    assert_eq!(app.test_playhead(), bar(&app, 5));
    press(&mut app, KeyChord::named(NamedKey::Home, Mods::NONE), false);
    assert_eq!(app.test_playhead(), 0);
}
