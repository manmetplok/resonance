//! Undo snapshot/restore is a fixed point (ARCH-01 A1-1).
//!
//! `build_project_file(restore(snapshot_for_undo(app))) == snapshot.file`,
//! and the whole snapshot (`ProjectFile` + MIDI notes + `UndoExtras`)
//! comes back `same_state`, through BOTH restore paths:
//!
//!   * the structure-preserving diff replay (`try_diff_replay`), reached
//!     when only scalars changed since the snapshot;
//!   * the full `ClearAll → AllCleared → replay_loaded_project →
//!     finalize_undo_restore` pipeline, forced here by adding a track
//!     after the snapshot.
//!
//! Every fixture (the demo project and each built-in template) is first
//! taken through one round of edits per domain — transport, mixer, tempo
//! events, markers, chord track, automation, clip fade/gain, freeze,
//! external instrument, reference trim, drum arrangement, vocal lyrics —
//! so the snapshot carries every field `UndoExtras` duplicates; then a
//! second round with different values is applied, and the restore must
//! undo it exactly. That is the guard for folding each extras field into
//! the `ProjectFile` (A1-2): a field that stops being restored shows up
//! here as a JSON diff or a `same_state` mismatch.
//!
//! Compare through serde JSON for diagnostics: a pretty-printed diff finds
//! the first differing line, which a bare struct `assert_eq!` on a
//! thousand-field `ProjectFile` would not. `ProjectFile` derives
//! `PartialEq` since ARCH-01 A-8 (`same_state` / `gesture_changed_since`
//! compare structs now, not JSON) — see
//! `struct_equality_agrees_with_json_equality_across_fixtures` below for the
//! guard that the two ways of comparing a file agree.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use resonance_app::chord_track::{ChordRegion, KeyChange};
use resonance_app::compose::messages::ArrangementMessage;
use resonance_app::compose::{ComposeMessage, EntryLength};
use resonance_app::demo;
use resonance_app::message::*;
use resonance_app::project::{LoadedProject, ProjectExternalInstrument, ProjectFile};
use resonance_app::reference::ReferenceMessage;
use resonance_app::state::FreezeStatus;
use resonance_app::undo::UndoSnapshot;
use resonance_app::update::project_io::BuiltinTemplateId;
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, ClipId, TrackId, TrackType};
use resonance_common::{AutomationTarget, CurveKind, FreezeCacheRef, FreezeCacheStatus};
use resonance_music_theory::{Chord, ChordQuality, Mode, PitchClass, Scale};

const CHORD_KEY_ID: u64 = 900_001;
const CHORD_REGION_ID: u64 = 900_002;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct Fixture {
    app: Resonance,
    rx: Receiver<AudioCommand>,
    /// The `.rproj` directory the project is anchored at; its sibling
    /// `.freeze/` directory holds the freeze cache the fixture creates.
    project: PathBuf,
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A capturing app with `load` applied and a saved project path, so
/// `can_record_undo` holds and the freeze cache has somewhere to live.
fn fixture(tag: &str, load: impl FnOnce(&mut Resonance, &Path)) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "resonance-undo-fixed-point-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("fixture.rproj");
    std::fs::create_dir_all(project.join("audio")).expect("create project dir");
    std::fs::create_dir_all(project.with_extension("freeze")).expect("create freeze dir");

    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    load(&mut app, &project);
    echo_midi_clip_loads(&mut app, &rx);
    app.test_set_active_project(true);
    app.test_set_project_path(project.clone());
    Fixture {
        app,
        rx,
        project,
        root,
    }
}

/// Play the engine's part for the MIDI clips the fixture loaded: every
/// `LoadMidiClipDirect` the seeding sent comes back as `MidiClipCreated`,
/// exactly as the live engine echoes it. The demo materialises its drum
/// clip through the engine and registers it in `compose.derived_clips`
/// before the echo lands; without the echo that map points at a clip the
/// mirror never receives. The mirror handler is idempotent, so clips a
/// replay already pushed are untouched.
fn echo_midi_clip_loads(app: &mut Resonance, rx: &Receiver<AudioCommand>) {
    for cmd in drain(rx) {
        if let AudioCommand::LoadMidiClipDirect {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes,
            name,
            trim_start_ticks,
            trim_end_ticks,
        } = cmd
        {
            app.test_apply_engine_event(AudioEvent::MidiClipCreated {
                clip_id,
                track_id,
                start_sample,
                duration_ticks,
                name,
                notes,
                trim_start_ticks,
                trim_end_ticks,
            });
        }
    }
}

fn load_demo(app: &mut Resonance, _project: &Path) {
    demo::seed_demo_content(app);
}

/// Instantiate a built-in template exactly as the picker does, minus the
/// `ClearAll` round-trip: straight into `replay_loaded_project`.
fn load_template(id: BuiltinTemplateId) -> impl FnOnce(&mut Resonance, &Path) {
    move |app, project| {
        let built = id.build();
        app.test_replay_loaded_project_from(LoadedProject {
            file: built.file,
            project_dir: project.to_path_buf(),
            midi_notes: built.midi_notes,
            plugin_states: HashMap::new(),
        });
    }
}

/// Entities the per-domain edits are aimed at, resolved from the loaded
/// project so the same edit script works for every fixture. Anything a
/// fixture lacks (an audio clip, a section, a vocal clip) is skipped.
struct Handles {
    tracks: Vec<TrackId>,
    audio_clip: Option<ClipId>,
    /// A MIDI clip on a vocal track, with its note count (lyrics are
    /// indexed per note).
    vocal_clip: Option<(ClipId, usize)>,
    definition: Option<u64>,
    pattern: Option<u64>,
    device: Option<String>,
}

fn handles(app: &Resonance) -> Handles {
    let file = app.test_build_project_file();
    let tracks: Vec<TrackId> = file.tracks.iter().map(|t| t.id).collect();
    let vocal_tracks: Vec<TrackId> = file
        .tracks
        .iter()
        .filter(|t| t.track_type == "vocal")
        .map(|t| t.id)
        .collect();
    let vocal_clip = app
        .test_midi_clips()
        .iter()
        .find(|mc| vocal_tracks.contains(&mc.track_id) && !mc.notes.is_empty())
        .map(|mc| (mc.id, mc.notes.len()));
    let compose = app.compose_state();
    Handles {
        tracks,
        audio_clip: file.clips.first().map(|c| c.id),
        vocal_clip,
        definition: compose.definitions.first().map(|d| d.id),
        pattern: compose.drum_patterns.first().map(|p| p.id),
        device: app.test_device_registry_ids().first().cloned(),
    }
}

/// One edit per domain. `variant` 0 establishes the state the snapshot
/// captures (creating what needs creating); `variant` 1 changes every
/// one of those values again without changing the project's *shape*, so
/// the diff replay stays eligible.
fn edit_every_domain(f: &mut Fixture, h: &Handles, variant: u8) {
    let app = &mut f.app;
    let v = variant as f32;
    let vu = variant as u64;

    // Transport + loop range.
    app.test_dispatch(Message::Transport(TransportMessage::SetBpmText(format!(
        "{}",
        100.0 + 10.0 * v
    ))));
    app.test_dispatch(Message::Transport(TransportMessage::CommitBpm));
    app.test_dispatch(Message::Transport(TransportMessage::SetLoopRange {
        loop_in: 0,
        loop_out: 48_000 * (vu + 1),
        enabled: Some(true),
    }));

    // Mixer scalars.
    if let Some(&t) = h.tracks.first() {
        app.test_dispatch(Message::Track(TrackMessage::SetTrackVolume(
            t,
            -3.0 - 3.0 * v,
        )));
    }
    app.test_dispatch(Message::Track(TrackMessage::SetMasterVolume(-1.0 - v)));

    // Tempo track.
    app.test_dispatch(Message::GlobalTrack(GlobalTrackMessage::AddTempoEvent {
        bar: 4 + 4 * variant as u32,
        bpm: 100.0 + 10.0 * v,
    }));

    // Arrangement markers. Adding one changes the marker id set, which is
    // structural for the diff replay, so the second round renames instead.
    if variant == 0 {
        app.test_dispatch(Message::Marker(MarkerMessage::AddAtPlayhead));
    } else if let Some(id) = app.test_markers().markers.first().map(|m| m.id) {
        app.test_dispatch(Message::Marker(MarkerMessage::Rename(id, "renamed".into())));
    }

    // Global chord track.
    if variant == 0 {
        let track = app.test_chord_track_mut();
        track.insert_key_change(KeyChange {
            id: CHORD_KEY_ID,
            start_sample: 0,
            scale: Scale::new(PitchClass::C, Mode::Major),
        });
        track.insert_region(ChordRegion {
            id: CHORD_REGION_ID,
            chord: Chord::new(PitchClass::C, ChordQuality::Maj),
            start_sample: 0,
            end_sample: 96_000,
            pinned: false,
        });
    } else {
        app.test_dispatch(Message::ChordTrack(ChordTrackMessage::SetSymbol {
            id: CHORD_REGION_ID,
            symbol: "Am".into(),
        }));
        app.test_dispatch(Message::ChordTrack(ChordTrackMessage::TogglePin {
            id: CHORD_REGION_ID,
        }));
    }

    // Automation: a breakpoint per round on the first track's gain lane.
    if let Some(&t) = h.tracks.first() {
        app.test_dispatch(Message::Automation(AutomationMessage::AddBreakpoint {
            target: AutomationTarget::TrackGain(t),
            time_frames: 48_000 * (vu + 1),
            value: 0.25 + 0.5 * v,
            curve: CurveKind::Linear,
        }));
    }

    // Clip fade / gain.
    if let Some(c) = h.audio_clip {
        app.test_dispatch(Message::Clip(ClipMessage::SetClipFadeInMs {
            clip_id: c,
            ms: 250.0 + 250.0 * v,
        }));
        app.test_dispatch(Message::Clip(ClipMessage::SetClipGainDb {
            clip_id: c,
            gain_db: -6.0 + 3.0 * v,
        }));
    }

    // Freeze: the second track is frozen with a real cache file in round
    // 0 and unfrozen in round 1, so the restore has to bring the cache
    // back as `Frozen` (the file still exists) rather than `Stale`.
    if let Some(&t) = h.tracks.get(1) {
        let cache_filename = format!("freeze_{t}.wav");
        if variant == 0 {
            std::fs::write(
                f.project.with_extension("freeze").join(&cache_filename),
                b"",
            )
            .expect("write freeze cache");
            app.test_set_freeze_status(
                t,
                FreezeStatus::Frozen {
                    cache_ref: FreezeCacheRef {
                        cache_filename,
                        sample_rate: 48_000,
                        bit_depth: 24,
                        render_fingerprint: 7,
                        status: FreezeCacheStatus::Frozen,
                    },
                },
            );
        } else {
            app.test_set_freeze_status(t, FreezeStatus::Idle);
        }
    }

    // External instrument on the last track, with a bundled device that
    // round 1 deselects, so the restore has to bring the device back.
    if let Some(&t) = h.tracks.last() {
        if variant == 0 {
            app.test_dispatch(Message::ExternalInstrument(
                ExternalInstrumentMessage::Enable(t),
            ));
            app.test_dispatch(Message::ExternalInstrument(
                ExternalInstrumentMessage::SetDevice(t, h.device.clone()),
            ));
        } else {
            app.test_dispatch(Message::ExternalInstrument(
                ExternalInstrumentMessage::SetDevice(t, None),
            ));
        }
        app.test_dispatch(Message::ExternalInstrument(
            ExternalInstrumentMessage::SetProgram(t, Some(5 + variant)),
        ));
    }
    // A second external track, with no device, that round 1 takes out of
    // external mode: the restore has to put it back.
    if let Some(t) = second_external(h) {
        let msg = if variant == 0 {
            ExternalInstrumentMessage::Enable(t)
        } else {
            ExternalInstrumentMessage::Disable(t)
        };
        app.test_dispatch(Message::ExternalInstrument(msg));
    }

    // Reference A/B trim.
    app.test_dispatch(Message::Reference(ReferenceMessage::TrimChanged(-3.0 + v)));

    // Drum arrangement on the first section.
    if let (Some(d), Some(p)) = (h.definition, h.pattern) {
        if variant == 0 {
            app.test_dispatch(Message::Compose(ComposeMessage::Arrangement(
                ArrangementMessage::AddEntry {
                    definition_id: d,
                    pattern_id: p,
                },
            )));
        } else {
            app.test_dispatch(Message::Compose(ComposeMessage::Arrangement(
                ArrangementMessage::SetEntryLength {
                    definition_id: d,
                    index: 0,
                    length: EntryLength::Bars(2),
                },
            )));
        }
    }

    // Vocal lyrics with a trailing empty entry and one short of the note
    // count: the serializer strips trailing empties and a disk load pads
    // back to the note count, so this is the shape on which the two
    // restore paths could disagree in live state (FU-H2c).
    if let Some((clip, n)) = h.vocal_clip {
        let syllable = if variant == 0 { "la" } else { "da" };
        app.test_set_clip_lyrics(clip, short_lyrics(syllable, n));
    }

    // Performance footer: tuning + capo live in the `ProjectFile`, so a
    // restore must bring them back on both paths (FU-H2c).
    app.test_dispatch(Message::Ui(UiMessage::SetPerformanceTuning(1 + variant as usize)));
    app.test_dispatch(Message::Ui(UiMessage::SetPerformanceCapo(2 + variant)));

    // The arrangement edit re-materialises the drum clip through the
    // engine; land its echo as the live engine would, so the snapshot
    // never captures a derived-clip entry whose mirror is still pending.
    echo_midi_clip_loads(app, &f.rx);
}

/// The track the second external instrument goes on: the first one, when
/// it is not also the last (which carries the first external instrument).
fn second_external(h: &Handles) -> Option<TrackId> {
    (h.tracks.len() >= 2).then(|| h.tracks[0])
}

/// A lyric vector shorter than the clip's `n` notes (for `n >= 3`) that
/// ends in an empty entry.
fn short_lyrics(syllable: &str, n: usize) -> Vec<String> {
    let mut lyrics = vec![syllable.to_string(); n.saturating_sub(2).max(1)];
    lyrics.push(String::new());
    lyrics
}

// ---------------------------------------------------------------------------
// Assertions
// ---------------------------------------------------------------------------

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn pretty(file: &ProjectFile) -> String {
    serde_json::to_string_pretty(file).expect("ProjectFile serializes")
}

/// `build_project_file` of the restored app must equal the snapshot's
/// file byte for byte. On a mismatch, report the first differing line
/// with some context from both sides rather than two 2,000-line dumps.
fn assert_file_fixed_point(path: &str, restored: &ProjectFile, snapshot: &ProjectFile) {
    let a = pretty(restored);
    let b = pretty(snapshot);
    if a == b {
        return;
    }
    let (al, bl): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let first = al
        .iter()
        .zip(bl.iter())
        .position(|(x, y)| x != y)
        .unwrap_or(al.len().min(bl.len()));
    let ctx = |lines: &[&str]| {
        lines
            .iter()
            .skip(first.saturating_sub(4))
            .take(10)
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    };
    panic!(
        "{path}: build_project_file(restore(snapshot)) != snapshot.file — first difference at line {}\n\
         --- restored:\n{}\n--- snapshot:\n{}",
        first + 1,
        ctx(&al),
        ctx(&bl)
    );
}

/// Which parts of two snapshots differ, by name — the diagnostic behind
/// a `same_state` failure once the project files already match.
fn snapshot_differences(a: &UndoSnapshot, b: &UndoSnapshot) -> Vec<String> {
    let (x, y) = (&a.extras, &b.extras);
    let mut out = Vec::new();
    let mut check = |name: &str, same: bool| {
        if !same {
            out.push(name.to_string());
        }
    };
    check(
        "midi_notes",
        a.project.midi_notes.len() == b.project.midi_notes.len()
            && a.project.midi_notes.iter().all(|(id, n)| {
                b.project.midi_notes.get(id).is_some_and(|o| {
                    resonance_app::update::project_io::replay_diff::midi_notes_equal(n, o)
                })
            }),
    );
    check(
        "compose_derived_clips",
        x.compose_derived_clips == y.compose_derived_clips,
    );
    check(
        "compose_next_derived_clip_id",
        x.compose_next_derived_clip_id == y.compose_next_derived_clip_id,
    );
    check(
        "vocal_clip_lyrics",
        x.vocal_clip_lyrics == y.vocal_clip_lyrics,
    );
    check("automation_lanes", x.automation_lanes == y.automation_lanes);
    check(
        "reference",
        x.reference.entries == y.reference.entries
            && x.reference.active_id == y.reference.active_id
            && x.reference.loudness_match == y.reference.loudness_match
            && x.reference.trim_db.to_bits() == y.reference.trim_db.to_bits(),
    );
    check("track_freeze", x.track_freeze == y.track_freeze);
    out
}

fn assert_fixed_point(f: &Fixture, path: &str, snapshot: &UndoSnapshot) {
    assert_file_fixed_point(
        path,
        &f.app.test_build_project_file(),
        &snapshot.project.file,
    );
    let after = f.app.test_snapshot_for_undo();
    assert!(
        Resonance::test_snapshot_same_state(&after, snapshot),
        "{path}: the project file matches but these parts of the snapshot do not: {:?}\n\
         restored extras: {:#?}\nsnapshot extras: {:#?}",
        snapshot_differences(&after, snapshot),
        after.extras,
        snapshot.extras
    );
}

/// The first round of edits must be visible in the snapshot, or a domain
/// whose dispatch was silently refused would make its leg of the test
/// vacuous (the silent-goldens rule, applied to state).
fn assert_seeded(snapshot: &UndoSnapshot, h: &Handles) {
    let file = &snapshot.project.file;
    let x = &snapshot.extras;
    assert!((file.bpm - 100.0).abs() < 1e-6, "bpm edit landed");
    assert!(
        file.loop_enabled && file.loop_out == 48_000,
        "loop range landed"
    );
    assert!(!file.tempo_events.is_empty(), "tempo event landed");
    assert!(!file.arrangement_markers.is_empty(), "marker landed");
    assert_eq!(file.performance.capo, 2, "performance capo landed");
    assert_eq!(file.chord_track.regions.len(), 1, "chord region landed");
    assert_eq!(file.chord_track.key_changes.len(), 1, "key change landed");
    assert!(
        (x.reference.trim_db + 3.0).abs() < 1e-6,
        "reference trim landed"
    );
    if let Some(&t) = h.tracks.first() {
        assert!(
            x.automation_lanes
                .contains_key(&AutomationTarget::TrackGain(t)),
            "automation lane landed"
        );
        let track = file
            .tracks
            .iter()
            .find(|pt| pt.id == t)
            .expect("first track");
        assert!((track.volume + 3.0).abs() < 1e-6, "track volume landed");
    }
    if let Some(&t) = h.tracks.get(1) {
        assert!(
            matches!(x.track_freeze.get(&t), Some(FreezeStatus::Frozen { .. })),
            "freeze landed"
        );
    }
    assert!(h.device.is_some(), "the registry has a bundled device");
    if let Some(&t) = h.tracks.last() {
        let ext = external_of(file, t).expect("external instrument landed");
        assert_eq!(ext.device_id, h.device, "device selection landed");
        assert_eq!(ext.program, Some(5), "program landed");
    }
    if let Some(t) = second_external(h) {
        assert!(
            external_of(file, t).is_some(),
            "second external instrument landed"
        );
    }
    if let Some(c) = h.audio_clip {
        let clip = file.clips.iter().find(|pc| pc.id == c).expect("audio clip");
        assert!(
            clip.fade_in_frames > 0 && clip.gain_db < 0.0,
            "fade/gain landed"
        );
    }
    if let Some(d) = h.definition {
        let def = file
            .section_definitions
            .iter()
            .find(|sd| sd.id == d)
            .expect("section definition");
        assert!(!def.arrangement.is_empty(), "arrangement entry landed");
    }
    if let Some((clip, n)) = h.vocal_clip {
        assert_eq!(
            x.vocal_clip_lyrics.get(&clip).map(Vec::len),
            Some(short_lyrics("", n).len()),
            "lyrics landed"
        );
    }
}

fn external_of(file: &ProjectFile, t: TrackId) -> Option<&ProjectExternalInstrument> {
    file.tracks
        .iter()
        .find(|pt| pt.id == t)
        .and_then(|pt| pt.external_instrument.as_ref())
}

/// Each restore path re-asserts the snapshot's external-instrument config
/// exactly once per external track (A1-2 (3): the slow path used to do it
/// twice, from the replay and again from `UndoExtras`), with the config
/// the snapshot's `ProjectFile` carries, and binds the selected device's
/// params once.
fn assert_external_restored(path: &str, cmds: &[AudioCommand], snapshot: &ProjectFile) {
    let mut externals = 0;
    for pt in &snapshot.tracks {
        let sets: Vec<_> = cmds
            .iter()
            .filter_map(|c| match c {
                AudioCommand::SetExternalInstrument { config } if config.track_id == pt.id => {
                    Some(*config)
                }
                _ => None,
            })
            .collect();
        let binds = cmds
            .iter()
            .filter(|c| {
                matches!(c, AudioCommand::SetTrackDeviceParams { track_id, params }
                    if *track_id == pt.id && !params.is_empty())
            })
            .count();
        let Some(ext) = &pt.external_instrument else {
            assert!(
                sets.is_empty(),
                "{path}: track {} is not external in the snapshot",
                pt.id
            );
            continue;
        };
        externals += 1;
        assert_eq!(
            sets.len(),
            1,
            "{path}: track {} must be re-asserted external exactly once",
            pt.id
        );
        let config = sets[0];
        assert_eq!(
            (config.bank, config.program, config.latency_offset_samples),
            (ext.bank, ext.program, ext.latency_offset_samples),
            "{path}: track {} restored with the snapshot's config",
            pt.id
        );
        assert_eq!(
            binds,
            usize::from(ext.device_id.is_some()),
            "{path}: track {} binds its device params once iff a device is selected",
            pt.id
        );
    }
    assert!(
        externals > 0 || snapshot.tracks.is_empty(),
        "{path}: the snapshot has an external track"
    );
}

/// Run both restore paths against `f` and assert the fixed point after each.
fn check_both_paths(mut f: Fixture) {
    let h = handles(&f.app);
    edit_every_domain(&mut f, &h, 0);
    let snapshot = f.app.test_snapshot_for_undo();
    assert_seeded(&snapshot, &h);

    // -- Fast path: scalar edits only, so the diff replay is eligible. --
    edit_every_domain(&mut f, &h, 1);
    let dirty = f.app.test_snapshot_for_undo();
    assert!(
        !Resonance::test_snapshot_same_state(&dirty, &snapshot),
        "the second round of edits must change the snapshot, or the restore is vacuous"
    );
    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    let cmds = drain(&f.rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "scalar-only edits must take the diff replay, not the full clear"
    );
    assert_external_restored("fast path", &cmds, &snapshot.project.file);
    assert_fixed_point(&f, "fast path (try_diff_replay)", &snapshot);

    // -- Slow path: an extra track forces the structural fallback. --
    edit_every_domain(&mut f, &h, 1);
    f.app.test_add_track(9_999, TrackType::Audio);
    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    let cmds = drain(&f.rx);
    assert!(
        cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "a structural change must fall back to the full clear-and-replay"
    );
    f.app.test_apply_engine_event(AudioEvent::AllCleared);
    let cmds = drain(&f.rx);
    assert_external_restored("slow path", &cmds, &snapshot.project.file);
    assert!(
        f.app.test_project_path() == Some(f.project.as_path()),
        "the undo replay must keep the project path"
    );
    assert_fixed_point(
        &f,
        "slow path (ClearAll → replay_loaded_project)",
        &snapshot,
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn demo_project_restores_to_a_fixed_point() {
    check_both_paths(fixture("demo", load_demo));
}

#[test]
fn vocal_songwriting_template_restores_to_a_fixed_point() {
    check_both_paths(fixture(
        "vocal-songwriting",
        load_template(BuiltinTemplateId::VocalSongwriting),
    ));
}

#[test]
fn band_recording_template_restores_to_a_fixed_point() {
    check_both_paths(fixture(
        "band-recording",
        load_template(BuiltinTemplateId::BandRecording),
    ));
}

#[test]
fn beatmaking_template_restores_to_a_fixed_point() {
    check_both_paths(fixture(
        "beatmaking",
        load_template(BuiltinTemplateId::Beatmaking),
    ));
}

#[test]
fn empty_template_restores_to_a_fixed_point() {
    check_both_paths(fixture("empty", load_template(BuiltinTemplateId::Empty)));
}

// ---------------------------------------------------------------------------
// Derived clips whose engine echo is still in flight (FU-H2a)
// ---------------------------------------------------------------------------

/// Derived-clip entries whose clip the mirror doesn't hold.
fn unmirrored_derived(app: &Resonance) -> Vec<ClipId> {
    app.compose_state()
        .derived_clips
        .values()
        .copied()
        .filter(|id| !app.test_midi_clips().iter().any(|mc| mc.id == *id))
        .collect()
}

/// A snapshot taken while a re-derived clip's `MidiClipCreated` echo is
/// still pending carries its `derived_clips` entry but not the clip. The
/// fast path used to rebuild the map from the mirror and drop the entry —
/// so the echo then landed an orphan the next regeneration duplicated —
/// while the slow path copied the snapshot's map verbatim, leaving an
/// entry for a clip `ClearAll` had wiped (which also blinds the UPD-05
/// freeze check on that track for good). Both paths now take the
/// snapshot's map, and the slow path drops only the entries whose clip
/// cannot arrive any more.
#[test]
fn derived_clips_with_a_pending_echo_survive_both_restore_paths() {
    let mut f = fixture("derived-pending", load_demo);
    let h = handles(&f.app);
    let (Some(d), Some(p)) = (h.definition, h.pattern) else {
        panic!("the demo has a section and a drum pattern");
    };
    let _ = drain(&f.rx);
    f.app
        .test_dispatch(Message::Compose(ComposeMessage::Arrangement(
            ArrangementMessage::AddEntry {
                definition_id: d,
                pattern_id: p,
            },
        )));
    // Keep the re-materialised drum clip's echo in flight.
    let pending_loads = drain(&f.rx);
    let pending = unmirrored_derived(&f.app);
    assert!(
        !pending.is_empty(),
        "the arrangement edit must leave a derived clip with a pending echo, or this test is vacuous"
    );
    let snapshot = f.app.test_snapshot_for_undo();

    // Fast path: a scalar edit, then restore.
    if let Some(&t) = h.tracks.first() {
        f.app
            .test_dispatch(Message::Track(TrackMessage::SetTrackVolume(t, -9.0)));
    }
    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    assert!(
        !drain(&f.rx)
            .iter()
            .any(|c| matches!(c, AudioCommand::ClearAll)),
        "a scalar edit takes the diff replay"
    );
    assert_eq!(
        f.app.compose_state().derived_clips,
        snapshot.extras.compose_derived_clips,
        "the fast path keeps the entry whose echo is still pending"
    );

    // The echo lands: the clip is now mirrored and still claimed.
    for cmd in pending_loads {
        if let AudioCommand::LoadMidiClipDirect {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes,
            name,
            trim_start_ticks,
            trim_end_ticks,
        } = cmd
        {
            f.app.test_apply_engine_event(AudioEvent::MidiClipCreated {
                clip_id,
                track_id,
                start_sample,
                duration_ticks,
                name,
                notes,
                trim_start_ticks,
                trim_end_ticks,
            });
        }
    }
    assert!(unmirrored_derived(&f.app).is_empty(), "the echo landed");

    // Slow path: the snapshot lacks that clip, so `ClearAll` wipes it and
    // nothing will ever re-create it — the entry must not survive.
    f.app.test_add_track(9_999, TrackType::Audio);
    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    f.app.test_apply_engine_event(AudioEvent::AllCleared);
    assert_eq!(
        unmirrored_derived(&f.app),
        Vec::<ClipId>::new(),
        "the slow path leaves no entry for a clip ClearAll wiped"
    );
    for (key, id) in &snapshot.extras.compose_derived_clips {
        if !pending.contains(id) {
            assert_eq!(
                f.app.compose_state().derived_clips.get(key),
                Some(id),
                "every entry whose clip was replayed is kept"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Struct `PartialEq` vs. the old `serde_json` compare (ARCH-01 A-8)
// ---------------------------------------------------------------------------

/// The pre-A-8 `same_state` / `gesture_changed_since` file comparison:
/// kept here only as the historical baseline this file's guard test checks
/// the new derived `PartialEq` against, never for production use again.
/// `serde_json::to_value` maps a NaN/infinite float to `Value::Null` rather
/// than failing, so two files that both carry one in the same field compare
/// equal here even though a derived `PartialEq` would call them unequal
/// (`NaN != NaN`) — the trap `ProjectPluginParam`'s hand-written `PartialEq`
/// exists for.
fn json_file_equal(a: &ProjectFile, b: &ProjectFile) -> bool {
    match (serde_json::to_value(a), serde_json::to_value(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// For one fixture: build a snapshot, restore it onto itself through the
/// real undo restore path (a) and apply one small scalar edit (b), and
/// assert struct `PartialEq` and the old JSON compare agree at every step —
/// both on whether the files are equal, and (so a vacuous "both always say
/// unequal" can't pass) on what the answer actually is.
fn check_struct_vs_json_equality(name: &str, load: impl FnOnce(&mut Resonance, &Path)) {
    let mut f = fixture(name, load);
    let before = f.app.test_snapshot_for_undo();

    // (a) A snapshot vs. itself after a round-trip restore. Nothing
    // changed since the snapshot was taken, so this takes the
    // structure-preserving diff replay — the common "click that moved
    // nothing" case `same_state` exists to detect.
    f.app.test_begin_restore_from_snapshot(before.clone());
    let restored = f.app.test_snapshot_for_undo();
    let struct_eq = before.project.file == restored.project.file;
    let json_eq = json_file_equal(&before.project.file, &restored.project.file);
    assert_eq!(
        struct_eq, json_eq,
        "{name}: struct/JSON file equality disagree on a round-trip restore"
    );
    assert!(
        struct_eq,
        "{name}: a round-trip restore of an unchanged snapshot must be a no-op"
    );

    // (b) A snapshot vs. one after a small, real edit (toggling the
    // metronome — present on every fixture, structural on none of them,
    // so every fixture takes the same code path here).
    f.app.test_dispatch(Message::Transport(TransportMessage::ToggleMetronome));
    let edited = f.app.test_snapshot_for_undo();
    let struct_eq = before.project.file == edited.project.file;
    let json_eq = json_file_equal(&before.project.file, &edited.project.file);
    assert_eq!(
        struct_eq, json_eq,
        "{name}: struct/JSON file equality disagree on a small edit"
    );
    assert!(
        !struct_eq,
        "{name}: the edit must actually change the file, or this test is vacuous"
    );
}

// `check_struct_vs_json_equality`'s fixture tags are prefixed `eq-` so its
// `resonance-undo-fixed-point-<tag>-<pid>` scratch directory can never
// collide with another test's fixture of the same base name — the fixed-
// point tests above run concurrently with this one, in the same process
// (same pid), so a shared tag would race on the same directory.
#[test]
fn struct_equality_agrees_with_json_equality_across_fixtures() {
    check_struct_vs_json_equality("eq-demo", load_demo);
    check_struct_vs_json_equality(
        "eq-vocal-songwriting",
        load_template(BuiltinTemplateId::VocalSongwriting),
    );
    check_struct_vs_json_equality(
        "eq-band-recording",
        load_template(BuiltinTemplateId::BandRecording),
    );
    check_struct_vs_json_equality("eq-beatmaking", load_template(BuiltinTemplateId::Beatmaking));
    check_struct_vs_json_equality("eq-empty", load_template(BuiltinTemplateId::Empty));
}
