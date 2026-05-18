use resonance_dsp::{BandType, Biquad};

#[test]
fn from_to_index_round_trip() {
    for &bt in &[
        BandType::Bell,
        BandType::LowShelf,
        BandType::HighShelf,
        BandType::HighPass,
        BandType::LowPass,
    ] {
        assert_eq!(BandType::from_index(bt.to_index()), bt);
    }
}

#[test]
fn unknown_index_falls_back_to_bell() {
    assert_eq!(BandType::from_index(-1), BandType::Bell);
    assert_eq!(BandType::from_index(99), BandType::Bell);
}

#[test]
fn bell_biquad_matches_manual_set_bell() {
    let sr = 48_000.0;
    let freq = 1_000.0;
    let q = 1.0;
    let gain_db = 6.0;

    let via_enum = BandType::Bell.to_biquad(sr, freq, q, gain_db);
    let mut manual = Biquad::identity();
    manual.set_bell(sr, freq, q, gain_db);
    assert_eq!(via_enum.b0, manual.b0);
    assert_eq!(via_enum.b1, manual.b1);
    assert_eq!(via_enum.a2, manual.a2);
}
