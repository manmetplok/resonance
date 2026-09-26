//! Dragging a tempo point keeps the list sorted and the selection pointing
//! at the dragged event (code review VIEW-15).
//!
//! The selection is an index. `EndTempoDrag` used to sort the events
//! without remapping it, so after dragging a point past its neighbour,
//! Delete removed the neighbour; and a drag onto an occupied bar stacked
//! two events on one bar.

use resonance_app::message::{GlobalTrackMessage, Message};
use resonance_app::Resonance;
use resonance_audio::types::TempoPoint;

fn app_with_tempo_points(bars: &[u32]) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_flat_tempo(120.0);
    for &bar in bars.iter().filter(|&&b| b != 0) {
        app.test_push_tempo_event(TempoPoint { bar, bpm: 120.0 + bar as f32 });
    }
    app.test_rebuild_tempo_map();
    app
}

fn drag(app: &mut Resonance, index: usize, to_bar: u32) {
    let bpm = app.test_tempo_events()[index].bpm;
    for m in [
        GlobalTrackMessage::StartTempoDrag(index),
        GlobalTrackMessage::UpdateTempoEvent {
            index,
            bar: to_bar,
            bpm,
        },
        GlobalTrackMessage::EndTempoDrag,
    ] {
        let _ = app.update(Message::GlobalTrack(m));
    }
}

fn bars(app: &Resonance) -> Vec<u32> {
    app.test_tempo_events().iter().map(|e| e.bar).collect()
}

#[test]
fn delete_after_drag_removes_the_dragged_event() {
    let mut app = app_with_tempo_points(&[0, 4, 8]);
    drag(&mut app, 1, 12);
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::DeleteSelectedEvent));
    assert_eq!(bars(&app), vec![0, 8]);
}

#[test]
fn a_drag_never_stacks_two_events_on_one_bar() {
    let mut app = app_with_tempo_points(&[0, 4, 8]);
    drag(&mut app, 1, 8);
    let list = bars(&app);
    let mut dedup = list.clone();
    dedup.dedup();
    assert_eq!(list, dedup, "two tempo events on one bar");
    assert!(list.windows(2).all(|w| w[0] < w[1]), "tempo events unsorted: {list:?}");
}

#[test]
fn the_list_stays_sorted_during_the_drag() {
    let mut app = app_with_tempo_points(&[0, 4, 8]);
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::StartTempoDrag(1)));
    let _ = app.update(Message::GlobalTrack(GlobalTrackMessage::UpdateTempoEvent {
        index: 1,
        bar: 12,
        bpm: 124.0,
    }));
    let list = bars(&app);
    assert!(list.windows(2).all(|w| w[0] < w[1]), "unsorted mid-drag: {list:?}");
}
