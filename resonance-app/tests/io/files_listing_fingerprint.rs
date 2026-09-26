//! VIEW-27: the Files-tab folder listing sits in a `lazy` region keyed on
//! `listing_fingerprint`, and each waveform thumbnail caches its geometry
//! keyed on its peaks. The live audition playhead must not move either
//! key; everything the rows draw must.

use std::path::PathBuf;

use resonance_app::message::{BrowserMessage, Message};
use resonance_app::state::{BrowserTab, ViewMode};
use resonance_app::{demo, Resonance};
use resonance_audio::types::AudioEvent;

fn app() -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_dispatch(Message::Browser(BrowserMessage::ToggleVisible));
    app.test_dispatch(Message::Browser(BrowserMessage::SelectTab(BrowserTab::Files)));
    demo::seed_files_folder(&mut app);
    app
}

fn first_file(app: &Resonance) -> PathBuf {
    PathBuf::from(&app.test_browser().scan.files[0].path)
}

#[test]
fn unchanged_listing_keeps_the_key() {
    let app = app();
    assert!(!app.test_browser().scan.files.is_empty(), "precondition: rows");
    assert_eq!(
        app.test_files_listing_fingerprint(),
        app.test_files_listing_fingerprint()
    );
}

#[test]
fn audition_playhead_does_not_move_the_key() {
    let mut app = app();
    let path = first_file(&app);
    app.test_set_audition_playing(Some(path));
    let before = app.test_files_listing_fingerprint();
    app.test_apply_engine_event(AudioEvent::AuditionPosition { frame: 12_345 });
    assert_eq!(app.test_files_listing_fingerprint(), before);
}

#[test]
fn drawn_state_moves_the_key() {
    let mut app = app();
    let base = app.test_files_listing_fingerprint();

    app.test_dispatch(Message::Browser(BrowserMessage::SetFilter("kick".into())));
    let filtered = app.test_files_listing_fingerprint();
    assert_ne!(filtered, base, "the filter changes which rows show");
    app.test_dispatch(Message::Browser(BrowserMessage::SetFilter(String::new())));
    assert_eq!(app.test_files_listing_fingerprint(), base);

    let path = first_file(&app);
    app.test_set_audition_playing(Some(path));
    assert_ne!(
        app.test_files_listing_fingerprint(),
        base,
        "the playing row carries the highlight"
    );
    app.test_set_audition_playing(None);
    assert_eq!(app.test_files_listing_fingerprint(), base);

    demo::seed_empty_files_folder(&mut app);
    assert_ne!(app.test_files_listing_fingerprint(), base, "a new scan");
}

#[test]
fn thumbnail_key_follows_peaks_and_tint() {
    let peaks = [(-0.5_f32, 0.5_f32), (-0.2, 0.3)];
    let key = Resonance::test_wave_thumbnail_key(&peaks, false);
    assert_eq!(key, Resonance::test_wave_thumbnail_key(&peaks, false));
    assert_ne!(key, Resonance::test_wave_thumbnail_key(&peaks, true));
    assert_ne!(
        key,
        Resonance::test_wave_thumbnail_key(&[(-0.5, 0.5), (-0.2, 0.4)], false)
    );
    assert_ne!(key, Resonance::test_wave_thumbnail_key(&[], false));
}
