//! Undo snapshot/restore is a fixed point (ARCH-01 A1-1).
//!
//! `build_project_file(restore(snapshot_for_undo(app))) == snapshot.file`,
//! and the whole snapshot (`ProjectFile` + MIDI notes; the former
//! `UndoExtras` side-car is gone since A-7) comes back `same_state`,
//! through BOTH restore paths:
//!
//!   * the structure-preserving diff replay (`try_diff_replay`), reached
//!     when only scalars changed since the snapshot;
//!   * the full `ClearAll → AllCleared → replay_loaded_project` pipeline
//!     (with `io.restoring_undo` set), forced here by adding a track after
//!     the snapshot.
//!
//! Every fixture (the demo project and each built-in template) is first
//! taken through one round of edits per domain — transport, mixer, tempo
//! events, markers, chord track, automation, clip fade/gain, freeze,
//! external instrument, reference trim, drum arrangement, vocal lyrics —
//! so the snapshot carries every field `UndoExtras` used to duplicate; then a
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
use resonance_app::reference::{ReferenceMessage, ReferenceStatus};
use resonance_app::state::ids::DERIVED_CLIP_ID_BASE;
use resonance_app::state::FreezeStatus;
use resonance_app::undo::UndoSnapshot;
use resonance_app::update::project_io::BuiltinTemplateId;
use resonance_app::Resonance;
use resonance_audio::test_support::Receiver;
use resonance_audio::types::{
    ABSource, AudioCommand, AudioEvent, ClipId, PluginInstanceId, ReferenceId, TrackId,
    TrackType,
};
use resonance_common::{
    AutomationLane, AutomationTarget, CurveKind, FreezeCacheRef, FreezeCacheStatus,
};
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
    check("project file", a.project.file == b.project.file);
    out
}

fn assert_fixed_point(f: &Fixture, path: &str, snapshot: &UndoSnapshot) {
    assert_file_fixed_point(
        path,
        &f.app.test_build_project_file(),
        &snapshot.project.file,
    );
    assert_lyrics_canonical(&f.app, path, snapshot);
    let after = f.app.test_snapshot_for_undo();
    assert!(
        Resonance::test_snapshot_same_state(&after, snapshot),
        "{path}: the project file matches but these parts of the snapshot do not: {:?}",
        snapshot_differences(&after, snapshot),
    );
}

/// The live lyric side-table a restore must leave, derived from the
/// snapshot's `ProjectFile` alone: an entry for every clip whose saved
/// `vocal_lyrics` is non-empty, padded (or cut) to the clip's note count
/// — the shape a disk load and a vocal install produce (A-2).
fn canonical_lyrics(snapshot: &UndoSnapshot) -> HashMap<ClipId, Vec<String>> {
    snapshot
        .project
        .file
        .midi_clips
        .iter()
        .filter(|pmc| !pmc.vocal_lyrics.is_empty())
        .map(|pmc| {
            let n = snapshot.project.midi_notes.get(&pmc.id).map_or(0, Vec::len);
            let mut lyrics = pmc.vocal_lyrics.clone();
            lyrics.resize(n, String::new());
            (pmc.id, lyrics)
        })
        .collect()
}

/// Both restore paths leave the lyric side-table in the canonical form
/// of the snapshot's file, and that normalisation is not itself a change
/// a gesture would record.
fn assert_lyrics_canonical(app: &Resonance, path: &str, snapshot: &UndoSnapshot) {
    assert_eq!(
        app.compose_state().vocal_audio.clip_lyrics,
        canonical_lyrics(snapshot),
        "{path}: the restored lyrics are the snapshot file's, padded to the note count"
    );
    assert!(
        !app.test_gesture_changed_since(snapshot),
        "{path}: a restore reads as unchanged against its own snapshot"
    );
}

/// The first round of edits must be visible in the snapshot, or a domain
/// whose dispatch was silently refused would make its leg of the test
/// vacuous (the silent-goldens rule, applied to state).
fn assert_seeded(snapshot: &UndoSnapshot, h: &Handles) {
    let file = &snapshot.project.file;
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
        (file.reference_settings.trim_db + 3.0).abs() < 1e-6,
        "reference trim landed"
    );
    if let Some(&t) = h.tracks.first() {
        assert!(
            file.automation_lanes
                .iter()
                .any(|lane| lane.target == AutomationTarget::TrackGain(t)),
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
            file.tracks
                .iter()
                .find(|pt| pt.id == t)
                .is_some_and(|pt| pt.freeze.is_validly_frozen()),
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
        // The file form drops the trailing empty entry.
        let pmc = file
            .midi_clips
            .iter()
            .find(|pmc| pmc.id == clip)
            .expect("vocal clip");
        assert_eq!(
            pmc.vocal_lyrics.len(),
            short_lyrics("", n).len() - 1,
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
/// twice, from the replay and again from the old `UndoExtras`), with the config
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

/// The derived-clip map an undo snapshot carries, from its
/// `ProjectFile::derived_clips` (A-6).
fn file_derived(snapshot: &UndoSnapshot) -> HashMap<(u64, u64, TrackId), ClipId> {
    snapshot
        .project
        .file
        .derived_clips
        .as_ref()
        .expect("a snapshot's file always carries the derived-clip map")
        .iter()
        .map(|e| ((e.definition_id, e.placement_id, e.track_id), e.clip_id))
        .collect()
}

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
        file_derived(&snapshot),
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
    for (key, id) in &file_derived(&snapshot) {
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
// Vocal lyric shapes (A-2)
// ---------------------------------------------------------------------------

/// Every live shape the lyric side-table can hold for a clip of `n`
/// notes, including those that differ from the note count. The file form
/// strips trailing empties (and cuts past the note count); a restore pads
/// it back to `n`, on both paths.
fn lyric_shapes(n: usize) -> Vec<(&'static str, Option<Vec<String>>)> {
    let exact: Vec<String> = (0..n)
        .map(|i| match i % 3 {
            0 => "la".to_string(),
            1 => String::new(),
            _ => "+".to_string(),
        })
        .collect();
    vec![
        ("short, trailing empty", Some(short_lyrics("la", n))),
        ("longer than the notes", Some(vec!["lo".to_string(); n + 2])),
        ("exact, interior empties", Some(exact)),
        ("all empty", Some(vec![String::new(); n])),
        ("empty vec", Some(Vec::new())),
        ("no entry", None),
    ]
}

/// Each lyric shape round-trips through BOTH restore paths to the same
/// live side-table — the canonical form of the snapshot's file — and a
/// restore never reads as a change against its own snapshot.
#[test]
fn vocal_lyric_shapes_restore_identically_through_both_paths() {
    let mut f = fixture(
        "lyric-shapes",
        load_template(BuiltinTemplateId::VocalSongwriting),
    );
    let (clip, n) = handles(&f.app)
        .vocal_clip
        .expect("the vocal template has a vocal clip");
    assert!(n >= 3, "the vocal clip has enough notes for every shape");
    let other = vec!["da".to_string(); n];

    for (shape, lyrics) in lyric_shapes(n) {
        match &lyrics {
            Some(l) => f.app.test_set_clip_lyrics(clip, l.clone()),
            None => f.app.test_clear_clip_lyrics(clip),
        }
        let snapshot = f.app.test_snapshot_for_undo();

        // Fast path.
        f.app.test_set_clip_lyrics(clip, other.clone());
        let _ = drain(&f.rx);
        f.app.test_begin_restore_from_snapshot(snapshot.clone());
        assert!(
            !drain(&f.rx)
                .iter()
                .any(|c| matches!(c, AudioCommand::ClearAll)),
            "{shape}: a lyric edit takes the diff replay"
        );
        let fast = f.app.compose_state().vocal_audio.clip_lyrics.clone();
        assert_fixed_point(&f, &format!("{shape}: fast path"), &snapshot);

        // Slow path.
        f.app.test_set_clip_lyrics(clip, other.clone());
        f.app.test_add_track(9_999, TrackType::Audio);
        let _ = drain(&f.rx);
        f.app.test_begin_restore_from_snapshot(snapshot.clone());
        f.app.test_apply_engine_event(AudioEvent::AllCleared);
        let _ = drain(&f.rx);
        let slow = f.app.compose_state().vocal_audio.clip_lyrics.clone();
        assert_fixed_point(&f, &format!("{shape}: slow path"), &snapshot);

        assert_eq!(fast, slow, "{shape}: both paths restore the same lyrics");
    }
}

/// A restore that normalises the lyric table — here an all-empty entry,
/// which a slur toggled on and off again leaves behind, comes back as no
/// entry — must not read as a content change to the UPD-05 freeze check:
/// the frozen vocal track stays frozen through both paths.
#[test]
fn a_lyric_normalising_restore_leaves_a_frozen_vocal_track_frozen() {
    let mut f = fixture(
        "lyric-freeze",
        load_template(BuiltinTemplateId::VocalSongwriting),
    );
    let h = handles(&f.app);
    let (clip, n) = h.vocal_clip.expect("the vocal template has a vocal clip");
    let track = f
        .app
        .test_midi_clips()
        .iter()
        .find(|mc| mc.id == clip)
        .map(|mc| mc.track_id)
        .expect("vocal clip");
    let other = *h
        .tracks
        .iter()
        .find(|&&t| t != track)
        .expect("a second track");
    f.app.test_set_clip_lyrics(clip, vec![String::new(); n]);
    let cache_filename = format!("freeze_{track}.wav");
    std::fs::write(
        f.project.with_extension("freeze").join(&cache_filename),
        b"",
    )
    .expect("write freeze cache");
    f.app.test_set_freeze_status(
        track,
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
    let snapshot = f.app.test_snapshot_for_undo();
    let frozen = |app: &Resonance| {
        matches!(app.test_freeze_status(track), FreezeStatus::Frozen { .. })
    };

    // Fast path.
    f.app
        .test_dispatch(Message::Track(TrackMessage::SetTrackVolume(other, -9.0)));
    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    f.app.test_update(Message::Tick);
    assert!(frozen(&f.app), "fast path: the vocal track stays frozen");

    // Slow path.
    f.app.test_add_track(9_999, TrackType::Audio);
    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    f.app.test_apply_engine_event(AudioEvent::AllCleared);
    f.app.test_update(Message::Tick);
    assert!(frozen(&f.app), "slow path: the vocal track stays frozen");
}

// ---------------------------------------------------------------------------
// Automation lanes (A-3)
// ---------------------------------------------------------------------------

/// The live lane map a restore must leave, derived from the snapshot's
/// `ProjectFile` alone — one lane per target, keyed by its own target,
/// the way a disk load builds it.
fn file_lanes(file: &ProjectFile) -> HashMap<AutomationTarget, AutomationLane> {
    file.automation_lanes
        .iter()
        .map(|lane| (lane.target.clone(), lane.clone()))
        .collect()
}

/// Some plugin instance in the project — on a track, a bus or the master.
fn any_plugin(file: &ProjectFile) -> Option<PluginInstanceId> {
    file.tracks
        .iter()
        .flat_map(|t| t.plugins.iter())
        .chain(file.busses.iter().flat_map(|b| b.plugins.iter()))
        .chain(file.master_plugins.iter())
        .map(|p| p.instance_id)
        .next()
}

fn automate(app: &mut Resonance, msg: AutomationMessage) {
    app.test_dispatch(Message::Automation(msg));
}

fn breakpoint(app: &mut Resonance, target: &AutomationTarget, frame: u64, value: f32) {
    automate(
        app,
        AutomationMessage::AddBreakpoint {
            target: target.clone(),
            time_frames: frame,
            value,
            curve: CurveKind::Linear,
        },
    );
}

/// The lane targets the automation test drives.
struct LaneTargets {
    gain: AutomationTarget,
    plugin: AutomationTarget,
    master: AutomationTarget,
    /// Lives on `doomed`, the track the slow-path round deletes.
    pan: AutomationTarget,
    device: AutomationTarget,
    /// Absent from the snapshot; the edits add it.
    added: AutomationTarget,
    doomed: TrackId,
}

/// The edits between snapshot and restore: lanes edited (a point added,
/// a point dragged, the Read flag flipped), one deleted, one added.
fn edit_lanes(app: &mut Resonance, l: &LaneTargets) {
    breakpoint(app, &l.gain, 144_000, 0.9);
    automate(
        app,
        AutomationMessage::DragBreakpoint {
            target: l.plugin.clone(),
            index: 0,
            time_frames: 12_000,
            value: 0.05,
        },
    );
    automate(app, AutomationMessage::ToggleRead(l.device.clone()));
    automate(app, AutomationMessage::RemoveLane(l.master.clone()));
    breakpoint(app, &l.added, 0, 1.0);
}

/// `(set, cleared)` automation targets among `cmds`, sorted by debug form.
fn lane_commands(cmds: &[AudioCommand]) -> (Vec<String>, Vec<String>) {
    let mut set = Vec::new();
    let mut cleared = Vec::new();
    for c in cmds {
        match c {
            AudioCommand::SetAutomationLane { lane } => set.push(format!("{:?}", lane.target)),
            AudioCommand::ClearAutomationLane { target } => cleared.push(format!("{target:?}")),
            _ => {}
        }
    }
    set.sort();
    cleared.sort();
    (set, cleared)
}

fn assert_lanes_restored(app: &Resonance, path: &str, snapshot: &UndoSnapshot) {
    assert_eq!(
        app.test_automation().lanes,
        file_lanes(&snapshot.project.file),
        "{path}: the live lanes are the snapshot file's"
    );
}

/// Automation lanes on every kind of target — a track, a plugin param,
/// the master, a track the slow path deletes, an external device param —
/// round-trip through BOTH restore paths to the snapshot file's lanes,
/// and each path re-sends the engine exactly the lanes that differ.
#[test]
fn automation_lanes_restore_identically_through_both_paths() {
    let mut f = fixture("automation-lanes", load_demo);
    let h = handles(&f.app);
    assert!(h.tracks.len() >= 3, "the demo has three tracks");
    let plugin = any_plugin(&f.app.test_build_project_file())
        .expect("the demo has a plugin to automate");
    let (t0, doomed, ext) = (h.tracks[0], h.tracks[1], h.tracks[h.tracks.len() - 1]);
    let l = LaneTargets {
        gain: AutomationTarget::TrackGain(t0),
        plugin: AutomationTarget::PluginParam {
            instance: plugin,
            param_id: 3,
        },
        master: AutomationTarget::MasterGain,
        pan: AutomationTarget::TrackPan(doomed),
        device: AutomationTarget::DeviceParam {
            track: ext,
            param_id: "cutoff".into(),
        },
        added: AutomationTarget::TrackMute(t0),
        doomed,
    };

    // Seed: several points per lane, added out of order; one lane Read-off.
    f.app.test_dispatch(Message::ExternalInstrument(
        ExternalInstrumentMessage::Enable(ext),
    ));
    breakpoint(&mut f.app, &l.gain, 96_000, 0.8);
    breakpoint(&mut f.app, &l.gain, 0, 0.2);
    breakpoint(&mut f.app, &l.plugin, 48_000, 0.6);
    breakpoint(&mut f.app, &l.plugin, 24_000, 0.3);
    breakpoint(&mut f.app, &l.master, 0, 0.7);
    automate(&mut f.app, AutomationMessage::ToggleRead(l.master.clone()));
    breakpoint(&mut f.app, &l.pan, 0, 0.5);
    breakpoint(&mut f.app, &l.device, 0, 0.4);
    let snapshot = f.app.test_snapshot_for_undo();
    assert_eq!(
        snapshot.project.file.automation_lanes.len(),
        5,
        "five lanes seeded"
    );
    assert_lanes_restored(&f.app, "seeded", &snapshot);

    // What each restore must re-send: every edited lane plus the deleted
    // one, and clear only the added one. The fast path leaves the
    // untouched pan lane alone; the slow path re-sends it, since the
    // track delete dropped it.
    let expected = |with_pan: bool| {
        let mut set: Vec<String> = [&l.gain, &l.plugin, &l.master, &l.device]
            .into_iter()
            .chain(with_pan.then_some(&l.pan))
            .map(|t| format!("{t:?}"))
            .collect();
        set.sort();
        (set, vec![format!("{:?}", l.added)])
    };

    // -- Fast path. --
    edit_lanes(&mut f.app, &l);
    assert!(
        !Resonance::test_snapshot_same_state(&f.app.test_snapshot_for_undo(), &snapshot),
        "the lane edits change the snapshot"
    );
    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    let cmds = drain(&f.rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "lane edits take the diff replay"
    );
    assert_eq!(lane_commands(&cmds), expected(false), "fast path: engine lane traffic");
    assert_lanes_restored(&f.app, "fast path", &snapshot);
    assert_fixed_point(&f, "lanes: fast path", &snapshot);
    let fast = f.app.test_automation().lanes.clone();

    // -- Slow path: also delete the track that owns the pan lane. --
    edit_lanes(&mut f.app, &l);
    f.app
        .test_dispatch(Message::Track(TrackMessage::RequestRemoveTrack(l.doomed)));
    f.app
        .test_dispatch(Message::Track(TrackMessage::ConfirmRemoveTrack));
    assert!(
        !f.app.test_automation().lanes.contains_key(&l.pan),
        "deleting the track dropped its lane"
    );
    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    let mut cmds = drain(&f.rx);
    assert!(
        cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "a deleted track forces the full clear-and-replay"
    );
    f.app.test_apply_engine_event(AudioEvent::AllCleared);
    cmds.extend(drain(&f.rx));
    assert_eq!(lane_commands(&cmds), expected(true), "slow path: engine lane traffic");
    assert_lanes_restored(&f.app, "slow path", &snapshot);
    assert_fixed_point(&f, "lanes: slow path", &snapshot);

    assert_eq!(
        fast,
        f.app.test_automation().lanes,
        "both paths restore the same lanes"
    );
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

// ---------------------------------------------------------------------------
// Track freeze (A-4)
// ---------------------------------------------------------------------------

/// The five freeze shapes the restore test drives, one track each.
struct FreezeTracks {
    /// Frozen at the snapshot and still frozen, cache present.
    kept: TrackId,
    /// Live at the snapshot, frozen since (undo of a freeze): the restore
    /// detaches and deletes its cache.
    undone: TrackId,
    /// Frozen at the snapshot, unfrozen since (which deleted its cache):
    /// the restore can only bring it back `Stale`.
    missing: TrackId,
    /// Stale at the snapshot, cache present: stays `Stale`.
    stale: TrackId,
    /// `Failed` at the snapshot — a transient status, not project state.
    failed: TrackId,
}

fn freeze_ref(t: TrackId, status: FreezeCacheStatus) -> FreezeCacheRef {
    FreezeCacheRef {
        cache_filename: format!("freeze_{t}.wav"),
        sample_rate: 48_000,
        bit_depth: 24,
        render_fingerprint: 7,
        status,
    }
}

fn freeze_cache(f: &Fixture, t: TrackId) -> PathBuf {
    f.project
        .with_extension("freeze")
        .join(format!("freeze_{t}.wav"))
}

fn write_freeze_cache(f: &Fixture, t: TrackId) {
    std::fs::write(freeze_cache(f, t), b"").expect("write freeze cache");
}

fn set_frozen(f: &mut Fixture, t: TrackId) {
    f.app.test_set_freeze_status(
        t,
        FreezeStatus::Frozen {
            cache_ref: freeze_ref(t, FreezeCacheStatus::Frozen),
        },
    );
}

/// The state the snapshot captures.
fn seed_freeze(f: &mut Fixture, t: &FreezeTracks) {
    for id in [t.kept, t.missing, t.stale] {
        write_freeze_cache(f, id);
    }
    set_frozen(f, t.kept);
    set_frozen(f, t.missing);
    f.app.test_set_freeze_status(
        t.stale,
        FreezeStatus::Stale {
            cache_ref: freeze_ref(t.stale, FreezeCacheStatus::Stale),
        },
    );
    f.app.test_set_freeze_status(
        t.failed,
        FreezeStatus::Failed {
            message: "render failed".into(),
        },
    );
}

/// The edits between snapshot and restore: freeze `undone` (a real cache
/// file), unfreeze `missing` (deleting its cache, as `UnfreezeTrack`
/// does), clear the failure.
fn edit_freeze(f: &mut Fixture, t: &FreezeTracks) {
    write_freeze_cache(f, t.undone);
    set_frozen(f, t.undone);
    let _ = std::fs::remove_file(freeze_cache(f, t.missing));
    f.app.test_set_freeze_status(t.missing, FreezeStatus::Idle);
    f.app.test_set_freeze_status(t.failed, FreezeStatus::Idle);
}

/// The live freeze state a restore must leave, whichever path ran.
fn assert_freeze_restored(
    f: &mut Fixture,
    path: &str,
    t: &FreezeTracks,
    cmds: &[AudioCommand],
    kept_baseline: Option<u64>,
) {
    let app = &f.app;
    assert_eq!(
        app.test_freeze_status(t.kept),
        FreezeStatus::Frozen {
            cache_ref: freeze_ref(t.kept, FreezeCacheStatus::Frozen)
        },
        "{path}: a track frozen throughout stays frozen"
    );
    assert!(freeze_cache(f, t.kept).exists(), "{path}: its cache is kept");
    assert_eq!(
        app.test_freeze_content_baseline(t.kept),
        kept_baseline,
        "{path}: its UPD-05 content baseline survives the restore (FU-H2b)"
    );

    assert_eq!(
        app.test_freeze_status(t.undone),
        FreezeStatus::Idle,
        "{path}: undoing a freeze leaves the track live"
    );
    assert!(
        !freeze_cache(f, t.undone).exists(),
        "{path}: undoing a freeze deletes its cache"
    );
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            AudioCommand::UnfreezeTrack { track_id } if *track_id == t.undone
        )),
        "{path}: undoing a freeze detaches its cache from the engine"
    );

    assert_eq!(
        app.test_freeze_status(t.missing),
        FreezeStatus::Stale {
            cache_ref: freeze_ref(t.missing, FreezeCacheStatus::Stale)
        },
        "{path}: a re-frozen track whose cache is gone comes back stale"
    );
    assert_eq!(
        app.test_freeze_status(t.stale),
        FreezeStatus::Stale {
            cache_ref: freeze_ref(t.stale, FreezeCacheStatus::Stale)
        },
        "{path}: a stale track stays stale"
    );
    assert!(freeze_cache(f, t.stale).exists(), "{path}: its cache is kept");
    assert_eq!(
        app.test_freeze_status(t.failed),
        FreezeStatus::Idle,
        "{path}: a failed freeze is not project state; the file form restores it live"
    );

    // No content edit happened, so the post-dispatch UPD-05 check must
    // leave the kept track frozen.
    f.app.test_update(Message::Tick);
    assert!(
        matches!(
            f.app.test_freeze_status(t.kept),
            FreezeStatus::Frozen { .. }
        ),
        "{path}: the restored baseline matches the restored content"
    );
}

fn freeze_statuses(app: &Resonance, t: &FreezeTracks) -> Vec<FreezeStatus> {
    [t.kept, t.undone, t.missing, t.stale, t.failed]
        .into_iter()
        .map(|id| app.test_freeze_status(id))
        .collect()
}

/// Every freeze shape — frozen throughout, a freeze undone, a freeze
/// re-established whose cache is gone, stale, failed — restores to the
/// same live state through BOTH restore paths, derived from the snapshot's
/// `ProjectTrack.freeze` alone, with the cache of the undone freeze
/// deleted and the UPD-05 content baseline of the kept one intact.
#[test]
fn freeze_states_restore_identically_through_both_paths() {
    let mut f = fixture("freeze", load_demo);
    let t = FreezeTracks {
        kept: 800,
        undone: 801,
        missing: 802,
        stale: 803,
        failed: 804,
    };
    for id in 800..=804 {
        f.app.test_add_track(id, TrackType::Instrument);
    }
    seed_freeze(&mut f, &t);
    let kept_baseline = f.app.test_freeze_content_baseline(t.kept);
    assert!(
        kept_baseline.is_some(),
        "the frozen track has a content baseline"
    );
    let snapshot = f.app.test_snapshot_for_undo();
    let frozen_in_file = |id: TrackId| {
        snapshot
            .project
            .file
            .tracks
            .iter()
            .find(|pt| pt.id == id)
            .is_some_and(|pt| pt.freeze.is_frozen)
    };
    assert!(
        frozen_in_file(t.kept) && frozen_in_file(t.missing) && frozen_in_file(t.stale),
        "the snapshot file carries the frozen tracks"
    );

    // -- Fast path. --
    edit_freeze(&mut f, &t);
    f.app
        .test_dispatch(Message::Track(TrackMessage::SetTrackVolume(t.kept, -9.0)));
    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    let cmds = drain(&f.rx);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "freeze + scalar edits take the diff replay"
    );
    assert_freeze_restored(&mut f, "fast path", &t, &cmds, kept_baseline);
    let fast = freeze_statuses(&f.app, &t);

    // -- Slow path: the same edits plus an extra track. --
    write_freeze_cache(&f, t.missing);
    set_frozen(&mut f, t.missing);
    edit_freeze(&mut f, &t);
    f.app.test_add_track(9_999, TrackType::Audio);
    let _ = drain(&f.rx);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    let mut cmds = drain(&f.rx);
    assert!(
        cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "a structural change forces the full clear-and-replay"
    );
    f.app.test_apply_engine_event(AudioEvent::AllCleared);
    cmds.extend(drain(&f.rx));
    assert_freeze_restored(&mut f, "slow path", &t, &cmds, kept_baseline);

    assert_eq!(
        fast,
        freeze_statuses(&f.app, &t),
        "both paths restore the same freeze state"
    );
}

// ---------------------------------------------------------------------------
// Reference A/B content vs monitor state (A-5)
// ---------------------------------------------------------------------------
//
// A reference's *content* — which files are loaded (path, name, cached
// loudness, markers), the active selection, loudness-match and trim — is
// undoable and lives in `ProjectFile::references` / `reference_settings`.
// Its *monitor* state — which source the A/B switch monitors, loop-to-mix,
// the meters — is live and never moved by an undo. These tests drive the
// app through a stand-in for the engine's `ReferencePlayer` (same id
// allocator, same command semantics), so a restore that updates the GUI
// mirror without telling the engine shows up as the two disagreeing.

/// The engine's reference state as the command stream leaves it — a
/// model of `resonance_audio`'s `ReferencePlayer`.
#[derive(Debug, Default)]
struct EngineRefs {
    ids: Vec<u32>,
    active: Option<u32>,
    next_id: u32,
    ab_reference: bool,
    loop_to_mix: bool,
    loudness_match: bool,
    trim_db: f32,
    /// `(id, path)` of every load the model registered, in order, so the
    /// test can echo the ones it wants to land.
    loads: Vec<(u32, String)>,
    /// Commands the engine would mis-handle — a hinted id that is already
    /// registered, a selection of an id it does not hold.
    errors: Vec<String>,
}

impl EngineRefs {
    fn new() -> Self {
        Self {
            next_id: 1,
            ..Self::default()
        }
    }

    fn apply(&mut self, cmd: &AudioCommand) {
        match cmd {
            AudioCommand::ClearAll => {
                let errors = std::mem::take(&mut self.errors);
                *self = Self::new();
                self.errors = errors;
            }
            AudioCommand::LoadReferenceTrack { id_hint, path } => {
                let id = match id_hint {
                    Some(h) => {
                        self.next_id = self.next_id.max(h.0 + 1);
                        h.0
                    }
                    None => {
                        self.next_id += 1;
                        self.next_id - 1
                    }
                };
                if self.ids.contains(&id) {
                    self.errors
                        .push(format!("LoadReferenceTrack reuses live id {id}"));
                }
                self.ids.push(id);
                self.loads.push((id, path.to_string_lossy().into_owned()));
            }
            AudioCommand::RemoveReferenceTrack { id } => {
                self.ids.retain(|x| *x != id.0);
                if self.active == Some(id.0) {
                    self.active = None;
                }
            }
            AudioCommand::SetActiveReference { id } => {
                if self.ids.contains(&id.0) {
                    self.active = Some(id.0);
                } else {
                    self.errors
                        .push(format!("SetActiveReference to unknown id {}", id.0));
                }
            }
            AudioCommand::ClearActiveReference => self.active = None,
            AudioCommand::SetABSource { source } => {
                self.ab_reference = *source == ABSource::Reference;
            }
            AudioCommand::SetRefLoopToMix { enabled } => self.loop_to_mix = *enabled,
            AudioCommand::SetRefLoudnessMatch { enabled } => self.loudness_match = *enabled,
            AudioCommand::SetRefTrim { db } => self.trim_db = *db,
            _ => {}
        }
    }
}

/// Drain the capture channel through the engine model.
fn sync_refs(f: &Fixture, engine: &mut EngineRefs) -> Vec<AudioCommand> {
    let cmds = drain(&f.rx);
    for cmd in &cmds {
        engine.apply(cmd);
    }
    cmds
}

fn ref_file(f: &Fixture, name: &str) -> PathBuf {
    let path = f.root.join(format!("{name}.wav"));
    std::fs::write(&path, b"").expect("write reference file");
    path
}

/// Load `path` as the user does and land the engine's `ReferenceLoaded`.
fn load_reference(
    f: &mut Fixture,
    engine: &mut EngineRefs,
    path: &Path,
    lufs: f32,
) -> ReferenceId {
    f.app
        .test_dispatch(Message::Reference(ReferenceMessage::LoadRequested(
            path.to_path_buf(),
        )));
    sync_refs(f, engine);
    let (id, echoed) = engine
        .loads
        .last()
        .cloned()
        .expect("the load reached the engine");
    assert_eq!(echoed, path.to_string_lossy());
    f.app.test_apply_engine_event(AudioEvent::ReferenceLoaded {
        id: ReferenceId(id),
        name: path.file_stem().unwrap().to_string_lossy().into_owned(),
        path: echoed,
        integrated_lufs: lufs,
        waveform_peaks: vec![(-0.5, 0.5)],
        length_samples: 480_000,
    });
    ReferenceId(id)
}

fn ref_id_of(app: &Resonance, path: &Path) -> ReferenceId {
    let path = path.to_string_lossy();
    app.test_reference()
        .entries
        .iter()
        .find(|e| e.path == path)
        .map(|e| e.id)
        .expect("reference is loaded")
}

fn ref_msg(f: &mut Fixture, engine: &mut EngineRefs, m: ReferenceMessage) {
    f.app.test_dispatch(Message::Reference(m));
    sync_refs(f, engine);
}

/// The live monitor state: `(ab_source, loop_to_mix)`.
fn ref_monitor(app: &Resonance) -> (ABSource, bool) {
    let st = app.test_reference();
    (st.monitor.ab_source, st.monitor.loop_to_mix)
}

/// The reference content a restore must bring back, from a file.
fn reference_content(file: &ProjectFile) -> String {
    let s = &file.reference_settings;
    format!(
        "{}\nactive={:?} loudness_match={} trim_db={}",
        serde_json::to_string_pretty(&file.references).unwrap(),
        s.active,
        s.loudness_match,
        s.trim_db
    )
}

/// Everything wrong with the restored reference state, by name — so the
/// guard reports every leg at once instead of stopping at the first.
fn reference_restore_problems(
    f: &Fixture,
    engine: &EngineRefs,
    snapshot: &UndoSnapshot,
    monitor: (ABSource, bool),
) -> Vec<String> {
    let mut out = Vec::new();
    let st = f.app.test_reference();
    let file = f.app.test_build_project_file();

    // Content: from the snapshot's `ProjectFile`.
    if reference_content(&file) != reference_content(&snapshot.project.file) {
        out.push(format!(
            "content differs from the snapshot:\n--- restored:\n{}\n--- snapshot:\n{}",
            reference_content(&file),
            reference_content(&snapshot.project.file)
        ));
    }
    // Monitor: whatever it was live, untouched.
    if ref_monitor(&f.app) != monitor {
        out.push(format!(
            "monitor state moved: (ab_source, loop_to_mix) = {:?}, was {:?}",
            ref_monitor(&f.app),
            monitor
        ));
    }
    // The engine holds exactly the references the panel shows, under
    // the same ids, with the same selection and levels.
    let mut gui_ids: Vec<u32> = st
        .entries
        .iter()
        .filter(|e| e.status != ReferenceStatus::Missing)
        .map(|e| e.id.0)
        .collect();
    gui_ids.sort_unstable();
    let mut engine_ids = engine.ids.clone();
    engine_ids.sort_unstable();
    if gui_ids != engine_ids {
        out.push(format!("panel ids {gui_ids:?} != engine ids {engine_ids:?}"));
    }
    if st.active_id.map(|id| id.0) != engine.active {
        out.push(format!(
            "panel active {:?} != engine active {:?}",
            st.active_id, engine.active
        ));
    }
    if st.loudness_match != engine.loudness_match {
        out.push(format!(
            "panel loudness_match {} != engine {}",
            st.loudness_match, engine.loudness_match
        ));
    }
    if st.trim_db != engine.trim_db {
        out.push(format!(
            "panel trim {} != engine trim {}",
            st.trim_db, engine.trim_db
        ));
    }
    let (ab, looped) = ref_monitor(&f.app);
    if (ab == ABSource::Reference, looped) != (engine.ab_reference, engine.loop_to_mix) {
        out.push(format!(
            "panel monitor ({ab:?}, {looped}) != engine ({}, {})",
            engine.ab_reference, engine.loop_to_mix
        ));
    }
    if !engine.errors.is_empty() {
        out.push(format!("engine refused: {:?}", engine.errors));
    }
    // And the restore is the snapshot, as far as undo can tell.
    let after = f.app.test_snapshot_for_undo();
    if !Resonance::test_snapshot_same_state(&after, snapshot) {
        out.push(format!(
            "not same_state as the snapshot: {:?}",
            snapshot_differences(&after, snapshot)
        ));
    }
    if f.app.test_gesture_changed_since(snapshot) {
        out.push("gesture_changed_since(snapshot) after the restore".into());
    }
    out
}

/// Reference content edited every way the panel can — trim, loudness
/// match, a remove, a load, a new selection — between snapshot and
/// restore, with the A/B switch and loop-to-mix flipped as well, restores
/// the content through BOTH paths, leaves the monitor where the user put
/// it, and keeps the engine in step with the panel.
#[test]
fn reference_content_restores_and_monitor_state_stays_through_both_paths() {
    let mut f = fixture("reference", load_template(BuiltinTemplateId::Empty));
    let mut engine = EngineRefs::new();
    let _ = sync_refs(&f, &mut engine);
    let (x, a, b, c) = (
        ref_file(&f, "x"),
        ref_file(&f, "a"),
        ref_file(&f, "b"),
        ref_file(&f, "c"),
    );

    // Seed: a reference loaded and removed again first, so the live ids
    // are not the 1..=K a reload hands out.
    let xid = load_reference(&mut f, &mut engine, &x, -10.0);
    ref_msg(&mut f, &mut engine, ReferenceMessage::Remove(xid));
    let aid = load_reference(&mut f, &mut engine, &a, -11.0);
    let bid = load_reference(&mut f, &mut engine, &b, -12.0);
    f.app.test_apply_engine_event(AudioEvent::RefMarkerAdded {
        ref_id: aid,
        marker_id: 1,
        position_samples: 44_100,
        label: "drop".into(),
    });
    ref_msg(&mut f, &mut engine, ReferenceMessage::SetActive(bid));
    ref_msg(&mut f, &mut engine, ReferenceMessage::ToggleLoudnessMatch);
    ref_msg(&mut f, &mut engine, ReferenceMessage::TrimChanged(-3.0));
    let snapshot = f.app.test_snapshot_for_undo();
    let seeded = &snapshot.project.file;
    assert_eq!(seeded.references.len(), 2, "two references landed");
    assert_eq!(seeded.references[0].markers.len(), 1, "marker landed");
    assert_eq!(seeded.reference_settings.active, Some(1), "selection landed");
    assert!(seeded.reference_settings.loudness_match, "loudness match landed");
    assert_eq!(seeded.reference_settings.trim_db, -3.0, "trim landed");

    let dirty = |f: &mut Fixture, engine: &mut EngineRefs| {
        ref_msg(f, engine, ReferenceMessage::TrimChanged(-6.0));
        ref_msg(f, engine, ReferenceMessage::ToggleLoudnessMatch);
        let aid = ref_id_of(&f.app, &a);
        ref_msg(f, engine, ReferenceMessage::Remove(aid));
        let cid = load_reference(f, engine, &c, -13.0);
        ref_msg(f, engine, ReferenceMessage::SetActive(cid));
        assert_eq!(f.app.test_reference().entries.len(), 2);
    };

    // -- Fast path. The monitor goes to the reference, looping. --
    dirty(&mut f, &mut engine);
    ref_msg(&mut f, &mut engine, ReferenceMessage::ToggleAbSource);
    ref_msg(&mut f, &mut engine, ReferenceMessage::ToggleLoopToMix);
    let monitor = ref_monitor(&f.app);
    assert_eq!(monitor, (ABSource::Reference, true));
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    let cmds = sync_refs(&f, &mut engine);
    assert!(
        !cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "reference edits take the diff replay"
    );
    let fast = reference_restore_problems(&f, &engine, &snapshot, monitor);

    // -- Slow path: the same edits plus an extra track. --
    dirty(&mut f, &mut engine);
    f.app.test_add_track(9_999, TrackType::Audio);
    let _ = sync_refs(&f, &mut engine);
    let monitor = ref_monitor(&f.app);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    let cmds = sync_refs(&f, &mut engine);
    assert!(
        cmds.iter().any(|c| matches!(c, AudioCommand::ClearAll)),
        "a structural change forces the full clear-and-replay"
    );
    f.app.test_apply_engine_event(AudioEvent::AllCleared);
    let _ = sync_refs(&f, &mut engine);
    let slow = reference_restore_problems(&f, &engine, &snapshot, monitor);

    assert!(
        fast.is_empty() && slow.is_empty(),
        "fast path: {fast:#?}\nslow path: {slow:#?}"
    );
}

/// Undoing the first selection of a reference leaves nothing selected —
/// in the engine too, or the A/B switch keeps auditioning a reference the
/// panel no longer shows as active.
#[test]
fn undoing_the_first_reference_selection_clears_the_engine_selection() {
    let mut f = fixture("reference-select", load_template(BuiltinTemplateId::Empty));
    let mut engine = EngineRefs::new();
    let _ = sync_refs(&f, &mut engine);
    let a = ref_file(&f, "a");
    let aid = load_reference(&mut f, &mut engine, &a, -11.0);
    let snapshot = f.app.test_snapshot_for_undo();
    assert_eq!(snapshot.project.file.reference_settings.active, None);

    ref_msg(&mut f, &mut engine, ReferenceMessage::SetActive(aid));
    ref_msg(&mut f, &mut engine, ReferenceMessage::ToggleAbSource);
    let monitor = ref_monitor(&f.app);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    let _ = sync_refs(&f, &mut engine);
    let problems = reference_restore_problems(&f, &engine, &snapshot, monitor);
    assert!(problems.is_empty(), "{problems:#?}");
}

/// The monitor state is not part of an undo snapshot: flipping the A/B
/// switch or loop-to-mix is not a change a gesture records.
#[test]
fn reference_monitor_toggles_are_not_snapshot_state() {
    let mut f = fixture("reference-monitor", load_template(BuiltinTemplateId::Empty));
    let mut engine = EngineRefs::new();
    let _ = sync_refs(&f, &mut engine);
    let a = ref_file(&f, "a");
    let aid = load_reference(&mut f, &mut engine, &a, -11.0);
    ref_msg(&mut f, &mut engine, ReferenceMessage::SetActive(aid));
    let before = f.app.test_snapshot_for_undo();
    ref_msg(&mut f, &mut engine, ReferenceMessage::ToggleAbSource);
    ref_msg(&mut f, &mut engine, ReferenceMessage::ToggleLoopToMix);
    assert!(
        !f.app.test_gesture_changed_since(&before),
        "an A/B or loop-to-mix toggle is not an undoable change"
    );
    assert!(Resonance::test_snapshot_same_state(
        &f.app.test_snapshot_for_undo(),
        &before
    ));
}

// ---------------------------------------------------------------------------
// Derived-clip map and counter (A-6)
// ---------------------------------------------------------------------------
//
// The compose section→clip map `(definition, placement, track) → ClipId`
// is persisted as `ProjectFile::derived_clips` (the map verbatim, pending
// and dangling entries included), so both undo paths restore it from the
// snapshot's file; a disk load does too, and a file without the field
// (every project saved before A-6) still gets the positional rebuild. The
// derived-clip id counter is not undo state: it is session-monotonic, so
// an undo never re-issues an id the redo stack still names. See
// `docs/design/A-6-derived-clips.md`.

type DerivedMap = HashMap<(u64, u64, TrackId), ClipId>;

fn derived_map(app: &Resonance) -> DerivedMap {
    app.compose_state().derived_clips.clone()
}

fn derived_counter(app: &Resonance) -> u64 {
    app.compose_state().next_derived_clip_id
}

/// Every derived-range id the app holds: mirrored MIDI and audio clips and
/// the map's values. The counter must stay above all of them.
fn highest_derived_id(app: &Resonance) -> Option<ClipId> {
    let file = app.test_build_project_file();
    app.test_midi_clips()
        .iter()
        .map(|mc| mc.id)
        .chain(file.clips.iter().map(|c| c.id))
        .chain(app.compose_state().derived_clips.values().copied())
        .filter(|id| *id >= DERIVED_CLIP_ID_BASE)
        .max()
}

fn assert_counter_monotonic(app: &Resonance, path: &str, before: u64) {
    let after = derived_counter(app);
    assert!(
        after >= before,
        "{path}: the derived-clip counter went back from {before} to {after} — an id \
         issued before the undo (and still named by the redo stack) would be re-issued"
    );
    if let Some(max) = highest_derived_id(app) {
        assert!(after > max, "{path}: counter {after} not past live id {max}");
    }
}

/// A section resize re-derives the section's lanes between the snapshot
/// and the restore; both paths must bring back the snapshot's map, and
/// neither may rewind the counter past the ids the resize issued.
#[test]
fn a6_derived_clips_restore_through_both_paths_across_a_section_resize() {
    let mut f = fixture("a6-resize", load_demo);
    let h = handles(&f.app);
    let (Some(d), Some(p)) = (h.definition, h.pattern) else {
        panic!("the demo has a section and a drum pattern");
    };
    let at_snapshot = derived_map(&f.app);
    assert!(
        at_snapshot.len() >= 2,
        "the demo derives a vocal and a drum lane, or this test is vacuous: {at_snapshot:?}"
    );
    let snapshot = f.app.test_snapshot_for_undo();
    assert_eq!(file_derived(&snapshot), at_snapshot, "the snapshot's file carries the map");

    // -- Fast path: the drums re-materialise into the same slot ids. --
    f.app
        .test_dispatch(Message::Compose(ComposeMessage::Arrangement(
            ArrangementMessage::AddEntry {
                definition_id: d,
                pattern_id: p,
            },
        )));
    echo_midi_clip_loads(&mut f.app, &f.rx);
    if let Some(&t) = h.tracks.first() {
        f.app
            .test_dispatch(Message::Track(TrackMessage::SetTrackVolume(t, -7.0)));
    }
    let _ = drain(&f.rx);
    let before = derived_counter(&f.app);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    assert!(
        !drain(&f.rx)
            .iter()
            .any(|c| matches!(c, AudioCommand::ClearAll)),
        "a slot-preserving re-materialisation takes the diff replay"
    );
    assert_eq!(derived_map(&f.app), at_snapshot, "fast path: the snapshot's map");
    assert_counter_monotonic(&f.app, "fast path", before);

    // -- Slow path: resize the section (re-derives its lanes), plus a
    // track so the structural fallback is certain. --
    let length = f
        .app
        .compose_state()
        .find_definition(d)
        .map(|def| def.length_bars)
        .expect("definition");
    let _ = f
        .app
        .update(Message::Compose(ComposeMessage::ResizeSection {
            definition_id: d,
            length_bars: length + 1,
        }));
    echo_midi_clip_loads(&mut f.app, &f.rx);
    assert_ne!(
        derived_map(&f.app),
        at_snapshot,
        "the resize must re-derive a lane under a fresh id, or the slow-path counter check is vacuous"
    );
    f.app.test_add_track(9_999, TrackType::Audio);
    let _ = drain(&f.rx);
    let before = derived_counter(&f.app);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    assert!(drain(&f.rx)
        .iter()
        .any(|c| matches!(c, AudioCommand::ClearAll)));
    f.app.test_apply_engine_event(AudioEvent::AllCleared);
    let _ = drain(&f.rx);
    assert_eq!(derived_map(&f.app), at_snapshot, "slow path: the snapshot's map");
    assert_counter_monotonic(&f.app, "slow path", before);
    assert_fixed_point(&f, "slow path after a resize", &snapshot);
}

/// FU-H2a through both paths, restated for A-6: the map a restore brings
/// back is the one live at snapshot time (now read from the snapshot's
/// file), a pending entry survives the diff replay, the full replay keeps
/// exactly the entries whose clip it replayed, and the counter stays past
/// the pending clip once its echo lands.
#[test]
fn a6_a_pending_echo_entry_restores_through_both_paths() {
    let mut f = fixture("a6-pending", load_demo);
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
    let pending_loads = drain(&f.rx);
    let pending = unmirrored_derived(&f.app);
    assert!(!pending.is_empty(), "vacuous: no pending echo");
    let at_snapshot = derived_map(&f.app);
    let snapshot = f.app.test_snapshot_for_undo();
    assert_eq!(
        file_derived(&snapshot),
        at_snapshot,
        "the snapshot's file carries the whole map, the pending entry included"
    );

    // Fast path.
    if let Some(&t) = h.tracks.first() {
        f.app
            .test_dispatch(Message::Track(TrackMessage::SetTrackVolume(t, -9.0)));
    }
    let _ = drain(&f.rx);
    let before = derived_counter(&f.app);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    let _ = drain(&f.rx);
    assert_eq!(derived_map(&f.app), at_snapshot, "fast path keeps the pending entry");
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
    assert_counter_monotonic(&f.app, "fast path, echo landed", before);

    // Slow path.
    f.app.test_add_track(9_999, TrackType::Audio);
    let _ = drain(&f.rx);
    let before = derived_counter(&f.app);
    f.app.test_begin_restore_from_snapshot(snapshot.clone());
    f.app.test_apply_engine_event(AudioEvent::AllCleared);
    let _ = drain(&f.rx);
    let expected: DerivedMap = at_snapshot
        .iter()
        .filter(|(_, id)| !pending.contains(id))
        .map(|(k, v)| (*k, *v))
        .collect();
    assert_eq!(derived_map(&f.app), expected, "slow path drops only the wiped clip's entry");
    assert_counter_monotonic(&f.app, "slow path", before);
}

/// Save to disk, optionally edit the written `project.json`, and load it
/// into a fresh app the way `ProjectLoaded` does.
fn save_and_reload(
    f: &Fixture,
    tag: &str,
    edit: impl FnOnce(&mut serde_json::Value),
) -> (Resonance, Receiver<AudioCommand>) {
    let file = f.app.test_build_project_file();
    let midi: Vec<(ClipId, Vec<resonance_audio::types::MidiNote>)> = f
        .app
        .test_midi_clips()
        .iter()
        .map(|mc| (mc.id, mc.notes.clone()))
        .collect();
    let dir = f.root.join(format!("{tag}.rproj"));
    std::fs::create_dir_all(dir.join("audio")).expect("create reload dir");
    resonance_app::project::save_project(&dir, &file, &[], &midi).expect("save");
    let json = dir.join("project.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).expect("read")).expect("json");
    edit(&mut value);
    std::fs::write(&json, serde_json::to_string_pretty(&value).expect("json")).expect("write");
    let loaded = resonance_app::project::load_project(&dir).expect("load");
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_replay_loaded_project_from(loaded);
    (app, rx)
}

/// The two cases where the pre-A-6 positional rebuild disagrees with the
/// session's map: a derived drum clip the user moved off its bar (the
/// rebuild loses it, so the next regenerate duplicates it), and a
/// hand-drawn clip on the vocal lane that starts exactly on the placement
/// bar (the rebuild claims it, so the next regenerate deletes it).
/// Returns (drum key, vocal key, hand-drawn clip id).
fn diverge_from_positional_rebuild(
    f: &mut Fixture,
) -> ((u64, u64, TrackId), (u64, u64, TrackId), ClipId) {
    let map = derived_map(&f.app);
    let (&drum_key, &drum_clip) = map
        .iter()
        .find(|(_, id)| **id >= DERIVED_CLIP_ID_BASE)
        .expect("the demo materialises a drum clip in the derived range");
    let (&vocal_key, &vocal_clip) = map
        .iter()
        .find(|(_, id)| **id < DERIVED_CLIP_ID_BASE)
        .expect("the demo seeds its vocal clip below the derived range");
    let start = |app: &Resonance, id: ClipId| {
        app.test_midi_clips()
            .iter()
            .find(|mc| mc.id == id)
            .map(|mc| mc.start_sample)
            .expect("mirrored")
    };
    let drum_start = start(&f.app, drum_clip);
    let vocal_start = start(&f.app, vocal_clip);
    f.app.test_dispatch(Message::MidiClip(MidiClipMessage::MoveClipTo {
        clip_id: drum_clip,
        new_start_sample: drum_start + 12_345,
    }));
    let hand_drawn: ClipId = 5_000;
    f.app.test_push_midi_clip(resonance_app::state::MidiClipState {
        id: hand_drawn,
        track_id: vocal_key.2,
        start_sample: vocal_start,
        duration_ticks: 960,
        name: "hand-drawn".into(),
        notes: Vec::new(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    let _ = drain(&f.rx);
    assert_eq!(derived_map(&f.app), map, "neither edit touches the session's map");
    (drum_key, vocal_key, hand_drawn)
}

/// A disk round trip keeps the session's map exactly — including the
/// entries a positional rebuild would get wrong.
#[test]
fn a6_derived_clips_survive_a_disk_round_trip() {
    let mut f = fixture("a6-disk", load_demo);
    let _ = diverge_from_positional_rebuild(&mut f);
    let live = derived_map(&f.app);
    let (reloaded, _rx) = save_and_reload(&f, "reloaded", |v| {
        assert!(
            v.get("derived_clips").is_some_and(|d| d.is_array()),
            "project.json carries the derived-clip map"
        );
    });
    assert_eq!(derived_map(&reloaded), live, "the saved map comes back verbatim");
    assert!(
        derived_counter(&reloaded) > highest_derived_id(&reloaded).unwrap_or(0),
        "a load reserves the counter past every derived id"
    );
}

/// A project saved before A-6 has no `derived_clips` key: it loads and
/// gets the positional rebuild, exactly as before.
#[test]
fn a6_a_project_without_the_field_rebuilds_the_map_by_position() {
    let mut f = fixture("a6-legacy", load_demo);
    let (drum_key, vocal_key, hand_drawn) = diverge_from_positional_rebuild(&mut f);
    let live = derived_map(&f.app);
    let (reloaded, _rx) = save_and_reload(&f, "legacy", |v| {
        v.as_object_mut()
            .expect("project.json is an object")
            .remove("derived_clips");
    });
    let mut expected = live.clone();
    expected.remove(&drum_key);
    expected.insert(vocal_key, hand_drawn);
    assert_eq!(
        derived_map(&reloaded),
        expected,
        "a legacy file takes the positional rebuild: the moved drum clip is lost, the \
         hand-drawn clip on the placement bar is claimed"
    );
}
