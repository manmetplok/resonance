//! The inline rename across its surfaces (mixer-cleanup.md §2.3, §3.1):
//! a track or bus name renamed in place on its strip head or in the
//! inspector header. One rename is open at a time and only its surface
//! draws the field; the commit is the same `SetTrackName` / `RenameBus`
//! the control API sends, so it is one undo step.
//!
//! The strip-only behaviours (Enter, blur by pointer, Esc only when
//! captured, tab switch, shortcut isolation) are pinned for track strips
//! in `mixer_strip_anatomy.rs`; this file pins them for the inspector
//! header and the bus surfaces, and the rules that span surfaces.

use iced::{Point, Rectangle, Size};
use iced_test::selector::Candidate;
use iced_test::simulator::Simulator;
use resonance_app::commands::{KeyChord, Mods, NamedKey};
use resonance_app::message::{BusMessage, Message, TrackMessage, UiMessage};
use resonance_app::state::{RenameState, RenameSurface, RenameTarget, TrackState, ViewMode};
use resonance_app::update::shortcuts::TypingProbe;
use resonance_app::{theme, Resonance};
use resonance_audio::types::{AudioCommand, TrackType};
use std::sync::{Arc, Mutex};

const AUDIO: u64 = 1;
const SYNTH: u64 = 2;
const SUB: u64 = 3;
const BUS: u64 = 1;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    app.test_set_active_project(true);
    app.test_add_track(AUDIO, TrackType::Audio);
    app.test_add_track(SYNTH, TrackType::Instrument);
    app.test_push_track(TrackState::new_sub_track(
        SUB,
        2,
        "Synth \u{2192} Out 2".to_owned(),
        SYNTH,
        1,
    ));
    app.test_add_bus(BUS, "Verb");
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app
}

/// An app whose edits record undo (a saved project path).
fn undoable_app(tag: &str) -> Resonance {
    let mut app = app();
    app.test_set_project_path(std::path::PathBuf::from(format!(
        "/tmp/inline-rename-{tag}.rprj"
    )));
    app
}

fn begin(target: RenameTarget, surface: RenameSurface) -> Message {
    Message::Ui(UiMessage::BeginRename(target, surface))
}

fn input(text: &str) -> Message {
    Message::Ui(UiMessage::RenameInput(text.to_owned()))
}

fn commit() -> Message {
    Message::Ui(UiMessage::CommitRename)
}

fn escape(captured: bool) -> Message {
    Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(NamedKey::Escape, Mods::NONE),
        repeat: false,
        captured,
    })
}

fn select_track(id: u64) -> Message {
    Message::Ui(UiMessage::SelectTrack(Some(id)))
}

fn select_bus(id: u64) -> Message {
    Message::Ui(UiMessage::SelectBus(Some(id)))
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

fn bus_name(app: &Resonance, id: u64) -> String {
    app.test_registry()
        .busses
        .iter()
        .find(|b| b.id == id)
        .unwrap()
        .name
        .clone()
}

/// An open rename of a name as it was opened (its seed is the buffer).
fn open(target: RenameTarget, surface: RenameSurface, buffer: &str) -> Option<RenameState> {
    Some(RenameState {
        target,
        surface,
        buffer: buffer.to_owned(),
        seed: buffer.to_owned(),
    })
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

/// Every place `label` is drawn as text, left to right.
fn text_bounds(app: &Resonance, label: &str) -> Vec<Rectangle> {
    let sink: Arc<Mutex<Vec<Rectangle>>> = Arc::default();
    {
        let sink = Arc::clone(&sink);
        let label = label.to_owned();
        let _ = simulator(app).find(move |c: Candidate<'_>| -> Option<()> {
            if let Candidate::Text {
                content,
                visible_bounds: Some(b),
                ..
            } = c
            {
                if content == label {
                    sink.lock().unwrap().push(b);
                }
            }
            None
        });
    }
    let mut found = Arc::try_unwrap(sink).unwrap().into_inner().unwrap();
    found.sort_by(|a, b| a.x.total_cmp(&b.x));
    found
}

/// The inspector pane is the right-most column, so its header's copy of a
/// name is the right-most one drawn.
fn inspector_title(app: &Resonance, label: &str) -> Rectangle {
    *text_bounds(app, label)
        .last()
        .unwrap_or_else(|| panic!("{label:?} should be drawn"))
}

fn double_click(app: &Resonance, at: Rectangle) -> Vec<Message> {
    let mut ui = simulator(app);
    ui.point_at(Point::new(at.x + at.width / 2.0, at.y + at.height / 2.0));
    for _ in 0..2 {
        let _ = ui.simulate(iced_test::simulator::click());
    }
    ui.into_messages().collect()
}

/// How many rename fields the view draws.
fn rename_fields(app: &Resonance) -> usize {
    let found: Arc<Mutex<usize>> = Arc::default();
    {
        let found = Arc::clone(&found);
        let id = iced::widget::Id::new("mixer-inline-rename");
        let _ = simulator(app).find(move |c: Candidate<'_>| -> Option<()> {
            if let Candidate::TextInput { id: Some(at), .. } = c {
                if *at == id {
                    *found.lock().unwrap() += 1;
                }
            }
            None
        });
    }
    let n = *found.lock().unwrap();
    n
}

// ---------------------------------------------------------------------------
// Inspector header: track
// ---------------------------------------------------------------------------

#[test]
fn double_clicking_the_inspector_title_begins_a_track_rename() {
    let mut app = app();
    let _ = app.update(select_track(AUDIO));
    let name = track_name(&app, AUDIO);
    assert_eq!(text_bounds(&app, &name).len(), 2, "strip head and inspector header");
    let messages = double_click(&app, inspector_title(&app, &name));
    assert!(
        messages.iter().any(|m| matches!(
            m,
            Message::Ui(UiMessage::BeginRename(
                RenameTarget::Track(AUDIO),
                RenameSurface::Inspector
            ))
        )),
        "{messages:?}"
    );
}

#[test]
fn an_inspector_rename_commits_on_enter_as_one_undo_step() {
    let mut app = undoable_app("inspector-track");
    let _ = app.update(select_track(AUDIO));
    let before = track_name(&app, AUDIO);
    let _ = app.update(begin(RenameTarget::Track(AUDIO), RenameSurface::Inspector));
    assert_eq!(
        app.test_renaming(),
        open(RenameTarget::Track(AUDIO), RenameSurface::Inspector, &before)
    );
    // The field is drawn once — in the header; the strip keeps its name.
    assert_eq!(rename_fields(&app), 1);
    assert_eq!(app.test_strip_renaming(), None, "not a strip rename");
    assert_eq!(text_bounds(&app, &before).len(), 1, "only the strip draws the name");

    let _ = app.update(input(" Lead Vox  "));
    let _ = app.update(commit());
    assert_eq!(app.test_renaming(), None);
    assert_eq!(track_name(&app, AUDIO), "Lead Vox", "trimmed and applied");
    assert_eq!(text_bounds(&app, "Lead Vox").len(), 2, "strip and header");

    let _ = app.update(Message::Undo);
    assert_eq!(track_name(&app, AUDIO), before, "one undo step restores it");
    let _ = app.update(Message::Redo);
    assert_eq!(track_name(&app, AUDIO), "Lead Vox");
}

#[test]
fn escape_cancels_an_inspector_rename() {
    let mut app = app();
    let _ = app.update(select_track(AUDIO));
    let before = track_name(&app, AUDIO);
    let _ = app.update(begin(RenameTarget::Track(AUDIO), RenameSurface::Inspector));
    let _ = app.update(input("Nope"));
    let _ = app.update(escape(true));
    assert_eq!(app.test_renaming(), None);
    assert_eq!(track_name(&app, AUDIO), before);
}

/// Blur by pointer, as on the strip: a press in the field keeps it, a
/// press with the pointer off it commits.
#[test]
fn a_press_off_the_inspector_field_commits() {
    let mut app = app();
    let _ = app.update(select_track(AUDIO));
    let _ = app.update(begin(RenameTarget::Track(AUDIO), RenameSurface::Inspector));
    let _ = app.update(input("Bass DI"));
    let _ = app.update(Message::Ui(UiMessage::RenamePointer));
    assert!(app.test_renaming().is_some(), "a press in the field");
    let _ = app.update(Message::Ui(UiMessage::RenameHovered(false)));
    let _ = app.update(Message::Ui(UiMessage::RenamePointer));
    assert_eq!(app.test_renaming(), None);
    assert_eq!(track_name(&app, AUDIO), "Bass DI");
}

/// Typing in the header field fires no shortcut either.
#[test]
fn typing_in_the_inspector_field_fires_no_shortcut() {
    let mut app = app();
    app.test_set_typing_probe(TypingProbe::Assume { editing: false });
    let _ = app.update(select_track(AUDIO));
    let _ = app.update(begin(RenameTarget::Track(AUDIO), RenameSurface::Inspector));
    let _ = app.update(Message::Ui(UiMessage::ShortcutKey {
        chord: KeyChord::named(NamedKey::Space, Mods::NONE),
        repeat: false,
        captured: true,
    }));
    assert!(!app.test_transport_playing(), "Space typed into the field");
    assert!(app.test_renaming().is_some());
}

#[test]
fn switching_tabs_commits_an_inspector_rename() {
    let mut app = app();
    let _ = app.update(select_track(AUDIO));
    let _ = app.update(begin(RenameTarget::Track(AUDIO), RenameSurface::Inspector));
    let _ = app.update(input("Bass DI"));
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Arrange)));
    assert_eq!(app.test_renaming(), None);
    assert_eq!(track_name(&app, AUDIO), "Bass DI");
}

/// The header field is drawn nowhere once the inspector moves to another
/// channel (a key or a control call selected it — no press was
/// reported), so the rename commits: leaving it is a blur.
#[test]
fn the_inspector_moving_off_its_channel_commits_the_rename() {
    let mut app = app();
    let _ = app.update(select_track(AUDIO));
    let _ = app.update(begin(RenameTarget::Track(AUDIO), RenameSurface::Inspector));
    let _ = app.update(input("Bass DI"));
    let _ = app.update(select_bus(BUS));
    assert_eq!(app.test_renaming(), None);
    assert_eq!(track_name(&app, AUDIO), "Bass DI");

    // A strip rename survives a selection change: its strip still draws
    // the field.
    let _ = app.update(begin(RenameTarget::Track(SYNTH), RenameSurface::Strip));
    let _ = app.update(select_track(AUDIO));
    assert!(app.test_renaming().is_some());
}

/// Sub-tracks are named after their parent's output port: neither their
/// strip nor their inspector header renames.
#[test]
fn a_sub_track_header_offers_no_rename() {
    let mut app = app();
    let _ = app.update(select_track(SUB));
    let _ = app.update(begin(RenameTarget::Track(SUB), RenameSurface::Inspector));
    assert_eq!(app.test_renaming(), None);
    let messages = double_click(&app, inspector_title(&app, &track_name(&app, SUB)));
    assert!(
        !messages
            .iter()
            .any(|m| matches!(m, Message::Ui(UiMessage::BeginRename(..)))),
        "{messages:?}"
    );
}

/// The open header field is hashed into the inspector's lazy key, for
/// its own channel only.
#[test]
fn the_inspector_fingerprint_ignores_its_header_rename() {
    // The header's title row is built outside the inspector's lazy body,
    // so opening the field and typing in it must not rebuild the body.
    let mut app = app();
    let _ = app.update(select_track(AUDIO));
    let closed = app.test_inspector_fingerprint(AUDIO).unwrap();
    let strip = app.test_track_strip_fingerprint(AUDIO).unwrap();
    let _ = app.update(begin(RenameTarget::Track(AUDIO), RenameSurface::Inspector));
    assert_eq!(app.test_inspector_fingerprint(AUDIO).unwrap(), closed);
    let _ = app.update(input("x"));
    assert_eq!(app.test_inspector_fingerprint(AUDIO).unwrap(), closed);
    assert_eq!(
        app.test_track_strip_fingerprint(AUDIO).unwrap(),
        strip,
        "the strip draws no field for an inspector rename"
    );

    let _ = app.update(select_bus(BUS));
    let bus_closed = app.test_bus_inspector_fingerprint(BUS).unwrap();
    let _ = app.update(begin(RenameTarget::Bus(BUS), RenameSurface::Inspector));
    let _ = app.update(input("y"));
    assert_eq!(app.test_bus_inspector_fingerprint(BUS).unwrap(), bus_closed);
}

// ---------------------------------------------------------------------------
// Busses
// ---------------------------------------------------------------------------

#[test]
fn double_clicking_a_bus_name_begins_a_rename_on_either_surface() {
    let mut app = app();
    let strip = double_click(&app, text_bounds(&app, "Verb")[0]);
    assert!(
        matches!(strip.first(), Some(Message::Ui(UiMessage::SelectBus(Some(BUS))))),
        "the first press selects the bus: {strip:?}"
    );
    assert!(
        strip.iter().any(|m| matches!(
            m,
            Message::Ui(UiMessage::BeginRename(
                RenameTarget::Bus(BUS),
                RenameSurface::Strip
            ))
        )),
        "{strip:?}"
    );

    let _ = app.update(select_bus(BUS));
    let header = double_click(&app, inspector_title(&app, "Verb"));
    assert!(
        header.iter().any(|m| matches!(
            m,
            Message::Ui(UiMessage::BeginRename(
                RenameTarget::Bus(BUS),
                RenameSurface::Inspector
            ))
        )),
        "{header:?}"
    );
}

#[test]
fn a_bus_strip_rename_commits_as_one_undo_step() {
    let mut app = undoable_app("bus-strip");
    let _ = app.update(select_bus(BUS));
    let strip = app.test_bus_strip_fingerprint(BUS).unwrap();
    let _ = app.update(begin(RenameTarget::Bus(BUS), RenameSurface::Strip));
    assert_eq!(
        app.test_renaming(),
        open(RenameTarget::Bus(BUS), RenameSurface::Strip, "Verb")
    );
    assert_ne!(app.test_bus_strip_fingerprint(BUS).unwrap(), strip, "field drawn");
    assert_eq!(rename_fields(&app), 1);

    let _ = app.update(input("Plate Verb"));
    let _ = app.update(commit());
    assert_eq!(app.test_renaming(), None);
    assert_eq!(bus_name(&app, BUS), "Plate Verb");

    let _ = app.update(Message::Undo);
    assert_eq!(bus_name(&app, BUS), "Verb");
    let _ = app.update(Message::Redo);
    assert_eq!(bus_name(&app, BUS), "Plate Verb");
}

#[test]
fn a_bus_inspector_rename_commits_and_escape_cancels() {
    let mut app = app();
    let _ = app.update(select_bus(BUS));
    let _ = app.update(begin(RenameTarget::Bus(BUS), RenameSurface::Inspector));
    let _ = app.update(input("Nope"));
    let _ = app.update(escape(true));
    assert_eq!(app.test_renaming(), None);
    assert_eq!(bus_name(&app, BUS), "Verb");

    let _ = app.update(begin(RenameTarget::Bus(BUS), RenameSurface::Inspector));
    let _ = app.update(input("Room"));
    let _ = app.update(commit());
    assert_eq!(bus_name(&app, BUS), "Room");
    assert!(!text_bounds(&app, "Room").is_empty());
}

#[test]
fn an_empty_or_unchanged_bus_name_renames_nothing() {
    let mut app = undoable_app("bus-empty");
    let _ = app.update(begin(RenameTarget::Bus(BUS), RenameSurface::Strip));
    let _ = app.update(input("  "));
    let _ = app.update(commit());
    let _ = app.update(begin(RenameTarget::Bus(BUS), RenameSurface::Strip));
    let _ = app.update(commit());
    assert_eq!(bus_name(&app, BUS), "Verb");
    assert!(!app.test_can_undo(), "nothing recorded");
}

/// The rename never outlives its bus, and undo drops it before it runs.
#[test]
fn a_bus_rename_is_dropped_with_its_bus_and_by_undo() {
    let mut app = undoable_app("bus-drop");
    let _ = app.update(begin(RenameTarget::Bus(BUS), RenameSurface::Strip));
    let _ = app.update(input("First"));
    let _ = app.update(commit());
    let _ = app.update(begin(RenameTarget::Bus(BUS), RenameSurface::Strip));
    let _ = app.update(input("Second"));
    let _ = app.update(Message::Undo);
    assert_eq!(app.test_renaming(), None, "undo dropped the open rename");
    assert_eq!(bus_name(&app, BUS), "Verb", "and nothing re-committed it");

    let _ = app.update(begin(RenameTarget::Bus(BUS), RenameSurface::Strip));
    let _ = app.update(Message::Bus(BusMessage::RemoveBus(BUS)));
    assert!(app.test_registry().busses.is_empty());
    assert_eq!(app.test_renaming(), None, "its bus is gone");
}

// ---------------------------------------------------------------------------
// One rename across surfaces
// ---------------------------------------------------------------------------

/// Opening a rename anywhere commits the open one — another channel, or
/// the same channel on the other surface — so one field is ever drawn.
#[test]
fn only_one_rename_is_ever_open() {
    let mut app = app();
    let _ = app.update(select_track(AUDIO));
    let _ = app.update(begin(RenameTarget::Track(AUDIO), RenameSurface::Strip));
    let _ = app.update(input("Strip Name"));
    let _ = app.update(begin(RenameTarget::Track(AUDIO), RenameSurface::Inspector));
    assert_eq!(track_name(&app, AUDIO), "Strip Name", "the strip rename committed");
    assert_eq!(
        app.test_renaming(),
        open(RenameTarget::Track(AUDIO), RenameSurface::Inspector, "Strip Name")
    );
    assert_eq!(rename_fields(&app), 1);

    let _ = app.update(input("Header Name"));
    let _ = app.update(begin(RenameTarget::Bus(BUS), RenameSurface::Strip));
    assert_eq!(track_name(&app, AUDIO), "Header Name", "the header rename committed");
    assert_eq!(
        app.test_renaming(),
        open(RenameTarget::Bus(BUS), RenameSurface::Strip, "Verb")
    );
    assert_eq!(rename_fields(&app), 1);

    let _ = app.update(input("Hall"));
    let _ = app.update(begin(RenameTarget::Track(SYNTH), RenameSurface::Strip));
    assert_eq!(bus_name(&app, BUS), "Hall", "the bus rename committed");
    assert_eq!(rename_fields(&app), 1);
}

/// A bus rename reaches the engine (its bus carries the name too), once,
/// and an unchanged name sends nothing.
#[test]
fn a_bus_rename_tells_the_engine() {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_add_bus(BUS, "Verb");
    let _ = app.update(select_bus(BUS));
    while rx.try_recv().is_ok() {}
    let _ = app.update(begin(RenameTarget::Bus(BUS), RenameSurface::Inspector));
    let _ = app.update(input("Hall"));
    let _ = app.update(commit());
    let _ = app.update(Message::Bus(BusMessage::RenameBus(BUS, "Hall".into())));
    let sent: Vec<AudioCommand> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    let renames: Vec<_> = sent
        .iter()
        .filter_map(|c| match c {
            AudioCommand::SetBusName { bus_id, name } => Some((*bus_id, name.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(renames, vec![(BUS, "Hall")], "{sent:?}");
}

/// A track rename from the header is the plain `SetTrackName` (what
/// `track.rename` sends): the strip shows it too.
#[test]
fn a_header_track_rename_shows_on_the_strip() {
    let mut app = app();
    let _ = app.update(select_track(SYNTH));
    let _ = app.update(begin(RenameTarget::Track(SYNTH), RenameSurface::Inspector));
    let _ = app.update(input("Pad"));
    let _ = app.update(commit());
    assert_eq!(text_bounds(&app, "Pad").len(), 2, "strip head and header");
    let _ = app.update(Message::Track(TrackMessage::SetTrackName(SYNTH, "Keys".into())));
    assert_eq!(text_bounds(&app, "Keys").len(), 2);
}

// ---------------------------------------------------------------------------
// Golden images
// ---------------------------------------------------------------------------

fn demo_app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Mixer);
    resonance_app::demo::seed_demo_content(&mut app);
    let _ = app.update(Message::Ui(UiMessage::SwitchView(ViewMode::Mixer)));
    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = simulator(app);
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    crate::common::assert_golden(&snap, path);
}

/// The inspector header with a track rename open: the field replaces the
/// title (swatch and type tag stay beside it); the strip keeps its name.
#[test]
fn mixer_rename_inspector_track_golden() {
    let mut app = demo_app();
    let _ = app.update(select_track(1));
    let _ = app.update(begin(RenameTarget::Track(1), RenameSurface::Inspector));
    let _ = app.update(input("Drums Room"));
    snapshot_to(&app, "tests/snapshots/mixer_rename_inspector_track.png");
}

/// A bus strip with its rename open: the field takes the name's place in
/// the head, beside the mute button.
#[test]
fn mixer_rename_bus_strip_golden() {
    let mut app = demo_app();
    let bus = app.test_registry().busses[0].id;
    let _ = app.update(select_bus(bus));
    let _ = app.update(begin(RenameTarget::Bus(bus), RenameSurface::Strip));
    let _ = app.update(input("Kit Glue"));
    snapshot_to(&app, "tests/snapshots/mixer_rename_bus_strip.png");
}

// ---------------------------------------------------------------------------
// Undo granularity, concurrent renames, no-op renames
// ---------------------------------------------------------------------------

fn undo_depth(app: &Resonance) -> usize {
    app.test_undo_history().test_undo_entries().len()
}

/// `SetTrackName` coalesces per track (the Compose lane inspector renames
/// per keystroke), but two separate inline-rename commits are two undo
/// steps: one Undo lands on the intermediate name.
#[test]
fn two_rename_commits_on_one_track_are_two_undo_steps() {
    let mut app = undoable_app("two-commits");
    let original = track_name(&app, AUDIO);
    for name in ["First", "Second"] {
        let _ = app.update(begin(RenameTarget::Track(AUDIO), RenameSurface::Strip));
        let _ = app.update(input(name));
        let _ = app.update(commit());
    }
    assert_eq!(track_name(&app, AUDIO), "Second");
    let _ = app.update(Message::Undo);
    assert_eq!(track_name(&app, AUDIO), "First", "one Undo, one commit");
    let _ = app.update(Message::Undo);
    assert_eq!(track_name(&app, AUDIO), original);
}

/// A control-API rename landing while a field sits open and untouched
/// re-seeds the field; committing it (Enter, a click away) keeps the
/// remote name instead of writing the stale one back.
#[test]
fn an_untouched_open_rename_does_not_revert_a_remote_rename() {
    let mut app = undoable_app("remote-track");
    let _ = app.update(begin(RenameTarget::Track(AUDIO), RenameSurface::Strip));
    let r = crate::common::call(
        &mut app,
        "track.rename",
        serde_json::json!({ "track_id": AUDIO, "name": "Remote" }),
    );
    assert!(r.error.is_none(), "{:?}", r.error);
    assert_eq!(
        app.test_renaming(),
        open(RenameTarget::Track(AUDIO), RenameSurface::Strip, "Remote"),
        "the field follows the new name"
    );
    let depth = undo_depth(&app);
    let _ = app.update(commit());
    assert_eq!(track_name(&app, AUDIO), "Remote");
    assert_eq!(undo_depth(&app), depth, "the commit changed nothing");

    // The bus twin.
    let _ = app.update(select_bus(BUS));
    let _ = app.update(begin(RenameTarget::Bus(BUS), RenameSurface::Inspector));
    let r = crate::common::call(
        &mut app,
        "bus.rename",
        serde_json::json!({ "bus_id": BUS, "name": "Hall" }),
    );
    assert!(r.error.is_none(), "{:?}", r.error);
    let _ = app.update(commit());
    assert_eq!(bus_name(&app, BUS), "Hall");
}

/// A field the user typed in keeps the user's text: their edit is the
/// newer intent, and commits over the remote name.
#[test]
fn a_typed_open_rename_wins_over_a_remote_rename() {
    let mut app = undoable_app("remote-typed");
    let _ = app.update(begin(RenameTarget::Track(AUDIO), RenameSurface::Strip));
    let _ = app.update(input("Mine"));
    let _ = crate::common::call(
        &mut app,
        "track.rename",
        serde_json::json!({ "track_id": AUDIO, "name": "Remote" }),
    );
    let _ = app.update(commit());
    assert_eq!(track_name(&app, AUDIO), "Mine");
}

/// Renaming to the current name (after trimming) is acknowledged and
/// records nothing: no undo step, no dirty, no revision.
#[test]
fn a_no_op_rename_records_nothing() {
    let mut app = undoable_app("no-op");
    let current = track_name(&app, AUDIO);
    let calls = [
        ("bus.rename", serde_json::json!({ "bus_id": BUS, "name": "  Verb " })),
        (
            "track.rename",
            serde_json::json!({ "track_id": AUDIO, "name": format!(" {current} ") }),
        ),
    ];
    for (method, params) in calls {
        app.test_set_dirty(false);
        let (depth, revision) = (undo_depth(&app), app.revision());
        let r = crate::common::call(&mut app, method, params);
        assert!(r.error.is_none(), "{method} acks: {:?}", r.error);
        assert_eq!(undo_depth(&app), depth, "{method}: no undo step");
        assert_eq!(app.revision(), revision, "{method}: no revision");
        assert!(!app.test_dirty(), "{method}: not dirty");
    }
    // The GUI message itself, sent straight: gated before it records.
    let depth = undo_depth(&app);
    let _ = app.update(Message::Bus(BusMessage::RenameBus(BUS, "Verb ".into())));
    assert_eq!(undo_depth(&app), depth);
    assert!(!app.test_dirty());

    // A real rename is trimmed.
    let r = crate::common::call(
        &mut app,
        "bus.rename",
        serde_json::json!({ "bus_id": BUS, "name": "  Plate  " }),
    );
    assert!(r.error.is_none(), "{:?}", r.error);
    assert_eq!(bus_name(&app, BUS), "Plate");
    assert_eq!(undo_depth(&app), depth + 1);
}
