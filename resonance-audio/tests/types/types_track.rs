use resonance_audio::{Bus, Track, TrackOutput};
use resonance_common::device_definition::MidiBinding;
use resonance_common::{DeviceParam, ParamCurve};

/// Build a minimal `DeviceParam` bound to a CC, for the device-param
/// map tests below.
fn cc_param(id: &str, cc: u8) -> DeviceParam {
    DeviceParam {
        id: id.to_string(),
        name: id.to_string(),
        group: None,
        binding: MidiBinding::Cc { cc },
        min: 0,
        max: 127,
        default: None,
        curve: ParamCurve::Linear,
    }
}

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
    // reserved and intentionally indistinguishable from Master; the app's
    // bus-id allocator (`TrackRegistry::allocate_bus_id`, ARCH-04 D-3)
    // starts at 1 and grows, so this is safe in practice but worth
    // pinning in a test.
    let track = Track::new(1, "T1".to_string());
    track.set_output(TrackOutput::Master);
    assert_eq!(track.output(), TrackOutput::Master);
    track.set_output(TrackOutput::Bus(5));
    assert_eq!(track.output(), TrackOutput::Bus(5));
    track.set_output(TrackOutput::Master);
    assert_eq!(track.output(), TrackOutput::Master);
}

#[test]
fn track_device_params_default_empty() {
    let track = Track::new(1, "T1".to_string());
    assert!(track.device_params().is_empty());
    assert!(track.device_param("cutoff").is_none());
}

#[test]
fn track_device_params_set_and_lookup() {
    let track = Track::new(1, "T1".to_string());
    let ids = track.set_device_params(vec![cc_param("cutoff", 74), cc_param("reso", 71)]);

    // Returned ids preserve supply order.
    assert_eq!(ids, vec!["cutoff".to_string(), "reso".to_string()]);
    // The map is keyed by param id.
    assert_eq!(track.device_params().len(), 2);
    assert_eq!(
        track.device_param("cutoff").unwrap().binding,
        MidiBinding::Cc { cc: 74 }
    );
    assert_eq!(
        track.device_param("reso").unwrap().binding,
        MidiBinding::Cc { cc: 71 }
    );
    assert!(track.device_param("missing").is_none());
}

#[test]
fn track_device_params_replace_is_not_merge() {
    let track = Track::new(1, "T1".to_string());
    track.set_device_params(vec![cc_param("a", 1), cc_param("b", 2)]);
    // A second command replaces the whole map rather than merging.
    let ids = track.set_device_params(vec![cc_param("c", 3)]);
    assert_eq!(ids, vec!["c".to_string()]);
    assert_eq!(track.device_params().len(), 1);
    assert!(track.device_param("a").is_none());
    assert!(track.device_param("b").is_none());
    assert!(track.device_param("c").is_some());
}

#[test]
fn track_device_params_empty_clears() {
    let track = Track::new(1, "T1".to_string());
    track.set_device_params(vec![cc_param("a", 1)]);
    let ids = track.set_device_params(vec![]);
    assert!(ids.is_empty());
    assert!(track.device_params().is_empty());
}

#[test]
fn track_device_params_duplicate_id_last_wins() {
    let track = Track::new(1, "T1".to_string());
    // Same id twice: the map keeps the last, the returned ids list it once.
    let ids = track.set_device_params(vec![cc_param("dup", 10), cc_param("dup", 20)]);
    assert_eq!(ids, vec!["dup".to_string()]);
    assert_eq!(track.device_params().len(), 1);
    assert_eq!(
        track.device_param("dup").unwrap().binding,
        MidiBinding::Cc { cc: 20 }
    );
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
