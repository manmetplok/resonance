//! Aux-send / return-bus data model: the `AuxSend` value type and the
//! `Bus::is_return` role flag. Pure data, no engine required.

use resonance_audio::{AuxSend, Bus, SendSource};

#[test]
fn aux_send_holds_its_fields() {
    let send = AuxSend {
        id: 7,
        source: SendSource::Track(3),
        dest: 12,
        level_db: -6.0,
        pre_fader: true,
        enabled: false,
    };
    assert_eq!(send.id, 7);
    assert_eq!(send.source, SendSource::Track(3));
    assert_eq!(send.dest, 12);
    assert_eq!(send.level_db, -6.0);
    assert!(send.pre_fader);
    assert!(!send.enabled);
}

#[test]
fn send_source_distinguishes_track_and_bus_with_same_id() {
    // A track and a bus can share a numeric id; the source enum keeps
    // them apart so a Track(5)->Bus(5) send is never a self-route.
    assert_ne!(SendSource::Track(5), SendSource::Bus(5));
}

#[test]
fn bus_defaults_to_non_return() {
    let bus = Bus::new(1, "Reverb".to_string());
    assert!(!bus.is_return());
}

#[test]
fn bus_return_role_roundtrips() {
    let bus = Bus::new(1, "Reverb".to_string());
    bus.set_is_return(true);
    assert!(bus.is_return());
    bus.set_is_return(false);
    assert!(!bus.is_return());
}

#[test]
fn bus_return_role_is_independent_of_fx_bypass() {
    // The role flag must not alias the existing fx-bypass atomic.
    let bus = Bus::new(1, "Bus".to_string());
    bus.set_is_return(true);
    bus.set_fx_bypassed(true);
    assert!(bus.is_return());
    assert!(bus.fx_bypassed());
    bus.set_fx_bypassed(false);
    assert!(bus.is_return());
    assert!(!bus.fx_bypassed());
}
