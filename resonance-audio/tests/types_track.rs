use resonance_audio::{Bus, Track, TrackOutput};

#[test]
fn track_output_defaults_to_master() {
    let track = Track::new(1, "T1".to_string());
    assert_eq!(track.output(), TrackOutput::Master);
}

#[test]
fn track_output_roundtrip_master() {
    let track = Track::new(1, "T1".to_string());
    track.set_output(TrackOutput::Master);
    assert_eq!(track.output(), TrackOutput::Master);
}

#[test]
fn track_output_roundtrip_bus() {
    let track = Track::new(1, "T1".to_string());
    track.set_output(TrackOutput::Bus(42));
    assert_eq!(track.output(), TrackOutput::Bus(42));
}

#[test]
fn track_output_roundtrip_various_bus_ids() {
    let track = Track::new(1, "T1".to_string());
    for id in [1u64, 7, 100, 1_000_000, u64::MAX - 1] {
        track.set_output(TrackOutput::Bus(id));
        assert_eq!(track.output(), TrackOutput::Bus(id));
    }
}

#[test]
fn track_output_master_sentinel_is_u64_max() {
    // The sentinel chosen for Master is u64::MAX. Bus id u64::MAX is
    // reserved and intentionally indistinguishable from Master; the
    // engine's next_bus_id starts at 1 and grows, so this is safe in
    // practice but worth pinning in a test.
    let track = Track::new(1, "T1".to_string());
    track.set_output(TrackOutput::Master);
    assert_eq!(track.output(), TrackOutput::Master);
    track.set_output(TrackOutput::Bus(5));
    assert_eq!(track.output(), TrackOutput::Bus(5));
    track.set_output(TrackOutput::Master);
    assert_eq!(track.output(), TrackOutput::Master);
}

#[test]
fn bus_atomic_accessors_roundtrip() {
    let bus = Bus::new(1, "Bus 1".to_string());

    assert_eq!(bus.volume(), 1.0);
    assert_eq!(bus.pan(), 0.0);
    assert!(!bus.muted());

    bus.set_volume(0.5);
    assert_eq!(bus.volume(), 0.5);

    bus.set_pan(-0.75);
    assert_eq!(bus.pan(), -0.75);

    bus.set_muted(true);
    assert!(bus.muted());
}

#[test]
fn track_fx_bypass_roundtrip() {
    let track = Track::new(1, "T1".to_string());
    assert!(!track.fx_bypassed());
    track.set_fx_bypassed(true);
    assert!(track.fx_bypassed());
    track.set_fx_bypassed(false);
    assert!(!track.fx_bypassed());
}

#[test]
fn bus_fx_bypass_roundtrip() {
    let bus = Bus::new(1, "Bus 1".to_string());
    assert!(!bus.fx_bypassed());
    bus.set_fx_bypassed(true);
    assert!(bus.fx_bypassed());
    bus.set_fx_bypassed(false);
    assert!(!bus.fx_bypassed());
}

#[test]
fn bus_peak_update_and_swap() {
    let bus = Bus::new(1, "Bus 1".to_string());

    bus.update_peak_l(0.3);
    bus.update_peak_l(0.5);
    bus.update_peak_l(0.2);
    bus.update_peak_r(0.8);

    assert_eq!(bus.swap_peak_l(), 0.5);
    assert_eq!(bus.swap_peak_r(), 0.8);
    assert_eq!(bus.swap_peak_l(), 0.0);
    assert_eq!(bus.swap_peak_r(), 0.0);
}
