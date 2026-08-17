//! `generate.drums` must fill the whole section it is asked for (ba doc
//! #272 V-2a).
//!
//! `set_primary_pattern` installed `PatternEntry::once` — `RepeatN(1)` —
//! so a one-bar pattern resolved to a one-bar span and the rest of the
//! section stayed silent.
//!
//! The inverse half — a section nobody generated for still rendering the
//! project default, so you get drums you never asked for — is a
//! deliberate product decision rather than a defect (the GUI depends on
//! that fallback for newly created sections) and is tracked separately
//! by ba todo #1208.

use resonance_app::control_socket::{ControlMessage, ControlRequest, ReplySender};
use resonance_app::message::Message;
use resonance_app::state::ViewMode;
use resonance_app::{Resonance};
use resonance_audio::types::AudioCommand;
use resonance_control::ids::{SectionDefinitionId, TrackId as ProtoTrackId};
use resonance_control::methods::generate::{self as proto, GenerateResult};
use resonance_control::methods::harmony as harmony_proto;
use resonance_control::methods::section as section_proto;
use resonance_control::{KeyScale, Request, Response};

const SECTION_BARS: u32 = 4;
/// 4 bars of 4/4 at 480 ticks per quarter.
const SECTION_TICKS: u64 = SECTION_BARS as u64 * 4 * 480;

fn app_with_project() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/drum-coverage.rprj"));
    app
}

fn roundtrip(app: &mut Resonance, request: Request) -> Response {
    let (reply, rx) = ReplySender::test_pair();
    let _ = app.update(Message::Control(ControlMessage::Request(ControlRequest {
        conn: 1,
        request,
        reply,
    })));
    rx.try_recv().expect("one reply per request")
}

fn call<T: serde::Serialize>(app: &mut Resonance, method: &str, params: &T) -> Response {
    roundtrip(app, Request::new(1, method, params).expect("params serialize"))
}

fn section_named(app: &mut Resonance, name: &str) -> SectionDefinitionId {
    let section_id = call(
        app,
        "section.create",
        &section_proto::CreateParams {
            name: name.to_owned(),
            length_bars: SECTION_BARS,
            scale: None,
            place: true,
        },
    )
    .result::<section_proto::CreateResult>()
    .expect("section.create")
    .section_id;

    let mut params = harmony_proto::ApplyProgressionParams::for_section(section_id);
    params.key = Some(KeyScale {
        tonic: "A".to_owned(),
        scale: "minor".to_owned(),
    });
    params.numerals = Some(["i", "iv", "v", "i"].into_iter().map(str::to_owned).collect());
    call(app, "harmony.apply_progression", &params)
        .result::<harmony_proto::ApplyProgressionResult>()
        .expect("progression applies");
    section_id
}

/// Latest note tick written per section, keyed by the section name in
/// the derived clip's name.
fn last_tick_by_section(
    rx: &resonance_audio::__test_support::Receiver<AudioCommand>,
) -> std::collections::BTreeMap<String, u64> {
    let mut out = std::collections::BTreeMap::new();
    while let Ok(cmd) = rx.try_recv() {
        if let AudioCommand::LoadMidiClipDirect { notes, name, .. } = cmd {
            let section = name.split(" · ").next().unwrap_or(&name).to_owned();
            let last = notes.iter().map(|n| n.start_tick).max().unwrap_or(0);
            out.insert(section, last);
        }
    }
    out
}

/// A section pinned to the "silence" groove stays drumless — including
/// after a later generate elsewhere, which used to refill it from the
/// project default.
///
/// A control generate is now scoped to the section it names (bug 4 of the
/// control-API report), so the intro is not rewritten at all — a stronger
/// guarantee than "rewritten, but still empty". The assertion accepts
/// either, and fails on the one thing that matters: the intro coming back
/// with notes in it.
#[test]
fn a_section_pinned_to_silence_stays_drumless() {
    let mut app = app_with_project();
    let intro = section_named(&mut app, "Intro");
    let verse = section_named(&mut app, "Verse");
    app.test_add_drum_track(82);

    call(&mut app, "generate.drums", &proto::DrumsParams {
        section_id: intro,
        track_id: ProtoTrackId(82),
        pattern: Some("silence".to_owned()),
        density: None,
        seed: None,
    })
    .result::<GenerateResult>()
    .expect("silence generates");

    // Generate a different section; the pinned-silent one must not come
    // back with the default groove.
    let rx = app.test_capture_engine();
    call(&mut app, "generate.drums", &proto::DrumsParams {
        section_id: verse,
        track_id: ProtoTrackId(82),
        pattern: Some("four-on-floor".to_owned()),
        density: None,
        seed: None,
    })
    .result::<GenerateResult>()
    .expect("generates");

    let mut intro_notes = None;
    let mut saw_verse = false;
    while let Ok(cmd) = rx.try_recv() {
        if let AudioCommand::LoadMidiClipDirect { notes, name, .. } = cmd {
            match name.split(" · ").next().unwrap_or(&name) {
                "Intro" => intro_notes = Some(notes.len()),
                "Verse" => saw_verse = true,
                _ => {}
            }
        }
    }
    assert!(saw_verse, "sanity: the verse was written");
    assert!(
        matches!(intro_notes, None | Some(0)),
        "a section pinned to silence must render no drum notes (got {intro_notes:?})"
    );
}

#[test]
fn generated_drums_cover_the_whole_section() {
    let mut app = app_with_project();
    let verse = section_named(&mut app, "Verse");
    app.test_add_drum_track(80);

    let rx = app.test_capture_engine();
    call(&mut app, "generate.drums", &proto::DrumsParams {
        section_id: verse,
        track_id: ProtoTrackId(80),
        pattern: Some("four-on-floor".to_owned()),
        density: None,
        seed: None,
    })
    .result::<GenerateResult>()
    .expect("generates");

    let last = last_tick_by_section(&rx);
    let verse_last = *last.get("Verse").expect("the verse clip was written");
    // The last note should sit in the final bar, not the first: a 4-bar
    // section runs to tick 7680, so anything at or below 1920 means only
    // bar 1 was written.
    assert!(
        verse_last >= SECTION_TICKS - 1920,
        "drums stop at tick {verse_last} of {SECTION_TICKS} — only the first bar was filled"
    );
}
