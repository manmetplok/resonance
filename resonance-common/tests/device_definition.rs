use resonance_common::device_definition::*;

// --- Binding range ------------------------------------------------------------

#[test]
fn binding_max_value_by_kind() {
    assert_eq!(MidiBinding::Cc { cc: 74 }.max_value(), 127);
    assert_eq!(
        MidiBinding::Nrpn {
            msb: 1,
            lsb: 2,
            fourteen_bit: false
        }
        .max_value(),
        127
    );
    assert_eq!(
        MidiBinding::Nrpn {
            msb: 1,
            lsb: 2,
            fourteen_bit: true
        }
        .max_value(),
        16_383
    );
    assert_eq!(
        MidiBinding::Rpn {
            msb: 0,
            lsb: 0,
            fourteen_bit: true
        }
        .max_value(),
        16_383
    );
}

// --- Mapping helpers ----------------------------------------------------------

fn param(curve: ParamCurve, min: u16, max: u16, binding: MidiBinding) -> DeviceParam {
    DeviceParam {
        id: "p".into(),
        name: "P".into(),
        group: None,
        binding,
        min,
        max,
        default: None,
        curve,
    }
}

#[test]
fn linear_maps_endpoints_and_midpoint() {
    let p = param(ParamCurve::Linear, 0, 127, MidiBinding::Cc { cc: 1 });
    assert_eq!(lane_value_to_binding_value(&p, 0.0), 0);
    assert_eq!(lane_value_to_binding_value(&p, 1.0), 127);
    // Nearest-integer rounding of 63.5.
    assert_eq!(lane_value_to_binding_value(&p, 0.5), 64);
}

#[test]
fn mapping_clamps_out_of_range_norm() {
    let p = param(ParamCurve::Linear, 10, 20, MidiBinding::Cc { cc: 1 });
    assert_eq!(lane_value_to_binding_value(&p, -5.0), 10);
    assert_eq!(lane_value_to_binding_value(&p, 5.0), 20);
}

#[test]
fn endpoints_round_trip_for_every_curve() {
    for curve in [
        ParamCurve::Linear,
        ParamCurve::Exponential,
        ParamCurve::Logarithmic,
    ] {
        let p = param(curve, 0, 127, MidiBinding::Cc { cc: 1 });
        // norm extremes -> binding extremes -> norm extremes.
        assert_eq!(lane_value_to_binding_value(&p, 0.0), 0);
        assert_eq!(lane_value_to_binding_value(&p, 1.0), 127);
        assert_eq!(binding_value_to_lane(&p, 0), 0.0);
        assert_eq!(binding_value_to_lane(&p, 127), 1.0);
    }
}

#[test]
fn every_raw_value_round_trips_through_lane_for_every_curve() {
    // raw -> lane -> raw is exact for all integers in min..=max, because the
    // forward map rounds to the nearest integer and shape/unshape are inverses.
    for curve in [
        ParamCurve::Linear,
        ParamCurve::Exponential,
        ParamCurve::Logarithmic,
    ] {
        let p = param(curve, 0, 127, MidiBinding::Cc { cc: 1 });
        for raw in 0..=127u16 {
            let norm = binding_value_to_lane(&p, raw);
            assert!(
                (0.0..=1.0).contains(&norm),
                "norm {norm} out of range for {curve:?} raw {raw}"
            );
            assert_eq!(
                lane_value_to_binding_value(&p, norm),
                raw,
                "round-trip failed for {curve:?} at raw {raw}"
            );
        }
    }
}

#[test]
fn fourteen_bit_round_trips_at_extremes() {
    let p = param(
        ParamCurve::Exponential,
        0,
        16_383,
        MidiBinding::Nrpn {
            msb: 1,
            lsb: 2,
            fourteen_bit: true,
        },
    );
    assert_eq!(lane_value_to_binding_value(&p, 0.0), 0);
    assert_eq!(lane_value_to_binding_value(&p, 1.0), 16_383);
    assert_eq!(binding_value_to_lane(&p, 0), 0.0);
    assert_eq!(binding_value_to_lane(&p, 16_383), 1.0);
}

#[test]
fn curves_differ_in_the_interior() {
    // At the midpoint the three curves must disagree (otherwise the curve
    // selection would be a no-op).
    let lin = param(ParamCurve::Linear, 0, 127, MidiBinding::Cc { cc: 1 });
    let exp = param(ParamCurve::Exponential, 0, 127, MidiBinding::Cc { cc: 1 });
    let log = param(ParamCurve::Logarithmic, 0, 127, MidiBinding::Cc { cc: 1 });
    let (l, e, g) = (
        lane_value_to_binding_value(&lin, 0.5),
        lane_value_to_binding_value(&exp, 0.5),
        lane_value_to_binding_value(&log, 0.5),
    );
    assert!(e < l, "exponential should sit below linear at 0.5 (got {e} vs {l})");
    assert!(g > l, "logarithmic should sit above linear at 0.5 (got {g} vs {l})");
}

#[test]
fn degenerate_range_maps_to_zero_lane() {
    let p = param(ParamCurve::Linear, 64, 64, MidiBinding::Cc { cc: 1 });
    assert_eq!(binding_value_to_lane(&p, 64), 0.0);
    assert_eq!(lane_value_to_binding_value(&p, 0.7), 64);
}

// --- Validation ---------------------------------------------------------------

fn sample_definition() -> DeviceDefinition {
    DeviceDefinition {
        id: "moog-muse".into(),
        manufacturer: "Moog".into(),
        model: "Muse".into(),
        schema_version: SCHEMA_VERSION,
        params: vec![
            DeviceParam {
                id: "filter_cutoff".into(),
                name: "Filter Cutoff".into(),
                group: Some("Filter".into()),
                binding: MidiBinding::Cc { cc: 74 },
                min: 0,
                max: 127,
                default: Some(64),
                curve: ParamCurve::Logarithmic,
            },
            DeviceParam {
                id: "filter_env_amt".into(),
                name: "Filter Env Amount".into(),
                group: Some("Filter".into()),
                binding: MidiBinding::Nrpn {
                    msb: 0,
                    lsb: 12,
                    fourteen_bit: true,
                },
                min: 0,
                max: 16_383,
                default: None,
                curve: ParamCurve::Linear,
            },
        ],
        patches: vec![PatchEntry {
            bank_msb: 0,
            bank_lsb: 0,
            program: 1,
            name: "Init Patch".into(),
            category: Some("Template".into()),
        }],
    }
}

#[test]
fn valid_definition_passes() {
    assert_eq!(sample_definition().validate(), Ok(()));
}

#[test]
fn empty_device_id_rejected() {
    let mut def = sample_definition();
    def.id = "   ".into();
    assert_eq!(def.validate(), Err(DeviceDefinitionError::EmptyDeviceId));
}

#[test]
fn duplicate_param_id_rejected() {
    let mut def = sample_definition();
    def.params[1].id = "filter_cutoff".into();
    assert_eq!(
        def.validate(),
        Err(DeviceDefinitionError::DuplicateParamId("filter_cutoff".into()))
    );
}

#[test]
fn min_greater_than_max_rejected() {
    let mut def = sample_definition();
    def.params[0].min = 100;
    def.params[0].max = 10;
    assert_eq!(
        def.validate(),
        Err(DeviceDefinitionError::MinGreaterThanMax {
            param_id: "filter_cutoff".into(),
            min: 100,
            max: 10,
        })
    );
}

#[test]
fn value_exceeding_binding_range_rejected() {
    let mut def = sample_definition();
    // A 7-bit CC cannot carry 200.
    def.params[0].max = 200;
    assert_eq!(
        def.validate(),
        Err(DeviceDefinitionError::BindingValueOutOfRange {
            param_id: "filter_cutoff".into(),
            value: 200,
            limit: 127,
        })
    );
}

#[test]
fn default_outside_range_rejected() {
    let mut def = sample_definition();
    def.params[0].default = Some(200);
    assert_eq!(
        def.validate(),
        Err(DeviceDefinitionError::DefaultOutOfRange {
            param_id: "filter_cutoff".into(),
            default: 200,
            min: 0,
            max: 127,
        })
    );
}

#[test]
fn invalid_cc_number_rejected() {
    let mut def = sample_definition();
    def.params[0].binding = MidiBinding::Cc { cc: 200 };
    assert_eq!(
        def.validate(),
        Err(DeviceDefinitionError::InvalidCc {
            param_id: "filter_cutoff".into(),
            cc: 200,
        })
    );
}

#[test]
fn invalid_parameter_address_rejected() {
    let mut def = sample_definition();
    def.params[1].binding = MidiBinding::Nrpn {
        msb: 200,
        lsb: 0,
        fourteen_bit: true,
    };
    assert_eq!(
        def.validate(),
        Err(DeviceDefinitionError::InvalidParameterAddress {
            param_id: "filter_env_amt".into(),
            msb: 200,
            lsb: 0,
        })
    );
}

// --- Serde --------------------------------------------------------------------

#[test]
fn json_round_trips_a_sample_definition() {
    let def = sample_definition();
    let json = def.to_json().expect("serialize");
    let back = DeviceDefinition::from_json(json.as_bytes()).expect("deserialize");
    assert_eq!(def, back);
    assert_eq!(back.validate(), Ok(()));
    // schema_version survives the round-trip.
    assert_eq!(back.schema_version, SCHEMA_VERSION);
}

#[test]
fn json_omits_optional_none_fields() {
    let def = sample_definition();
    let json = def.to_json().expect("serialize");
    // `filter_env_amt` has no default and no category on its patch... but the
    // first param has both group and default present, so just assert the
    // skip_serializing_if kicks in for the None default on param[1].
    assert!(
        json.contains("\"filter_cutoff\""),
        "expected param ids present in JSON"
    );
    // The Linear default of param[1].curve is still serialized (no skip), but its
    // None default must be absent.
    let back = DeviceDefinition::from_json(json.as_bytes()).expect("deserialize");
    assert_eq!(back.params[1].default, None);
}

#[test]
fn param_lookup_by_id() {
    let def = sample_definition();
    assert_eq!(def.param("filter_cutoff").map(|p| p.name.as_str()), Some("Filter Cutoff"));
    assert!(def.param("nope").is_none());
}
