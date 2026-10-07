use resonance_music_theory::{VocalParams, VocalParamsError};

#[test]
fn default_validates() {
    assert!(VocalParams::default().validate().is_ok());
}

#[test]
fn lines_must_be_at_least_one() {
    let p = VocalParams {
        lines: 0,
        ..VocalParams::default()
    };
    assert_eq!(p.validate(), Err(VocalParamsError::LinesTooLow(0)));
}

#[test]
fn syllables_min_must_not_exceed_max() {
    let p = VocalParams {
        syllables_min: 10,
        syllables_max: 5,
        ..VocalParams::default()
    };
    assert_eq!(
        p.validate(),
        Err(VocalParamsError::SyllablesRange { min: 10, max: 5 })
    );
}

#[test]
fn syllables_min_must_be_positive() {
    let p = VocalParams {
        syllables_min: 0,
        syllables_max: 5,
        ..VocalParams::default()
    };
    assert_eq!(
        p.validate(),
        Err(VocalParamsError::SyllablesRange { min: 0, max: 5 })
    );
}

#[test]
fn range_lo_must_be_below_hi() {
    let p = VocalParams {
        range: (60, 60),
        ..VocalParams::default()
    };
    assert_eq!(
        p.validate(),
        Err(VocalParamsError::BadRange { lo: 60, hi: 60 })
    );
}

#[test]
fn out_of_range_unit_slider_is_rejected() {
    let p = VocalParams {
        breath: 1.5,
        ..VocalParams::default()
    };
    match p.validate() {
        Err(VocalParamsError::OutOfRange { field, value, lo, hi }) => {
            assert_eq!(field, "breath");
            assert!((value - 1.5).abs() < 1e-6);
            assert_eq!(lo, 0.0);
            assert_eq!(hi, 1.0);
        }
        other => panic!("expected OutOfRange, got {other:?}"),
    }
}

#[test]
fn tension_accepts_negative() {
    let mut p = VocalParams {
        tension: -0.5,
        ..VocalParams::default()
    };
    assert!(p.validate().is_ok());
    p.tension = -1.5;
    assert!(matches!(
        p.validate(),
        Err(VocalParamsError::OutOfRange { field: "tension", .. })
    ));
}

#[test]
fn vibrato_rate_lower_bound() {
    let p = VocalParams {
        vibrato_rate: 0.5,
        ..VocalParams::default()
    };
    assert!(matches!(
        p.validate(),
        Err(VocalParamsError::OutOfRange { field: "vibrato_rate", .. })
    ));
}

/// `rest_threshold_beats` is 0.125..=4 beats (default ½), and
/// `velocity_dynamics` 0..=1 (default 0.5) — spec §8, §9.
#[test]
fn rest_threshold_and_velocity_dynamics_ranges() {
    let d = VocalParams::default();
    assert_eq!(d.rest_threshold_beats, 0.5);
    assert_eq!(d.velocity_dynamics, 0.5);
    for (rest, ok) in [(0.125, true), (4.0, true), (0.1, false), (4.01, false), (f32::NAN, false)] {
        let p = VocalParams {
            rest_threshold_beats: rest,
            ..VocalParams::default()
        };
        assert_eq!(p.validate().is_ok(), ok, "rest_threshold_beats {rest}");
        if !ok {
            assert!(matches!(
                p.validate(),
                Err(VocalParamsError::OutOfRange { field: "rest_threshold_beats", .. })
            ));
        }
    }
    for (amount, ok) in [(0.0, true), (1.0, true), (-0.01, false), (1.01, false)] {
        let p = VocalParams {
            velocity_dynamics: amount,
            ..VocalParams::default()
        };
        assert_eq!(p.validate().is_ok(), ok, "velocity_dynamics {amount}");
        if !ok {
            assert!(matches!(
                p.validate(),
                Err(VocalParamsError::OutOfRange { field: "velocity_dynamics", .. })
            ));
        }
    }
}

/// A project written before the two fields existed reads them at their
/// defaults; written values round-trip.
#[test]
fn rest_threshold_and_velocity_dynamics_serde() {
    let mut json = serde_json::to_value(VocalParams::default()).unwrap();
    let obj = json.as_object_mut().unwrap();
    obj.remove("rest_threshold_beats").expect("written");
    obj.remove("velocity_dynamics").expect("written");
    let old: VocalParams = serde_json::from_value(json).unwrap();
    assert_eq!(old.rest_threshold_beats, 0.5);
    assert_eq!(old.velocity_dynamics, 0.5);

    let p = VocalParams {
        rest_threshold_beats: 1.25,
        velocity_dynamics: 0.9,
        ..VocalParams::default()
    };
    let back: VocalParams = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(back, p);
}
