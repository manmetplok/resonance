//! Cache fingerprints for the Compose canvases (perf defect: the 16 ms
//! app tick re-tessellated every uncached canvas at ~60 fps).
//!
//! Each canvas now renders its static content through a fingerprinted
//! `canvas::Cache`; these tests pin the invalidation contract per canvas:
//!
//! - **same state → equal** — identical inputs produce identical
//!   fingerprints, so the tick reuses the cached geometry;
//! - **content edit → differs** — anything the canvas paints (notes,
//!   pad steps, tempo events, selection, view params) repaints it;
//! - **unrelated / live churn → equal** — state the canvas does not
//!   paint (metering, volumes, other tracks' clips, the metronome
//!   flag) must NOT invalidate the cache.

use resonance_app::compose::{DrumGroup, DrumGroupPad};
use resonance_app::state::{MidiClipState, TrackState};
use resonance_app::view::compose::drumroll::{BarSpanView, ComposeDrumCanvas};
use resonance_app::view::compose::expanded_editor::ExpandedEditorCanvas;
use resonance_app::view::compose::global_tracks::ComposeGlobalTracksCanvas;
use resonance_app::view::compose::manual_motif_canvas::ManualMotifCanvas;
use resonance_app::view::compose::tracks::ComposeTrackCanvas;
use resonance_audio::types::{MidiNote, SignaturePoint, TempoMap, TempoPoint};
use resonance_music_theory::{ManualMotifNote, Mode, PitchClass, Scale};

// ── shared fixtures ──────────────────────────────────────────────────────────

const SAMPLE_RATE: u32 = 48_000;

fn note(pitch: u8, start_tick: u64, duration_ticks: u64) -> MidiNote {
    MidiNote {
        note: pitch,
        velocity: 0.8,
        start_tick,
        duration_ticks,
    }
}

fn clip(id: u64, track_id: u64, notes: Vec<MidiNote>) -> MidiClipState {
    MidiClipState {
        id,
        track_id,
        start_sample: 0,
        duration_ticks: 3840,
        name: format!("clip {id}"),
        notes,
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

fn tracks_fixture() -> Vec<TrackState> {
    vec![
        TrackState::new_instrument(1, 0),
        TrackState::new_instrument(2, 1),
    ]
}

fn clips_fixture() -> Vec<MidiClipState> {
    vec![
        clip(10, 1, vec![note(60, 0, 480), note(64, 480, 480)]),
        clip(11, 2, vec![note(48, 0, 960)]),
    ]
}

// ── ComposeTrackCanvas ───────────────────────────────────────────────────────

fn track_canvas<'a>(
    tracks: &'a [TrackState],
    clips: &'a [MidiClipState],
    tempo_map: &'a TempoMap,
) -> ComposeTrackCanvas<'a> {
    ComposeTrackCanvas {
        tracks,
        midi_clips: clips,
        section_start: 0,
        section_end: 8 * SAMPLE_RATE as u64,
        section_length_bars: 4,
        sample_rate: SAMPLE_RATE,
        tempo_map,
        start_bar: 0,
        scale: Some(Scale::new(PitchClass::C, Mode::Major)),
        details_track_id: None,
        expanded_track_id: None,
    }
}

#[test]
fn track_canvas_fingerprint_stable_for_identical_state() {
    let tracks = tracks_fixture();
    let clips = clips_fixture();
    let tempo = TempoMap::default();
    let a = track_canvas(&tracks, &clips, &tempo).fingerprint();
    let b = track_canvas(&tracks, &clips, &tempo).fingerprint();
    assert_eq!(a, b, "identical state must reuse the cached geometry");
}

#[test]
fn track_canvas_fingerprint_changes_on_note_edit() {
    let tracks = tracks_fixture();
    let clips = clips_fixture();
    let tempo = TempoMap::default();
    let before = track_canvas(&tracks, &clips, &tempo).fingerprint();

    let mut edited = clips_fixture();
    edited[0].notes[0].note = 62;
    let after = track_canvas(&tracks, &edited, &tempo).fingerprint();
    assert_ne!(before, after, "a note edit must repaint the lane grid");
}

#[test]
fn track_canvas_fingerprint_changes_on_tempo_edit() {
    let tracks = tracks_fixture();
    let clips = clips_fixture();
    let tempo = TempoMap::default();
    let before = track_canvas(&tracks, &clips, &tempo).fingerprint();

    let mut edited = TempoMap::default();
    edited.signature_points = vec![SignaturePoint {
        bar: 2,
        numerator: 3,
        denominator: 4,
    }];
    let after = track_canvas(&tracks, &clips, &edited).fingerprint();
    assert_ne!(before, after, "a signature edit moves bar lines");
}

#[test]
fn track_canvas_fingerprint_changes_on_selection() {
    let tracks = tracks_fixture();
    let clips = clips_fixture();
    let tempo = TempoMap::default();
    let base = track_canvas(&tracks, &clips, &tempo).fingerprint();

    let mut selected = track_canvas(&tracks, &clips, &tempo);
    selected.details_track_id = Some(1);
    assert_ne!(base, selected.fingerprint(), "selection highlight is drawn");
}

#[test]
fn track_canvas_fingerprint_ignores_metering_churn() {
    let tracks = tracks_fixture();
    let clips = clips_fixture();
    let tempo = TempoMap::default();
    let before = track_canvas(&tracks, &clips, &tempo).fingerprint();

    let mut churned = tracks_fixture();
    churned[0].level_l = 0.7;
    churned[0].level_r = 0.6;
    churned[1].volume = -6.0;
    churned[1].muted = true;
    let after = track_canvas(&churned, &clips, &tempo).fingerprint();
    assert_eq!(
        before, after,
        "meter/mixer churn is not painted here and must not repaint"
    );
}

// ── ComposeDrumCanvas ────────────────────────────────────────────────────────

fn drum_group(id: u64, pattern: Vec<u8>) -> DrumGroup {
    let cycle = pattern.len() as u32;
    DrumGroup {
        id,
        name: format!("group {id}"),
        color: [200, 120, 40],
        grid: 4,
        cycle,
        phase: 0,
        pads: vec![DrumGroupPad {
            name: "Kick".to_string(),
            note: 36,
            weight: 100,
            pattern,
        }],
        density: 0.5,
        swing: 0.0,
        accent: 0.0,
        humanize: 0.0,
        fills: 0.0,
        style: "Custom".to_string(),
        seed: 1,
    }
}

fn drum_canvas<'a>(
    track: &'a TrackState,
    groups: &'a [DrumGroup],
) -> ComposeDrumCanvas<'a> {
    ComposeDrumCanvas {
        track,
        groups,
        selected_group_id: None,
        track_selected: false,
        bar_spans: vec![BarSpanView {
            bar_start: 0,
            bar_end: 4,
            pattern_color: [200, 120, 40],
            pattern_groups: groups,
            is_fill: false,
        }],
        section_bars: 4,
    }
}

#[test]
fn drum_canvas_fingerprint_stable_for_identical_state() {
    let track = TrackState::new_instrument(3, 2);
    let groups = vec![drum_group(1, vec![100, 0, 0, 0, 100, 0, 0, 0])];
    let a = drum_canvas(&track, &groups).fingerprint();
    let b = drum_canvas(&track, &groups).fingerprint();
    assert_eq!(a, b, "identical state must reuse the cached geometry");
}

#[test]
fn drum_canvas_fingerprint_changes_on_pad_step_toggle() {
    let track = TrackState::new_instrument(3, 2);
    let groups = vec![drum_group(1, vec![100, 0, 0, 0, 100, 0, 0, 0])];
    let before = drum_canvas(&track, &groups).fingerprint();

    let edited = vec![drum_group(1, vec![100, 0, 100, 0, 100, 0, 0, 0])];
    let after = drum_canvas(&track, &edited).fingerprint();
    assert_ne!(before, after, "toggling a step must repaint the cells");
}

#[test]
fn drum_canvas_fingerprint_changes_on_group_selection() {
    let track = TrackState::new_instrument(3, 2);
    let groups = vec![drum_group(1, vec![100, 0, 0, 0])];
    let base = drum_canvas(&track, &groups).fingerprint();

    let mut selected = drum_canvas(&track, &groups);
    selected.track_selected = true;
    selected.selected_group_id = Some(1);
    assert_ne!(base, selected.fingerprint(), "focus tint is drawn");
}

#[test]
fn drum_canvas_fingerprint_ignores_metering_churn() {
    let track = TrackState::new_instrument(3, 2);
    let groups = vec![drum_group(1, vec![100, 0, 0, 0])];
    let before = drum_canvas(&track, &groups).fingerprint();

    let mut churned = TrackState::new_instrument(3, 2);
    churned.level_l = 0.9;
    churned.volume = -3.0;
    let after = drum_canvas(&churned, &groups).fingerprint();
    assert_eq!(
        before, after,
        "meter/mixer churn is not painted here and must not repaint"
    );
}

// ── ComposeGlobalTracksCanvas ────────────────────────────────────────────────

fn global_canvas(tempo_map: &TempoMap) -> ComposeGlobalTracksCanvas<'_> {
    ComposeGlobalTracksCanvas {
        tempo_map,
        start_bar: 0,
        section_length_bars: 8,
    }
}

#[test]
fn global_tracks_fingerprint_stable_for_identical_state() {
    let tempo = TempoMap::default();
    let a = global_canvas(&tempo).fingerprint();
    let b = global_canvas(&tempo).fingerprint();
    assert_eq!(a, b, "identical state must reuse the cached geometry");
}

#[test]
fn global_tracks_fingerprint_changes_on_tempo_event() {
    let tempo = TempoMap::default();
    let before = global_canvas(&tempo).fingerprint();

    let mut edited = TempoMap::default();
    edited.tempo_points = vec![TempoPoint { bar: 2, bpm: 90.0 }];
    let after = global_canvas(&edited).fingerprint();
    assert_ne!(before, after, "a tempo event redraws the tempo graph");
}

#[test]
fn global_tracks_fingerprint_ignores_metronome_toggle() {
    let tempo = TempoMap::default();
    let before = global_canvas(&tempo).fingerprint();

    let mut churned = TempoMap::default();
    churned.metronome_enabled = true;
    let after = global_canvas(&churned).fingerprint();
    assert_eq!(
        before, after,
        "the metronome flag is not painted and must not repaint"
    );
}

// ── ManualMotifCanvas ────────────────────────────────────────────────────────

fn motif_notes() -> Vec<ManualMotifNote> {
    vec![
        ManualMotifNote {
            scale_step: 0,
            duration_sixteenths: 4,
            accent: false,
            is_rest: false,
        },
        ManualMotifNote {
            scale_step: 2,
            duration_sixteenths: 4,
            accent: false,
            is_rest: false,
        },
    ]
}

fn motif_canvas(notes: &[ManualMotifNote]) -> ManualMotifCanvas<'_> {
    ManualMotifCanvas {
        definition_id: 7,
        notes,
        scale: Some(Scale::new(PitchClass::A, Mode::Minor)),
    }
}

#[test]
fn manual_motif_fingerprint_stable_for_identical_state() {
    let notes = motif_notes();
    let a = motif_canvas(&notes).fingerprint();
    let b = motif_canvas(&notes).fingerprint();
    assert_eq!(a, b, "identical state must reuse the cached geometry");
}

#[test]
fn manual_motif_fingerprint_changes_on_note_edit() {
    let notes = motif_notes();
    let base = motif_canvas(&notes).fingerprint();

    let mut accented = motif_notes();
    accented[1].accent = true;
    assert_ne!(
        base,
        motif_canvas(&accented).fingerprint(),
        "accent bars are drawn on note bodies"
    );

    let mut rested = motif_notes();
    rested[0].is_rest = true;
    assert_ne!(
        base,
        motif_canvas(&rested).fingerprint(),
        "rests render in their own row"
    );
}

// ── ExpandedEditorCanvas ─────────────────────────────────────────────────────

fn expanded_canvas<'a>(
    clips: &'a [MidiClipState],
    tempo_map: &'a TempoMap,
) -> ExpandedEditorCanvas<'a> {
    ExpandedEditorCanvas {
        track_id: 1,
        midi_clips: clips,
        section_start: 0,
        section_end: 8 * SAMPLE_RATE as u64,
        section_length_bars: 4,
        sample_rate: SAMPLE_RATE,
        tempo_map,
        start_bar: 0,
        scale: Some(Scale::new(PitchClass::C, Mode::Major)),
        zoom_y: 14.0,
        scroll_x: 0.0,
        scroll_y: 0.0,
    }
}

#[test]
fn expanded_editor_fingerprint_stable_for_identical_state() {
    let clips = clips_fixture();
    let tempo = TempoMap::default();
    let a = expanded_canvas(&clips, &tempo).fingerprint();
    let b = expanded_canvas(&clips, &tempo).fingerprint();
    assert_eq!(a, b, "identical state must reuse the cached geometry");
}

#[test]
fn expanded_editor_fingerprint_changes_on_own_track_note_edit() {
    let clips = clips_fixture();
    let tempo = TempoMap::default();
    let before = expanded_canvas(&clips, &tempo).fingerprint();

    let mut edited = clips_fixture();
    edited[0].notes[1].start_tick += 240; // clip 10 is on track 1
    let after = expanded_canvas(&edited, &tempo).fingerprint();
    assert_ne!(before, after, "editing the edited track's notes repaints");
}

#[test]
fn expanded_editor_fingerprint_ignores_other_tracks_clips() {
    let clips = clips_fixture();
    let tempo = TempoMap::default();
    let before = expanded_canvas(&clips, &tempo).fingerprint();

    let mut churned = clips_fixture();
    churned[1].notes[0].note = 50; // clip 11 is on track 2, not rendered
    let after = expanded_canvas(&churned, &tempo).fingerprint();
    assert_eq!(
        before, after,
        "only the expanded track's clips render in this editor"
    );
}

#[test]
fn expanded_editor_fingerprint_changes_on_view_params() {
    let clips = clips_fixture();
    let tempo = TempoMap::default();
    let base = expanded_canvas(&clips, &tempo).fingerprint();

    let mut scrolled = expanded_canvas(&clips, &tempo);
    scrolled.scroll_x = 120.0;
    assert_ne!(base, scrolled.fingerprint(), "scroll shifts the grid");

    let mut zoomed = expanded_canvas(&clips, &tempo);
    zoomed.zoom_y = 20.0;
    assert_ne!(base, zoomed.fingerprint(), "zoom resizes every row");
}
