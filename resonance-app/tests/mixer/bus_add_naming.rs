//! The GUI "+ Bus" button names its bus "Bus N" — the smallest free N,
//! the same rule as `bus.create` with no name — never by the raw id
//! (BUG-01: "Bus 2000000000").

use resonance_app::message::{BusMessage, Message};
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent};

#[test]
fn gui_bus_add_names_the_bus_by_count_not_by_id() {
    let (mut app, _task) = Resonance::new_for_test();
    let rx = app.test_capture_engine();
    let mut names = Vec::new();
    for _ in 0..2 {
        app.test_dispatch(Message::Bus(BusMessage::AddBus));
        let (id, name) = std::iter::from_fn(|| rx.try_recv().ok())
            .find_map(|c| match c {
                AudioCommand::AddBus { id, name } => Some((id, name)),
                _ => None,
            })
            .expect("AddBus sent");
        let name = name.expect("the app names the bus; the engine fallback is `Bus {id}`");
        assert!(!name.contains(&id.to_string()), "bus named by its raw id: {name:?}");
        app.test_apply_engine_event(AudioEvent::BusAdded { bus_id: id, name: name.clone() });
        names.push(name);
    }
    assert_ne!(names[0], names[1], "two adds get distinct names");
    assert!(names.iter().all(|n| n.starts_with("Bus ")), "{names:?}");
}
