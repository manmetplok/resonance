//! Vocal roll golden (vocal-mcp-control-and-quality.md §6, §8): a `_`
//! hold the lyrics write, a `+` hold the melody generator made (fainter,
//! warm ink), and a syllable whose written `·` pieces do not match its
//! vowels (BAD underline and corner tick) — side by side with plain sung
//! notes.

use iced::widget::Canvas;
use iced::{Length, Size};
use iced_test::simulator::Simulator;
use resonance_app::compose::ChordState;
use resonance_app::message::Message;
use resonance_app::state::MidiClipState;
use resonance_app::theme;
use resonance_app::view::compose::vocal_roll::VocalRollCanvas;
use resonance_audio::types::MidiNote;
use resonance_music_theory::{Chord, ChordQuality, LyricLine, PitchClass, VocalParams};

use crate::common;

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

fn clip() -> MidiClipState {
    // love · _ · me · + · lo · ve — a beat each.
    let notes = [67u8, 69, 67, 65, 64, 62]
        .iter()
        .enumerate()
        .map(|(i, &note)| MidiNote {
            note,
            velocity: 0.8,
            start_tick: i as u64 * 480,
            duration_ticks: 440,
        })
        .collect::<Vec<_>>();
    MidiClipState {
        id: 7,
        track_id: 3,
        start_sample: 0,
        duration_ticks: 3840,
        name: "vocal".to_owned(),
        notes: notes.into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    }
}

#[test]
fn vocal_roll_user_and_auto_holds_and_split_warning_golden() {
    let clip = clip();
    let mut params = VocalParams::default();
    params.range = (57, 72);
    params.draft = vec![LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: 5,
        text: "love _ me lo\u{00B7}ve".to_owned(),
        locked: true,
    }];
    // Note 3 is a generator hold of `me`.
    let lyrics: Vec<String> = ["", "", "", "+", "", ""].iter().map(|s| (*s).to_owned()).collect();
    let chords = vec![ChordState {
        id: 1,
        start_beat: 0,
        duration_beats: 8,
        chord: Chord::new(PitchClass::C, ChordQuality::Maj),
    }];
    let canvas = VocalRollCanvas {
        keys_blocked: false,
        keymap: resonance_app::commands::BindingMap::default_ref(),
        clip: &clip,
        track_id: 3,
        params: &params,
        bank: resonance_app::compose::vocal_svs::BankProfile::not_installed("lilia"),
        chords: &chords,
        section_beats: 8,
        scroll_y: 0.0,
        zoom_x: 0.25,
        zoom_y: 18.0,
        snap_ticks: 120,
        selected_note: None,
        time_sig_num: 4,
        bpm: 120.0,
        voice_label: "alto",
        lyrics: &lyrics,
    };
    let element: iced::Element<'_, Message> =
        Canvas::new(canvas).width(Length::Fill).height(Length::Fill).into();
    let mut ui = Simulator::with_size(sim_settings(), Size::new(860.0, 420.0), element);
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    common::assert_golden(&snap, "tests/snapshots/compose_vocal_roll_holds.png");
}
