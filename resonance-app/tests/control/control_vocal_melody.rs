//! A chord or section-length edit over the control API no longer rewrites
//! a vocal lane's melody unasked (BUG-70, vocal-melody-on-chord-change):
//! the lane keeps its notes and is flagged, unless the call passes
//! `vocal_melody: refit | regenerate` or the project remembers a choice;
//! `vocal_lanes_affected` says what happened. `vocal.refit_melody` re-fits
//! on demand. `song.vocal` reports `hand_edited` and
//! `melody_predates_chords`.

use resonance_app::compose::vocal_melody::VocalMelodyMsg;
use resonance_app::compose::{ComposeMessage, VocalMelodyChoice};
use resonance_app::message::Message;
use resonance_app::Resonance;
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_control::methods::harmony::{ApplyProgressionParams, ApplyProgressionResult};
use resonance_control::methods::section::ResizeResult;
use resonance_control::methods::song::{VocalLaneView, VocalView};
use resonance_control::methods::vocal::RefitMelodyResult;
use resonance_control::{KeyScale, VocalLaneAffected, VocalMelodyChoice as Wire};
use resonance_music_theory::{Mode, PitchClass, Scale};
use serde_json::json;

use crate::control_vocal_render_state::{app_with_project, call, rendered_lane, vocal_view, TRACK};

const TPQ: u64 = TICKS_PER_QUARTER_NOTE;

/// The lane's first clip: id and notes.
fn lane_clip(app: &Resonance) -> (u64, Vec<MidiNote>) {
    let clip = app
        .test_midi_clips()
        .iter()
        .find(|c| c.track_id == TRACK)
        .expect("the lane has a clip");
    (clip.id, clip.notes.as_ref().clone())
}

fn shape(notes: &[MidiNote]) -> Vec<(u8, u64, u64)> {
    notes.iter().map(|n| (n.note, n.start_tick, n.duration_ticks)).collect()
}

fn rhythm(notes: &[MidiNote]) -> Vec<(u64, u64)> {
    notes.iter().map(|n| (n.start_tick, n.duration_ticks)).collect()
}

fn lane(view: &VocalView, def: u64) -> &VocalLaneView {
    view.lanes
        .iter()
        .find(|l| u64::from(l.definition_id) == def)
        .expect("lane listed")
}

fn lyrics_of(app: &Resonance, clip_id: u64) -> Vec<String> {
    app.compose_state()
        .vocal_audio
        .clip_lyrics
        .get(&clip_id)
        .cloned()
        .unwrap_or_default()
}

/// Rewrite the lane's melody by hand over `notes.replace_all`: every note
/// a step up, same rhythm — notes the generator did not write.
fn hand_write(app: &mut Resonance) -> Vec<MidiNote> {
    let (clip_id, notes) = lane_clip(app);
    let written: Vec<MidiNote> = notes
        .iter()
        .map(|n| MidiNote {
            note: n.note + 2,
            ..n.clone()
        })
        .collect();
    let specs: Vec<serde_json::Value> = written
        .iter()
        .map(|n| {
            json!({
                "pitch": n.note,
                "start_beat": n.start_tick as f64 / TPQ as f64,
                "duration_beats": n.duration_ticks as f64 / TPQ as f64,
                "velocity": (n.velocity * 127.0).round() as u8,
            })
        })
        .collect();
    call(
        app,
        "notes.replace_all",
        &json!({ "clip_id": clip_id, "notes": specs, "confirm": true }),
    )
    .result::<serde_json::Value>()
    .expect("notes.replace_all succeeds");
    lane_clip(app).1
}

fn progression(app: &mut Resonance, def: u64, choice: Option<Wire>) -> ApplyProgressionResult {
    let mut params = ApplyProgressionParams::for_section(def.into());
    params.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(["VI", "III", "VII", "i"].map(str::to_owned).to_vec());
    params.vocal_melody = choice;
    call(app, "harmony.apply_progression", &params)
        .result::<ApplyProgressionResult>()
        .expect("progression applies")
}

fn affected(def: u64, outcome: Wire) -> Vec<VocalLaneAffected> {
    vec![VocalLaneAffected {
        definition_id: def.into(),
        track_id: TRACK.into(),
        outcome,
    }]
}

/// The headline case: a hand-written melody survives a chord edit made
/// over MCP, and the lane says it was written for other chords.
#[test]
fn a_hand_written_melody_survives_a_control_chord_edit_and_is_flagged() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let written = hand_write(&mut app);
    let view = vocal_view(&mut app);
    assert!(lane(&view, verse).hand_edited, "notes.* makes the lane hand-edited");
    assert!(!lane(&view, verse).melody_predates_chords);

    let result = progression(&mut app, verse, None);
    assert_eq!(result.vocal_lanes_affected, affected(verse, Wire::Keep));
    assert_eq!(shape(&lane_clip(&app).1), shape(&written), "the notes are untouched");
    let view = vocal_view(&mut app);
    assert!(lane(&view, verse).hand_edited);
    assert!(lane(&view, verse).melody_predates_chords, "flagged as predating the chords");
}

/// A generated lane is kept by default over control too (never lose
/// notes silently), and a later note edit clears the out-of-date flag.
#[test]
fn a_generated_lane_is_kept_by_default_and_a_note_edit_clears_the_flag() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let (_, before) = lane_clip(&app);
    let result = progression(&mut app, verse, None);
    assert_eq!(result.vocal_lanes_affected, affected(verse, Wire::Keep));
    assert_eq!(shape(&lane_clip(&app).1), shape(&before));
    let view = vocal_view(&mut app);
    assert!(!lane(&view, verse).hand_edited);
    assert!(lane(&view, verse).melody_predates_chords);

    hand_write(&mut app);
    let view = vocal_view(&mut app);
    assert!(lane(&view, verse).hand_edited);
    assert!(!lane(&view, verse).melody_predates_chords, "edited again since the chords changed");
}

/// `regenerate` is the old behaviour: exactly what `vocal.generate` with
/// the lane's own seed derives from the new chords, flags cleared.
#[test]
fn regenerate_matches_what_the_generator_derives() {
    let mut regenerated = app_with_project();
    let verse = rendered_lane(&mut regenerated, "Verse", 1);
    hand_write(&mut regenerated);
    let seed = regenerated.test_lane_generator_seed(verse, TRACK).unwrap();
    let result = progression(&mut regenerated, verse, Some(Wire::Regenerate));
    assert_eq!(result.vocal_lanes_affected, affected(verse, Wire::Regenerate));
    let view = vocal_view(&mut regenerated);
    assert!(!lane(&view, verse).hand_edited, "regenerating makes it generated again");
    assert!(!lane(&view, verse).melody_predates_chords);

    let mut reference = app_with_project();
    let verse_ref = rendered_lane(&mut reference, "Verse", 1);
    progression(&mut reference, verse_ref, None);
    call(
        &mut reference,
        "vocal.generate",
        &json!({ "track_id": TRACK, "section_id": verse_ref, "seed": seed, "lyrics": false }),
    )
    .result::<serde_json::Value>()
    .expect("vocal.generate succeeds");
    assert_eq!(shape(&lane_clip(&regenerated).1), shape(&lane_clip(&reference).1));
    assert_ne!(
        shape(&lane_clip(&regenerated).1).len(),
        0,
        "the regenerated lane sings something"
    );
}

/// Every pitch of a re-fitted melody sits on a chord tone (a note starting
/// on a chord change or strong beat, or a beat or longer) or a scale tone
/// of the section's key; rhythm, count, velocity and lyrics never move.
fn assert_fits(app: &Resonance, def: u64, notes: &[MidiNote]) {
    let defn = app.compose_state().find_definition(def).unwrap();
    let scale = defn.scale.unwrap_or(Scale::new(PitchClass::A, Mode::Minor));
    for n in notes {
        let beat = (n.start_tick / TPQ) as u32;
        let chord = defn
            .chords
            .iter()
            .find(|c| beat >= c.start_beat && beat < c.start_beat + c.duration_beats)
            .expect("every note sits under a chord");
        let strong = n.duration_ticks >= TPQ
            || (n.start_tick % TPQ == 0 && (beat % 2 == 0 || beat == chord.start_beat));
        if strong {
            let tones: Vec<u8> = chord.chord.pitch_classes().map(|p| p.to_semitone()).collect();
            assert!(tones.contains(&(n.note % 12)), "{} fits {}", n.note, chord.chord);
        } else {
            assert!(scale.contains(n.note), "{} is in the key", n.note);
        }
    }
}

#[test]
fn refit_keeps_rhythm_count_and_lyrics_and_fits_the_new_chords() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let written = hand_write(&mut app);
    let (clip_id, _) = lane_clip(&app);
    let lyrics = lyrics_of(&app, clip_id);

    let result = progression(&mut app, verse, Some(Wire::Refit));
    assert_eq!(result.vocal_lanes_affected, affected(verse, Wire::Refit));
    let (after_id, refit) = lane_clip(&app);
    assert_eq!(after_id, clip_id, "a re-fit edits the clip in place");
    assert_eq!(refit.len(), written.len(), "note count unchanged");
    assert_eq!(rhythm(&refit), rhythm(&written), "rhythm unchanged");
    assert!(refit.iter().zip(&written).all(|(a, b)| a.velocity == b.velocity));
    assert_eq!(lyrics_of(&app, clip_id), lyrics, "lyrics and holds unchanged");
    assert_fits(&app, verse, &refit);
    let view = vocal_view(&mut app);
    assert!(lane(&view, verse).hand_edited, "a re-fit keeps the lane the user's");
    assert!(!lane(&view, verse).melody_predates_chords);
}

/// Keep, then `vocal.refit_melody`: the same notes as `vocal_melody:
/// refit` on the chord edit itself, and the flag clears.
#[test]
fn vocal_refit_melody_refits_a_kept_lane_on_demand() {
    let mut kept = app_with_project();
    let verse = rendered_lane(&mut kept, "Verse", 1);
    hand_write(&mut kept);
    progression(&mut kept, verse, None);
    let result: RefitMelodyResult = call(
        &mut kept,
        "vocal.refit_melody",
        &json!({ "track_id": TRACK, "section_id": verse }),
    )
    .result()
    .expect("vocal.refit_melody succeeds");
    assert_eq!(u64::from(result.section_id), verse);
    assert_eq!(result.note_count, lane_clip(&kept).1.len());
    assert!(result.notes_changed > 0, "the hand-written line clashed with the new chords");
    assert!(!lane(&vocal_view(&mut kept), verse).melody_predates_chords);

    let mut refit = app_with_project();
    let verse_b = rendered_lane(&mut refit, "Verse", 1);
    hand_write(&mut refit);
    progression(&mut refit, verse_b, Some(Wire::Refit));
    assert_eq!(shape(&lane_clip(&kept).1), shape(&lane_clip(&refit).1));
}

#[test]
fn vocal_refit_melody_refuses_a_lane_without_notes() {
    let mut app = app_with_project();
    let section = call(
        &mut app,
        "section.create",
        &json!({ "name": "Bridge", "length_bars": 4 }),
    )
    .result::<serde_json::Value>()
    .expect("section.create");
    let def = section["section_id"].as_u64().unwrap();
    app.test_install_vocal_lane(def, TRACK);
    let response = call(
        &mut app,
        "vocal.refit_melody",
        &json!({ "track_id": TRACK, "section_id": def }),
    );
    let error = response.error.expect("refused");
    assert!(error.message.contains("no notes"), "{}", error.message);
}

/// One `edit.undo` takes back the chord edit and its vocal outcome.
#[test]
fn one_undo_takes_back_the_chord_edit_and_the_refit() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let written = hand_write(&mut app);
    let chords_before = app.test_section_chord_spans(verse);
    let symbols_before: Vec<String> = app
        .compose_state()
        .find_definition(verse)
        .unwrap()
        .chords
        .iter()
        .map(|c| c.chord.to_string())
        .collect();
    progression(&mut app, verse, Some(Wire::Refit));
    assert_ne!(shape(&lane_clip(&app).1), shape(&written));

    call(&mut app, "edit.undo", &json!({}))
        .result::<serde_json::Value>()
        .expect("edit.undo");
    assert_eq!(shape(&lane_clip(&app).1), shape(&written), "the notes come back");
    assert_eq!(app.test_section_chord_spans(verse), chords_before);
    let symbols_after: Vec<String> = app
        .compose_state()
        .find_definition(verse)
        .unwrap()
        .chords
        .iter()
        .map(|c| c.chord.to_string())
        .collect();
    assert_eq!(symbols_after, symbols_before, "the chords come back");
    let view = vocal_view(&mut app);
    assert!(lane(&view, verse).hand_edited, "the flags ride undo");
}

/// "Remember for this project" in the GUI question also answers control
/// calls that name no choice.
#[test]
fn a_remembered_choice_applies_to_control_calls() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    hand_write(&mut app);
    let first = app.compose_state().find_definition(verse).unwrap().chords[0].clone();
    let mut chord = first.chord;
    chord.root = chord.root.transpose(2);
    let _ = app.update(Message::Compose(ComposeMessage::EditChord {
        definition_id: verse,
        chord_id: first.id,
        chord,
    }));
    assert!(app.compose_state().vocal_melody.prompt.is_some(), "the GUI edit asks");
    for m in [
        VocalMelodyMsg::SetAllChoices(VocalMelodyChoice::Refit),
        VocalMelodyMsg::ToggleRemember,
        VocalMelodyMsg::Apply,
    ] {
        let _ = app.update(Message::Compose(ComposeMessage::VocalMelody(m)));
    }
    assert_eq!(app.compose_state().vocal_melody.remembered, Some(VocalMelodyChoice::Refit));

    let result = progression(&mut app, verse, None);
    assert_eq!(result.vocal_lanes_affected, affected(verse, Wire::Refit));
    assert!(!lane(&vocal_view(&mut app), verse).melody_predates_chords);
    assert_eq!(
        app.test_build_project_file().vocal_melody_choice,
        Some(VocalMelodyChoice::Refit),
        "the choice is saved with the project"
    );
}

/// `section.resize` follows the same rule: a kept melody is re-laid at the
/// new length, untouched.
#[test]
fn a_resize_keeps_a_hand_written_melody_at_the_new_length() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let written = hand_write(&mut app);
    let result: ResizeResult = call(
        &mut app,
        "section.resize",
        &json!({ "section_id": verse, "length_bars": 6 }),
    )
    .result()
    .expect("section.resize succeeds");
    assert_eq!(result.vocal_lanes_affected, affected(verse, Wire::Keep));
    let clip = app
        .test_midi_clips()
        .iter()
        .find(|c| c.track_id == TRACK)
        .unwrap();
    assert_eq!(clip.duration_ticks, 6 * 4 * TPQ, "the clip spans the new length");
    assert_eq!(shape(&clip.notes), shape(&written));
}

/// A lane the generator would re-derive identically has nothing to lose:
/// it is not reported and not flagged.
#[test]
fn an_edit_that_changes_no_melody_reports_nothing() {
    let mut app = app_with_project();
    let verse = rendered_lane(&mut app, "Verse", 1);
    let result: ResizeResult = call(
        &mut app,
        "section.resize",
        &json!({ "section_id": verse, "length_bars": 5 }),
    )
    .result()
    .expect("section.resize succeeds");
    assert!(result.vocal_lanes_affected.is_empty(), "{:?}", result.vocal_lanes_affected);
    assert!(!lane(&vocal_view(&mut app), verse).melody_predates_chords);
}
