//! The saved `bpm` / time signature are the song's (its bar-1 tempo and
//! meter events), not whatever sits under the playhead (code review
//! ARCH2-10). `transport.bpm` / `time_sig_*` follow the playhead through
//! a tempo change for the display; before the fix the project file was
//! written from them, so a song at 90 BPM that changes to 140 at bar 33
//! saved `bpm: 140` if playback stopped at bar 40.

use resonance_app::message::Message;
use resonance_app::state::{SignatureEvent, TempoEvent};
use resonance_app::Resonance;
use resonance_audio::types::AudioEvent;

#[test]
fn saved_tempo_is_the_bar_one_tempo_not_the_playhead_tempo() {
    let (mut app, _task, _rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_set_flat_tempo(90.0);
    app.test_push_tempo_event(TempoEvent { bar: 32, bpm: 140.0 });
    let (num0, den0) = app.test_transport_time_sig();
    let sigs = app.test_signature_events().to_vec();
    assert_eq!(sigs.len(), 1);
    app.test_set_signature_events(vec![
        sigs[0].clone(),
        SignatureEvent {
            bar: 32,
            numerator: 7,
            denominator: 8,
        },
    ]);
    app.test_rebuild_tempo_map();

    // Play from bar 40: the playback tick moves the display onto the
    // bar-33 tempo and meter.
    let bar_40 = app.test_tempo_map().bar_to_sample(39);
    app.test_set_transport_playing(true);
    app.test_apply_engine_event(AudioEvent::PlayheadMoved(bar_40));
    let _ = app.update(Message::Tick);
    assert_eq!(app.test_transport_bpm(), 140.0, "precondition: the display follows");
    assert_eq!(app.test_transport_time_sig(), (7, 8));

    let file = app.test_build_project_file();
    assert_eq!(file.bpm, 90.0, "saved bpm is the song's, not the playhead's");
    assert_eq!((file.time_sig_num, file.time_sig_den), (num0, den0));
    // A tempo-map rebuild here (any global-track edit) takes the song's
    // tempo and meter as its fallback, not the display's.
    app.test_rebuild_tempo_map();
    let map = app.test_tempo_map();
    assert_eq!(map.bpm, 90.0);
    assert_eq!((map.numerator, map.denominator), (num0, den0));
}
