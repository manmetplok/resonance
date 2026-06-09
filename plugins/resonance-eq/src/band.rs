//! Per-band types: BandKind, BandSlope, and the coefficient-dispatch helper
//! that turns a `BandSnapshot` into a cascade of up to four biquads.

use resonance_dsp::Biquad;

use crate::params::BandSnapshot;

/// Filter mode for a single EQ band.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BandKind {
    Bell,
    LowShelf,
    HighShelf,
    LowCut,
    HighCut,
}

impl BandKind {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => BandKind::LowShelf,
            2 => BandKind::HighShelf,
            3 => BandKind::LowCut,
            4 => BandKind::HighCut,
            _ => BandKind::Bell,
        }
    }

    pub fn to_index(self) -> i32 {
        match self {
            BandKind::Bell => 0,
            BandKind::LowShelf => 1,
            BandKind::HighShelf => 2,
            BandKind::LowCut => 3,
            BandKind::HighCut => 4,
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            BandKind::Bell => "Bell",
            BandKind::LowShelf => "LShelf",
            BandKind::HighShelf => "HShelf",
            BandKind::LowCut => "LCut",
            BandKind::HighCut => "HCut",
        }
    }

    pub fn is_cut(self) -> bool {
        matches!(self, BandKind::LowCut | BandKind::HighCut)
    }

    pub fn uses_gain(self) -> bool {
        matches!(
            self,
            BandKind::Bell | BandKind::LowShelf | BandKind::HighShelf
        )
    }
}

/// Slope selection for cut bands. 12 dB/oct = 1 biquad, 24 = 2, 48 = 4.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BandSlope {
    Db12,
    Db24,
    Db48,
}

impl BandSlope {
    pub fn from_index(i: i32) -> Self {
        match i {
            0 => BandSlope::Db12,
            2 => BandSlope::Db48,
            _ => BandSlope::Db24,
        }
    }

    pub fn to_index(self) -> i32 {
        match self {
            BandSlope::Db12 => 0,
            BandSlope::Db24 => 1,
            BandSlope::Db48 => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BandSlope::Db12 => "12 dB/oct",
            BandSlope::Db24 => "24 dB/oct",
            BandSlope::Db48 => "48 dB/oct",
        }
    }

    /// Number of cascaded 2nd-order sections required to realise this slope.
    pub fn num_stages(self) -> usize {
        self.stage_qs().len()
    }

    /// Per-stage Q values realising a true Butterworth response for this
    /// slope. These are the Butterworth pole-pair Qs
    /// `1 / (2 cos(θ_k))` for the order-2N pole angles, so the cascade is
    /// maximally flat and sits exactly -3 dB at the cutoff. (A uniform
    /// Q=0.707 cascade — the old behaviour — sagged to -6 dB at cutoff
    /// for 24 dB/oct and -12 dB for 48 dB/oct.)
    pub fn stage_qs(self) -> &'static [f32] {
        const Q_12: [f32; 1] = [std::f32::consts::FRAC_1_SQRT_2];
        const Q_24: [f32; 2] = [0.541_20, 1.306_56];
        const Q_48: [f32; 4] = [0.509_80, 0.601_34, 0.899_98, 2.562_92];
        match self {
            BandSlope::Db12 => &Q_12,
            BandSlope::Db24 => &Q_24,
            BandSlope::Db48 => &Q_48,
        }
    }
}

/// Number of biquad stages per band. 4 is enough for the steepest 48 dB/oct
/// cut; bell/shelf bands use only stage 0 and leave the rest as identity.
pub const MAX_STAGES_PER_BAND: usize = 4;

/// Apply a `BandSnapshot` to an array of biquad stages — writes only the
/// coefficients (leaves the z1/z2 state intact so the filter keeps running
/// smoothly across parameter changes). Returns the number of active stages.
pub fn configure_stages(
    snapshot: &BandSnapshot,
    sr: f32,
    stages: &mut [Biquad; MAX_STAGES_PER_BAND],
) -> usize {
    if !snapshot.enabled {
        for s in stages.iter_mut() {
            assign_identity(s);
        }
        return 0;
    }

    match snapshot.kind {
        BandKind::Bell => {
            let mut coeffs = Biquad::identity();
            coeffs.set_bell(sr, snapshot.freq, snapshot.q, snapshot.gain_db);
            assign_coeffs(&mut stages[0], &coeffs);
            for s in stages.iter_mut().skip(1) {
                assign_identity(s);
            }
            1
        }
        BandKind::LowShelf => {
            let mut coeffs = Biquad::identity();
            coeffs.set_low_shelf(sr, snapshot.freq, snapshot.q, snapshot.gain_db);
            assign_coeffs(&mut stages[0], &coeffs);
            for s in stages.iter_mut().skip(1) {
                assign_identity(s);
            }
            1
        }
        BandKind::HighShelf => {
            let mut coeffs = Biquad::identity();
            coeffs.set_high_shelf(sr, snapshot.freq, snapshot.q, snapshot.gain_db);
            assign_coeffs(&mut stages[0], &coeffs);
            for s in stages.iter_mut().skip(1) {
                assign_identity(s);
            }
            1
        }
        BandKind::LowCut => {
            // True Butterworth cascade: each 2nd-order section gets its own
            // pole-pair Q so the composite response is maximally flat and
            // crosses exactly -3 dB at the cutoff frequency.
            let qs = snapshot.slope.stage_qs();
            for (stage, &q) in stages.iter_mut().zip(qs.iter()) {
                let mut coeffs = Biquad::identity();
                coeffs.set_high_pass(sr, snapshot.freq, q);
                assign_coeffs(stage, &coeffs);
            }
            for s in stages.iter_mut().skip(qs.len()) {
                assign_identity(s);
            }
            qs.len()
        }
        BandKind::HighCut => {
            let qs = snapshot.slope.stage_qs();
            for (stage, &q) in stages.iter_mut().zip(qs.iter()) {
                let mut coeffs = Biquad::identity();
                coeffs.set_low_pass(sr, snapshot.freq, q);
                assign_coeffs(stage, &coeffs);
            }
            for s in stages.iter_mut().skip(qs.len()) {
                assign_identity(s);
            }
            qs.len()
        }
    }
}

/// Copy the 5 normalised coefficients from `src` into `dst`, preserving
/// `dst`'s internal delay-line state.
fn assign_coeffs(dst: &mut Biquad, src: &Biquad) {
    dst.b0 = src.b0;
    dst.b1 = src.b1;
    dst.b2 = src.b2;
    dst.a1 = src.a1;
    dst.a2 = src.a2;
}

fn assign_identity(dst: &mut Biquad) {
    dst.b0 = 1.0;
    dst.b1 = 0.0;
    dst.b2 = 0.0;
    dst.a1 = 0.0;
    dst.a2 = 0.0;
}
