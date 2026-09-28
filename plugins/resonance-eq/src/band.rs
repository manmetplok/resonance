//! Per-band types: BandKind, BandSlope, BandMs, and the coefficient-dispatch
//! helper that turns a `BandSnapshot` into a cascade of up to four biquads.
//!
//! The three "one knob" kinds (warmth-width-depth.md §6.4) are built from
//! the same sections as the rest:
//!
//! - **Tilt** — a first-order tilt around the band frequency (the pivot):
//!   `gain` dB at the top, `-gain` dB at the bottom, 0 dB at the pivot,
//!   one section, no Q.
//! - **LF Lift+Dip** — the passive-EQ trick of boosting and cutting the
//!   same low band at once: a low shelf with its midpoint at the band
//!   frequency (the lift, `gain` dB) plus a bell at 3.5 × that frequency
//!   cutting half as much (the dip). At 60–100 Hz that puts the dip at
//!   210–350 Hz, where the mud is. Q is fixed by the voicing.
//! - **Air** — a very broad first-order high shelf with its midpoint at
//!   the band frequency. Its pole sits at `freq × √G`, so at the top of
//!   the range the corner lands well above the audible band (40 kHz for
//!   +12 dB at 20 kHz) and only the gentle skirt is heard. The design is
//!   a prewarped bilinear first-order section, which is stable for any
//!   corner at any sample rate and needs no special case at Nyquist.

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
    /// First-order tilt around the band frequency.
    Tilt,
    /// Low shelf lift plus a bell dip 3.5× above it, one gain knob.
    LfLiftDip,
    /// Very broad first-order high shelf.
    Air,
}

/// Host-facing labels of the kind parameter, by index. Indices 0–4 are the
/// original kinds and must keep their meaning: saved projects store them.
pub const KIND_LABELS: &[&str] = &[
    "Bell",
    "Low Shelf",
    "High Shelf",
    "Low Cut",
    "High Cut",
    "Tilt",
    "LF Lift+Dip",
    "Air",
];

impl BandKind {
    /// Every kind, in index order.
    pub const ALL: [BandKind; 8] = [
        BandKind::Bell,
        BandKind::LowShelf,
        BandKind::HighShelf,
        BandKind::LowCut,
        BandKind::HighCut,
        BandKind::Tilt,
        BandKind::LfLiftDip,
        BandKind::Air,
    ];

    pub fn from_index(i: i32) -> Self {
        match i {
            1 => BandKind::LowShelf,
            2 => BandKind::HighShelf,
            3 => BandKind::LowCut,
            4 => BandKind::HighCut,
            5 => BandKind::Tilt,
            6 => BandKind::LfLiftDip,
            7 => BandKind::Air,
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
            BandKind::Tilt => 5,
            BandKind::LfLiftDip => 6,
            BandKind::Air => 7,
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            BandKind::Bell => "Bell",
            BandKind::LowShelf => "LShelf",
            BandKind::HighShelf => "HShelf",
            BandKind::LowCut => "LCut",
            BandKind::HighCut => "HCut",
            BandKind::Tilt => "Tilt",
            BandKind::LfLiftDip => "LF Lift",
            BandKind::Air => "Air",
        }
    }

    pub fn is_cut(self) -> bool {
        matches!(self, BandKind::LowCut | BandKind::HighCut)
    }

    pub fn uses_gain(self) -> bool {
        !self.is_cut()
    }

    /// Whether the band's Q parameter shapes this kind. The one-knob
    /// kinds fix their own shape.
    pub fn uses_q(self) -> bool {
        !matches!(self, BandKind::Tilt | BandKind::LfLiftDip | BandKind::Air)
    }

    /// Whether a dynamic band (`dyn_on`) acts on this kind: the bell, the
    /// shelves and Air, whose gain is a single cut or boost a dynamic
    /// reduction can pull down.
    ///
    /// Not the cuts, which have no gain, and not Tilt or LF Lift+Dip:
    /// their gain knob drives two opposite moves at once, so lowering it
    /// raises the other side — a tilt's bottom end, a lift's dip — and a
    /// "cut" would boost by up to the full reduction. On these kinds the
    /// DSP ignores every dyn param and the editor greys the controls.
    pub fn supports_dyn(self) -> bool {
        matches!(
            self,
            BandKind::Bell | BandKind::LowShelf | BandKind::HighShelf | BandKind::Air
        )
    }
}

/// Which part of the stereo signal a band filters.
///
/// `Stereo` filters left and right as before. `Mid` filters only
/// `(L + R) / 2` and `Side` only `(L - R) / 2`, the other component
/// passing untouched — so a Side band leaves the mono sum alone and a Mid
/// band leaves the stereo difference alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BandMs {
    Stereo,
    Mid,
    Side,
}

/// Host-facing labels of the per-band M/S parameter, by index.
pub const MS_LABELS: &[&str] = &["Stereo", "Mid", "Side"];

impl BandMs {
    pub const ALL: [BandMs; 3] = [BandMs::Stereo, BandMs::Mid, BandMs::Side];

    pub fn from_index(i: i32) -> Self {
        match i {
            1 => BandMs::Mid,
            2 => BandMs::Side,
            _ => BandMs::Stereo,
        }
    }

    pub fn to_index(self) -> i32 {
        match self {
            BandMs::Stereo => 0,
            BandMs::Mid => 1,
            BandMs::Side => 2,
        }
    }

    pub fn label(self) -> &'static str {
        MS_LABELS[self.to_index() as usize]
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

/// Where the LF Lift+Dip dip sits, as a multiple of the lift frequency.
pub const LF_DIP_RATIO: f32 = 3.5;
/// How deep the dip cuts, as a fraction of the lift's gain.
pub const LF_DIP_DEPTH: f32 = 0.5;
/// Q of the lift's low shelf. A touch above Butterworth gives the slight
/// bump before the shelf plateau that the passive original has.
const LF_LIFT_Q: f32 = 0.9;
/// Q of the dip's bell.
const LF_DIP_Q: f32 = 1.0;

/// Coefficients of a first-order tilt: `gain_db` at the top, `-gain_db`
/// at the bottom, unity at `pivot`.
///
/// `H(s) = A (s + w/A) / (s + wA)` with `A = 10^(gain/20)` and
/// `w = 2π·pivot`: DC gain `1/A`, HF gain `A`, and `|H(jw)| = 1`. The
/// bilinear transform is prewarped at the pivot so the digital curve
/// crosses 0 dB exactly there.
pub fn tilt_section(sr: f32, pivot: f32, gain_db: f32) -> Biquad {
    let a = 10f32.powf(gain_db / 20.0);
    let pivot = pivot.clamp(10.0, sr * 0.45);
    let w = 2.0 * std::f32::consts::PI * pivot;
    let mut b = Biquad::identity();
    b.set_first_order_analog(sr, a, w, 1.0, w * a, pivot);
    b
}

/// Coefficients of the Air band's first-order high shelf: unity at DC,
/// `gain_db` at the top, half of it (in dB) at `midpoint`.
///
/// `H(s) = G (s + w/√G) / (s + w√G)`. The analog pole is at
/// `midpoint × √G`, which may well be above Nyquist — the bilinear
/// transform still puts it inside the unit circle. The prewarp point is
/// the midpoint, or a quarter of the sample rate if that is lower, so the
/// digital curve tracks the analog one through the audible band and only
/// compresses the last octave below Nyquist (where it reaches `G`).
pub fn air_section(sr: f32, midpoint: f32, gain_db: f32) -> Biquad {
    let g = 10f32.powf(gain_db / 20.0);
    let rg = g.sqrt();
    let midpoint = midpoint.max(10.0);
    let w = 2.0 * std::f32::consts::PI * midpoint;
    let mut b = Biquad::identity();
    b.set_first_order_analog(sr, g, g * w / rg, 1.0, w * rg, midpoint.min(sr * 0.25));
    b
}

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
        BandKind::Tilt => {
            let coeffs = tilt_section(sr, snapshot.freq, snapshot.gain_db);
            assign_coeffs(&mut stages[0], &coeffs);
            for s in stages.iter_mut().skip(1) {
                assign_identity(s);
            }
            1
        }
        BandKind::LfLiftDip => {
            let mut lift = Biquad::identity();
            lift.set_low_shelf(sr, snapshot.freq, LF_LIFT_Q, snapshot.gain_db);
            let mut dip = Biquad::identity();
            let dip_hz = (snapshot.freq * LF_DIP_RATIO).min(sr * 0.45);
            dip.set_bell(sr, dip_hz, LF_DIP_Q, -snapshot.gain_db * LF_DIP_DEPTH);
            assign_coeffs(&mut stages[0], &lift);
            assign_coeffs(&mut stages[1], &dip);
            for s in stages.iter_mut().skip(2) {
                assign_identity(s);
            }
            2
        }
        BandKind::Air => {
            let coeffs = air_section(sr, snapshot.freq, snapshot.gain_db);
            assign_coeffs(&mut stages[0], &coeffs);
            for s in stages.iter_mut().skip(1) {
                assign_identity(s);
            }
            1
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
