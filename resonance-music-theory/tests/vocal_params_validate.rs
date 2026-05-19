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
