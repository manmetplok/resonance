//! The slim strip anatomy (mixer-cleanup.md §2, §5): one-line slot
//! lines with no controls, the FX header switch, the empty-instrument
//! line, the master strip's selected highlight, the inline rename on a
//! track strip's head, and the strip fingerprints for the new inputs.
//!
//! "The strip shows state; the inspector edits": these pin that nothing
//! structural is left on a strip, and that what is left reads on one
//! line at any name length.

use std::sync::{Arc, Mutex};

use iced::{Point, Rectangle, Size};
use iced_test::selector::Candidate;
use iced_test::simulator::Simulator;
use resonance_app::commands::{KeyChord, Mods, NamedKey};
use resonance_app::message::{
    BusMessage, MasterMessage, Message, PluginMessage, TrackMessage, UiMessage,
};
use resonance_app::state::{PluginSlotState, ViewMode};
use resonance_app::update::shortcuts::TypingProbe;
use resonance_app::{theme, Resonance};
use resonance_audio::types::{AudioEvent, ScannedPlugin, TrackType};

const AUDIO: u64 = 1;
const SYNTH: u64 = 2;
const EMPTY_SYNTH: u64 = 3;
const BUS: u64 = 1;

const EQ: u64 = 101;
const LONG: u64 = 102;
const WAVE: u64 = 201;
const BUS_FX: u64 = 301;
const MASTER_FX: u64 = 401;

const LONG_NAME: &str = "Resonance Granular Delay Deluxe Edition";

fn slot(instance: u64, name: &str) -> PluginSlotState {
    PluginSlotState::new(
        instance,
        name.to_owned(),
        format!("com.resonance.{instance}"),
        "/plugins/x.clap".to_owned(),
        Vec::new(),
        false,
    )
}

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    app.test_apply_engine_event(AudioEvent::PluginsScanned {
        plugins: vec![ScannedPlugin {
            clap_file_path: "/plugins/wave.clap".to_owned(),
            clap_plugin_id: "com.resonance.wave".to_owned(),
            name: "Resonance Wave".to_owned(),
            vendor: "Resonance".to_owned(),
            is_instrument: true,
            factory_presets: Vec::new(),
        }],
    });
    app.test_add_track(AUDIO, TrackType::Audio);
    app.test_add_track(SYNTH, TrackType::Instrument);
    app.test_add_track(EMPTY_SYNTH, TrackType::Instrument);
    app.test_add_bus(BUS, "Verb");
    app.test_push_track_plugin(AUDIO, slot(EQ, "Resonance EQ"));
    app.test_push_track_plugin(AUDIO, slot(LONG, LONG_NAME));
    app.test_push_track_plugin(SYNTH, slot(WAVE, "Resonance Wave"));
    app.test_push_bus_plugin(BUS, slot(BUS_FX, "Plate"));
    app.test_push_master_plugin(slot(MASTER_FX, "Limiter"));
    // Nothing selected: the inspector shows its placeholder, so every
    // text found below is drawn by a strip.
    let _ = app.update(Message::Ui(UiMessage::SelectTrack(None)));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app
}

fn simulator(app: &Resonance) -> Simulator<'_, Message> {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = vec![theme::ICON_FONT_BYTES.into()];
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    let settings = iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    };
    Simulator::with_size(settings, Size::new(1440.0, 900.0), app.view())
}

/// Every text candidate the view draws, with its visible bounds.
fn texts(app: &Resonance) -> Vec<(String, Rectangle)> {
    let sink: Arc<Mutex<Vec<(String, Rectangle)>>> = Arc::default();
    {
        let sink = Arc::clone(&sink);
        let _ = simulator(app).find(move |c: Candidate<'_>| -> Option<()> {
            if let Candidate::Text {
                content,
                visible_bounds: Some(b),
                ..
            } = c
            {
                sink.lock().unwrap().push((content.to_owned(), b));
            }
            None
        });
    }
    Arc::try_unwrap(sink).unwrap().into_inner().unwrap()
}

fn bounds_of(app: &Resonance, label: &str) -> Rectangle {
    texts(app)
        .into_iter()
        .find(|(c, _)| c == label)
        .unwrap_or_else(|| panic!("{label:?} should be drawn"))
        .1
}

fn press(app: &Resonance, at: Rectangle, clicks: usize) -> Vec<Message> {
    let mut ui = simulator(app);
    ui.point_at(Point::new(at.x + at.width / 2.0, at.y + at.height / 2.0));
    for _ in 0..clicks {
        let _ = ui.simulate(iced_test::simulator::click());
    }
    ui.into_messages().collect()
}

fn track_name(app: &Resonance, id: u64) -> String {
    app.test_registry()
        .tracks
        .iter()
        .find(|t| t.id == id)
        .unwrap()
        .name
        .clone()
}

// ---------------------------------------------------------------------------
// Slot lines
// ---------------------------------------------------------------------------

/// Acceptance 1: no plugin name on any strip wraps, at any name length.
/// The label is ellipsised to the 160 px budget and drawn on one line:
/// its bounds are one text line high and inside the strip.
#[test]
fn a_long_plugin_name_stays_on_one_line() {
    let app = app();
    let label = Resonance::test_strip_plugin_label(LONG_NAME, false);
    assert!(label.ends_with('\u{2026}'), "ellipsised: {label:?}");
    assert!(label.chars().count() <= theme::MIXER_SLOT_LINE_CHARS);

    let b = bounds_of(&app, &label);
    assert!(b.height < 18.0, "one line of size-10 text, got {b:?}");
    assert!(
        b.width <= theme::MIXER_STRIP_WIDTH - 20.0,
        "inside the strip's content width, got {b:?}"
    );
    // The short name next to it sits on the same one-line pitch.
    let eq = bounds_of(&app, "Resonance EQ");
    assert!((eq.height - b.height).abs() < 0.5);
}

/// The strip draws state only: no add pickers, lane header, mono,
/// bounce, trash, reorder carets or per-slot power / editor glyphs.
#[test]
fn the_strips_carry_no_structural_controls() {
    let app = app();
    let drawn: Vec<String> = texts(&app).into_iter().map(|(c, _)| c).collect();
    for gone in [
        "+ Automation",
        "+ Instrument",
        "+ FX",
        "Bounce",
        "Pan",
        "READ",
    ] {
        assert!(!drawn.iter().any(|c| c == gone), "{gone:?} left the strips");
    }
    for glyph in [
        theme::fa::TRASH,
        theme::fa::SLIDERS,
        theme::fa::CARET_UP,
        '\u{25b2}',
        '\u{00d7}',
    ] {
        let g = glyph.to_string();
        assert!(!drawn.contains(&g), "glyph {glyph:?} left the strips");
    }
    // Exactly one power glyph per FX header: two tracks with chains plus
    // the empty instrument track, one bus and the master.
    let power = theme::fa::POWER_OFF.to_string();
    assert_eq!(drawn.iter().filter(|c| **c == power).count(), 5);
}

#[test]
fn slot_line_states_read_back() {
    let mut app = app();
    assert_eq!(
        app.test_strip_slot_line(WAVE),
        Some(("Resonance Wave".to_string(), "active", false, false, true)),
        "slot 0 of an instrument track is the instrument line"
    );
    assert_eq!(
        app.test_strip_slot_line(EQ),
        Some(("Resonance EQ".to_string(), "active", false, false, false))
    );

    app.test_apply_engine_event(AudioEvent::PluginBypassChanged {
        instance_id: EQ,
        bypassed: true,
        own_bypass_param: false,
    });
    let (_, dot, dimmed, ..) = app.test_strip_slot_line(EQ).unwrap();
    assert_eq!((dot, dimmed), ("bypassed", true));

    app.test_apply_engine_event(AudioEvent::PluginLoadFailed {
        instance_id: Some(LONG),
        clap_plugin_id: format!("com.resonance.{LONG}"),
        clap_file_path: "/plugins/x.clap".to_owned(),
        reason: "Failed to load plugin: no such file".to_owned(),
    });
    let (label, dot, ..) = app.test_strip_slot_line(LONG).unwrap();
    assert_eq!(dot, "missing");
    assert!(label.starts_with('\u{26a0}'), "{label:?}");

    let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(WAVE)));
    let (.., focused, _) = app.test_strip_slot_line(WAVE).unwrap();
    assert!(focused, "the focused slot's line is highlighted");
    assert!(!app.test_strip_slot_line(EQ).unwrap().3);
}

/// A slot line on a bus and on the master focuses on a click and opens
/// on a double-click, like a track's.
#[test]
fn bus_and_master_slot_lines_focus_and_open() {
    let app = app();
    for (label, id) in [("Plate", BUS_FX), ("Limiter", MASTER_FX)] {
        let at = bounds_of(&app, label);
        let one = press(&app, at, 1);
        assert!(
            matches!(one.as_slice(), [Message::Plugin(PluginMessage::FocusSlot(i))] if *i == id),
            "{label}: {one:?}"
        );
        let two = press(&app, at, 2);
        assert!(
            two.iter().any(
                |m| matches!(m, Message::Plugin(PluginMessage::OpenPluginWindow(i)) if *i == id)
            ),
            "{label}: {two:?}"
        );
    }
}

/// An instrument track with an empty instrument slot shows a dim
/// "No instrument" line; clicking it selects the track, which puts the
/// inspector's add picker in front of the user.
#[test]
fn an_empty_instrument_slot_reads_no_instrument_and_selects_the_track() {
    let mut app = app();
    let at = bounds_of(&app, "No instrument");
    let messages = press(&app, at, 1);
    assert!(
        matches!(
            messages.as_slice(),
            [Message::Ui(UiMessage::SelectTrack(Some(EMPTY_SYNTH)))]
        ),
        "{messages:?}"
    );
    for m in messages {
        let _ = app.update(m);
    }
    // The inspector now describes the track (its header repeats the
    // name the strip shows) — where the `+ Add instrument` picker is.
    let name = track_name(&app, EMPTY_SYNTH);
    let shown = texts(&app).into_iter().filter(|(c, _)| *c == name).count();
    assert_eq!(shown, 2, "strip head + inspector header");
}

// ---------------------------------------------------------------------------
// The FX header switch
// ---------------------------------------------------------------------------

/// The "FX" switches: `(audio track, master, bus, count)` — the
/// leftmost on the top row, the rightmost on the top row, and the one on
/// the bus row below.
fn fx_switches(app: &Resonance) -> (Rectangle, Rectangle, Rectangle, usize) {
    let all: Vec<Rectangle> = texts(app)
        .into_iter()
        .filter(|(c, _)| c == "FX")
        .map(|(_, b)| b)
        .collect();
    let bus = *all.iter().max_by(|a, b| a.y.total_cmp(&b.y)).unwrap();
    let top: Vec<Rectangle> = all.iter().copied().filter(|b| b.y < bus.y - 50.0).collect();
    let audio = *top.iter().min_by(|a, b| a.x.total_cmp(&b.x)).unwrap();
    let master = *top.iter().max_by(|a, b| a.x.total_cmp(&b.x)).unwrap();
    (audio, master, bus, all.len())
}

#[test]
fn the_fx_switch_toggles_the_whole_chain() {
    let mut app = app();
    let (audio, master, bus, count) = fx_switches(&app);
    assert_eq!(count, 5, "one switch per strip");

    let messages = press(&app, audio, 1);
    assert!(
        matches!(
            messages.as_slice(),
            [Message::Track(TrackMessage::ToggleTrackFxBypass(AUDIO))]
        ),
        "{messages:?}"
    );
    assert!(
        press(&app, master, 1)
            .iter()
            .any(|m| matches!(m, Message::Master(MasterMessage::ToggleMasterFxBypass))),
        "the master strip's FX switch"
    );
    assert!(
        press(&app, bus, 1)
            .iter()
            .any(|m| matches!(m, Message::Bus(BusMessage::ToggleBusFxBypass(BUS)))),
        "the bus strip's FX switch"
    );

    for m in messages {
        let _ = app.update(m);
    }
    // The whole chain dims; the slots' own bypass flags are untouched.
    let (_, dot, dimmed, ..) = app.test_strip_slot_line(EQ).unwrap();
    assert_eq!((dot, dimmed), ("active", true));
}

// ---------------------------------------------------------------------------
// Master selection
// ---------------------------------------------------------------------------

#[test]
fn the_master_strip_highlights_only_while_selected() {
    let mut app = app();
    let (resting, resting_w) = app.test_master_strip_border();
    assert_eq!(resting, theme::LINE_2);
    assert_eq!(resting_w, 0.5);

    let _ = app.update(Message::Ui(UiMessage::SelectMaster));
    assert_eq!(app.test_master_strip_border(), (theme::ACCENT_LINE, 1.0));

    let _ = app.update(Message::Ui(UiMessage::SelectTrack(Some(AUDIO))));
    assert_eq!(app.test_master_strip_border(), (theme::LINE_2, 0.5));
}

// ---------------------------------------------------------------------------
// Inline rename
// ---------------------------------------------------------------------------

/// A double-click on a strip's name raises the rename (the first press
/// selects the track, like the strip around it).
#[test]
fn double_clicking_the_name_begins_a_rename() {
    let app = app();
    let at = bounds_of(&app, &track_name(&app, AUDIO));
    let messages = press(&app, at, 2);
    assert!(
        matches!(
            messages.first(),
            Some(Message::Ui(UiMessage::SelectTrack(Some(AUDIO))))
        ),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, Message::Ui(UiMessage::BeginStripRename(AUDIO)))),
        "{messages:?}"
    );
}

#[test]
fn enter_commits_the_rename_as_one_undoable_step() {
    let mut app = app();
    // Undo records only for a project with a saved path.
    app.test_set_project_path(std::path::PathBuf::from("/tmp/strip-rename.rprj"));
    let before = track_name(&app, AUDIO);
    let _ = app.update(Message::Ui(UiMessage::BeginStripRename(AUDIO)));
    assert_eq!(app.test_strip_renaming(), Some((AUDIO, before.clone())));

    let _ = app.update(Message::Ui(UiMessage::StripRenameInput(
        "  Vox Double ".into(),
    )));
    let _ = app.update(Message::Ui(UiMessage::CommitStripRename));
    assert_eq!(app.test_strip_renaming(), None);
    assert_eq!(track_name(&app, AUDIO), "Vox Double", "trimmed and applied");
    assert!(texts(&app).iter().any(|(c, _)| c == "Vox Double"));

    let _ = app.update(Message::Undo);
    assert_eq!(track_name(&app, AUDIO), before, "one undo step restores it");
}

#[test]
fn escape_cancels_the_rename_even_though_the_field_captured_it() {
    let mut app = app();
    let before = track_name(&app, AUDIO);
    let _ = app.update(Message::Ui(UiMessage::BeginStripRename(AUDIO)));
    let _ = app.update(Message::Ui(UiMessage::StripRenameInput("Nope".into())));
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(NamedKey::Escape, Mods::NONE),
        repeat: false,
        captured: true,
    }));
    assert_eq!(app.test_strip_renaming(), None);
    assert_eq!(track_name(&app, AUDIO), before);
}

/// Typing in the field never fires a global shortcut: the keys reach the
/// reducer as captured and are dropped there.
#[test]
fn typing_in_the_rename_field_fires_no_shortcut() {
    let mut app = app();
    app.test_set_typing_probe(TypingProbe::Assume { editing: false });
    let _ = app.update(Message::Ui(UiMessage::BeginStripRename(AUDIO)));
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(NamedKey::Space, Mods::NONE),
        repeat: false,
        captured: true,
    }));
    assert!(!app.test_transport_playing(), "Space typed into the field");
    assert!(
        app.test_strip_renaming().is_some(),
        "and the rename stays open"
    );

    // The same key uncaptured, with the field closed, does play — so the
    // assertion above is not vacuous.
    let _ = app.update(Message::Ui(UiMessage::CancelStripRename));
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(NamedKey::Space, Mods::NONE),
        repeat: false,
        captured: false,
    }));
    assert!(app.test_transport_playing());
}

/// Blur: a press that leaves the field unfocused commits; one that keeps
/// it focused (a press inside the field) leaves it open.
#[test]
fn losing_focus_commits_the_rename() {
    let mut app = app();
    let _ = app.update(Message::Ui(UiMessage::BeginStripRename(AUDIO)));
    let _ = app.update(Message::Ui(UiMessage::StripRenameInput("Bass DI".into())));
    let _ = app.update(Message::Ui(UiMessage::StripRenameFocusProbed(true)));
    assert!(app.test_strip_renaming().is_some());
    let _ = app.update(Message::Ui(UiMessage::StripRenameFocusProbed(false)));
    assert_eq!(app.test_strip_renaming(), None);
    assert_eq!(track_name(&app, AUDIO), "Bass DI");
}

#[test]
fn an_empty_or_unchanged_name_renames_nothing() {
    let mut app = app();
    let before = track_name(&app, AUDIO);
    let _ = app.update(Message::Ui(UiMessage::BeginStripRename(AUDIO)));
    let _ = app.update(Message::Ui(UiMessage::StripRenameInput("   ".into())));
    let _ = app.update(Message::Ui(UiMessage::CommitStripRename));
    assert_eq!(track_name(&app, AUDIO), before);
    assert_eq!(app.test_strip_renaming(), None);
}

/// Opening a rename on a second strip commits the first.
#[test]
fn a_second_rename_commits_the_first() {
    let mut app = app();
    let _ = app.update(Message::Ui(UiMessage::BeginStripRename(AUDIO)));
    let _ = app.update(Message::Ui(UiMessage::StripRenameInput("First".into())));
    let _ = app.update(Message::Ui(UiMessage::BeginStripRename(SYNTH)));
    assert_eq!(track_name(&app, AUDIO), "First");
    assert_eq!(
        app.test_strip_renaming(),
        Some((SYNTH, track_name(&app, SYNTH)))
    );
}

// ---------------------------------------------------------------------------
// Fingerprints for the new inputs
// ---------------------------------------------------------------------------

/// Focus is hashed per strip: focusing a slot moves its own strip's
/// hash, and leaves every other strip's alone.
#[test]
fn slot_focus_only_moves_the_owning_strips_fingerprint() {
    let mut app = app();
    let audio = app.test_track_strip_fingerprint(AUDIO).unwrap();
    let synth = app.test_track_strip_fingerprint(SYNTH).unwrap();
    let bus = app.test_bus_strip_fingerprint(BUS).unwrap();
    let master = app.test_master_strip_fingerprint();

    let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(EQ)));
    assert_ne!(app.test_track_strip_fingerprint(AUDIO).unwrap(), audio);
    assert_eq!(app.test_track_strip_fingerprint(SYNTH).unwrap(), synth);
    assert_eq!(app.test_bus_strip_fingerprint(BUS).unwrap(), bus);
    assert_eq!(app.test_master_strip_fingerprint(), master);

    // Moving the focus to the master releases the audio strip.
    let audio_focused = app.test_track_strip_fingerprint(AUDIO).unwrap();
    let _ = app.update(Message::Plugin(PluginMessage::FocusSlot(MASTER_FX)));
    assert_ne!(
        app.test_track_strip_fingerprint(AUDIO).unwrap(),
        audio_focused
    );
    assert_eq!(app.test_track_strip_fingerprint(AUDIO).unwrap(), audio);
    assert_ne!(app.test_master_strip_fingerprint(), master);
}

/// The rename buffer is hashed for the strip being renamed only, and
/// every keystroke moves it.
#[test]
fn the_rename_buffer_moves_only_its_strips_fingerprint() {
    let mut app = app();
    let synth = app.test_track_strip_fingerprint(SYNTH).unwrap();
    let _ = app.update(Message::Ui(UiMessage::BeginStripRename(AUDIO)));
    let open = app.test_track_strip_fingerprint(AUDIO).unwrap();
    let _ = app.update(Message::Ui(UiMessage::StripRenameInput("x".into())));
    assert_ne!(app.test_track_strip_fingerprint(AUDIO).unwrap(), open);
    assert_eq!(app.test_track_strip_fingerprint(SYNTH).unwrap(), synth);
}

/// Chain bypass, plugin availability and colour all reach the body.
#[test]
fn chain_bypass_availability_and_colour_move_the_fingerprints() {
    let cases: &[(&str, fn(&mut Resonance))] = &[
        ("chain bypass", |app| {
            let _ = app.update(Message::Track(TrackMessage::ToggleTrackFxBypass(AUDIO)));
        }),
        ("plugin unavailable", |app| {
            app.test_apply_engine_event(AudioEvent::PluginLoadFailed {
                instance_id: Some(EQ),
                clap_plugin_id: format!("com.resonance.{EQ}"),
                clap_file_path: "/plugins/x.clap".to_owned(),
                reason: "gone".to_owned(),
            });
        }),
        ("colour", |app| {
            let _ = app.update(Message::Track(TrackMessage::SetTrackColor(
                AUDIO,
                [9, 9, 9],
            )));
        }),
    ];
    for (facet, mutate) in cases {
        let mut app = app();
        let before = app.test_track_strip_fingerprint(AUDIO).unwrap();
        mutate(&mut app);
        assert_ne!(
            before,
            app.test_track_strip_fingerprint(AUDIO).unwrap(),
            "{facet} must move the strip fingerprint"
        );
    }

    let mut app = app();
    let bus = app.test_bus_strip_fingerprint(BUS).unwrap();
    let _ = app.update(Message::Bus(BusMessage::ToggleBusFxBypass(BUS)));
    assert_ne!(app.test_bus_strip_fingerprint(BUS).unwrap(), bus);
}

/// What left the strip must not churn its cache: mono and an editor
/// opening are inspector / window state now.
#[test]
fn state_the_strip_no_longer_draws_does_not_move_its_fingerprint() {
    let mut app = app();
    let before = app.test_track_strip_fingerprint(AUDIO).unwrap();
    let _ = app.update(Message::Track(TrackMessage::ToggleTrackMono(AUDIO)));
    app.test_apply_engine_event(AudioEvent::PluginEditorState {
        instance_id: EQ,
        open: true,
        failure: None,
    });
    assert_eq!(app.test_track_strip_fingerprint(AUDIO).unwrap(), before);
}
