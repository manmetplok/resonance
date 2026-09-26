//! VIEW-31: the vocal roll's cache fingerprint must cover everything the
//! cached layer paints — the chord strip's quality (not just the root),
//! the voice label in the corner, and the voicebank that remaps the
//! phoneme strip's symbols.

use resonance_app::compose::ChordState;
use resonance_app::state::MidiClipState;
use resonance_app::view::compose::vocal_roll::{VocalRollCanvas, VocalRollState};
use resonance_audio::types::MidiNote;
use resonance_music_theory::{Chord, ChordQuality, PitchClass, VocalParams, VocalVoicebank};

fn clip() -> MidiClipState {
    MidiClipState {
        id: 7,
        track_id: 3,
        start_sample: 0,
        duration_ticks: 3840,
        name: "vocal".to_owned(),
        notes: vec![MidiNote {
            note: 64,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: 480,
        }],
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

fn chords(quality: ChordQuality) -> Vec<ChordState> {
    vec![ChordState {
        id: 1,
        start_beat: 0,
        duration_beats: 4,
        chord: Chord::new(PitchClass::C, quality),
    }]
}

fn canvas<'a>(
    clip: &'a MidiClipState,
    params: &'a VocalParams,
    chords: &'a [ChordState],
    voice_label: &'a str,
) -> VocalRollCanvas<'a> {
    VocalRollCanvas {
        clip,
        track_id: 3,
        params,
        chords,
        section_beats: 16,
        scroll_y: 0.0,
        zoom_x: 1.0,
        zoom_y: 1.0,
        snap_ticks: 120,
        selected_note: None,
        time_sig_num: 4,
        bpm: 120.0,
        voice_label,
        lyrics: &[],
    }
}

#[test]
fn identical_state_fingerprints_equal() {
    let (c, p, ch) = (clip(), VocalParams::default(), chords(ChordQuality::Maj));
    let state = VocalRollState::default();
    assert_eq!(
        canvas(&c, &p, &ch, "alto").fingerprint(&state),
        canvas(&c, &p, &ch, "alto").fingerprint(&state)
    );
}

#[test]
fn chord_quality_change_invalidates() {
    let (c, p) = (clip(), VocalParams::default());
    let (maj, min) = (chords(ChordQuality::Maj), chords(ChordQuality::Min));
    let state = VocalRollState::default();
    assert_ne!(
        canvas(&c, &p, &maj, "alto").fingerprint(&state),
        canvas(&c, &p, &min, "alto").fingerprint(&state),
        "C -> Cm must repaint the chord strip"
    );
}

#[test]
fn voice_label_change_invalidates() {
    let (c, p, ch) = (clip(), VocalParams::default(), chords(ChordQuality::Maj));
    let state = VocalRollState::default();
    assert_ne!(
        canvas(&c, &p, &ch, "alto").fingerprint(&state),
        canvas(&c, &p, &ch, "tenor").fingerprint(&state)
    );
}

#[test]
fn voicebank_change_invalidates() {
    let c = clip();
    let ch = chords(ChordQuality::Maj);
    let tiger = VocalParams {
        voicebank: VocalVoicebank::Tiger,
        ..VocalParams::default()
    };
    let lilia = VocalParams {
        voicebank: VocalVoicebank::Lilia,
        ..VocalParams::default()
    };
    let state = VocalRollState::default();
    assert_ne!(
        canvas(&c, &tiger, &ch, "alto").fingerprint(&state),
        canvas(&c, &lilia, &ch, "alto").fingerprint(&state),
        "switching voicebank must repaint the phoneme strip"
    );
}
