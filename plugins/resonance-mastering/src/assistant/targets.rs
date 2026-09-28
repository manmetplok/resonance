//! Built-in tonal-balance target **bands** per genre
//! (warmth-width-depth.md §7.4, decision D6).
//!
//! A target is not one curve but a band: a minimum and a maximum level per
//! ISO 1/3-octave band, 20 Hz to 20 kHz — the Tonal-Balance-Control model.
//! A mix whose spectrum lies anywhere inside the band is on target there;
//! the decision engine ([`super::decide`]) acts only on the parts that fall
//! outside it.
//!
//! Values are **relative**: they describe spectral shape, normalised so the
//! band's midline reads 0 dB at 1 kHz, in the same per-bin *density* the
//! assistant's LTAS reads in (pink noise slopes −3 dB/oct there, white noise
//! is flat). The assistant aligns the analysed spectrum to the target's
//! midrange before comparing, so absolute level never enters into it.
//!
//! ## Where the numbers come from
//!
//! * **The slope** is the published average. Pestana, Reiss & Barbosa,
//!   *Spectral characteristics of popular commercial recordings 1950–2010*
//!   (AES 135th Convention, 2013) measured the long-term average spectrum of
//!   several hundred chart recordings and found it falls at roughly
//!   4.5–5 dB per octave between about 100 Hz and 4 kHz, with newer and
//!   louder genres at the shallow (brighter) end. Each genre below takes a
//!   slope from that range.
//! * **The shape outside 100 Hz–4 kHz** is the common shape those averages
//!   share: below 100 Hz the level keeps rising towards the lows, but far
//!   slower than the midrange slope would predict, and rolls off under
//!   40 Hz; above 4 kHz it falls faster than the midrange slope, and faster
//!   again above 12 kHz, where most masters carry little energy.
//! * **The genre offsets** on the low end (≤ 60 Hz, faded in over
//!   60–150 Hz) and the top (≥ 8 kHz, faded in over 3–8 kHz) are reasoned,
//!   not measured: they encode the usual mastering-practice differences —
//!   pop carries more sub and air, acoustic and jazz less sub, jazz a tamer
//!   top. They are small next to the tolerances on purpose.
//! * **The tolerance** (the band's half-width) follows the spread those
//!   studies report: tightest through the midrange, where commercial
//!   recordings agree most, and widest at the extremes, where they differ
//!   by several dB for purely artistic reasons. Acoustic and jazz get a
//!   wider band (more varied productions), pop a narrower one.
//!
//! Reference tracks (§7.5) are a separate comparison mode: they never
//! generate a band of their own (see [`super::decide::Target`]).

use std::sync::OnceLock;

use resonance_metering::spectrum::octave::OctaveTable;

use super::analyze::NUM_SPECTRUM_BINS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[derive(Default)]
pub enum Genre {
    #[default]
    Rock,
    Indie,
    Acoustic,
    Jazz,
    Pop,
}

impl Genre {
    pub const ALL: &'static [Self] = &[
        Self::Rock,
        Self::Indie,
        Self::Acoustic,
        Self::Jazz,
        Self::Pop,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Rock => "Rock",
            Self::Indie => "Indie",
            Self::Acoustic => "Acoustic",
            Self::Jazz => "Jazz",
            Self::Pop => "Pop",
        }
    }

    /// Stable lowercase id, used in saved state and on the control wire.
    pub fn id(&self) -> &'static str {
        match self {
            Self::Rock => "rock",
            Self::Indie => "indie",
            Self::Acoustic => "acoustic",
            Self::Jazz => "jazz",
            Self::Pop => "pop",
        }
    }

    /// The genre whose [`id`](Self::id) (or label) is `s`, ignoring case.
    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|g| g.id().eq_ignore_ascii_case(s.trim()))
    }

    /// Integrated-LUFS target appropriate for the genre. Matches the
    /// research brief's band-music guidelines — modern streaming is
    /// normalized to ~−14 LUFS, so the louder rock/pop targets are
    /// deliberately above that.
    pub fn target_lufs(&self) -> f32 {
        match self {
            Self::Rock => -11.0,
            Self::Indie => -13.0,
            Self::Acoustic => -16.0,
            Self::Jazz => -17.0,
            Self::Pop => -10.0,
        }
    }

    /// The genre's shape parameters. Every number is documented in the
    /// module docs; see there before changing one.
    fn shape(&self) -> GenreShape {
        match self {
            Self::Rock => GenreShape {
                slope_db_per_oct: -4.6,
                low_offset_db: 0.0,
                top_offset_db: 0.0,
                tolerance_scale: 1.0,
            },
            Self::Indie => GenreShape {
                slope_db_per_oct: -4.8,
                low_offset_db: -0.5,
                top_offset_db: -0.5,
                tolerance_scale: 1.0,
            },
            Self::Acoustic => GenreShape {
                slope_db_per_oct: -4.8,
                low_offset_db: -3.0,
                top_offset_db: 0.5,
                tolerance_scale: 1.2,
            },
            Self::Jazz => GenreShape {
                slope_db_per_oct: -5.0,
                low_offset_db: -2.0,
                top_offset_db: -2.0,
                tolerance_scale: 1.2,
            },
            Self::Pop => GenreShape {
                slope_db_per_oct: -4.5,
                low_offset_db: 2.0,
                top_offset_db: 1.5,
                tolerance_scale: 0.9,
            },
        }
    }
}

/// What distinguishes one genre's band from another's.
struct GenreShape {
    /// Pestana slope over 100 Hz–4 kHz, dB/oct.
    slope_db_per_oct: f32,
    /// Added at and below 60 Hz, faded to 0 at 150 Hz.
    low_offset_db: f32,
    /// Added at and above 8 kHz, faded to 0 at 3 kHz.
    top_offset_db: f32,
    /// Multiplies every tolerance.
    tolerance_scale: f32,
}

// The common shape (module docs, "The shape outside 100 Hz–4 kHz").
/// Below this the midline stops following the Pestana slope.
const SLOPE_LO_HZ: f32 = 100.0;
/// Above this the midline stops following the Pestana slope.
const SLOPE_HI_HZ: f32 = 4_000.0;
/// Density slope from 40 to 100 Hz, dB/oct: still rising towards the
/// lows, but slowly.
const LOW_SLOPE_DB_PER_OCT: f32 = -1.5;
/// Sub roll-off corner.
const SUB_CORNER_HZ: f32 = 40.0;
/// Extra fall per octave below the sub corner.
const SUB_ROLLOFF_DB_PER_OCT: f32 = 8.0;
/// Density slope from 4 to 12 kHz, dB/oct.
const TOP_SLOPE_DB_PER_OCT: f32 = -6.5;
/// Air roll-off corner.
const AIR_CORNER_HZ: f32 = 12_000.0;
/// Extra fall per octave above the air corner.
const AIR_ROLLOFF_DB_PER_OCT: f32 = 12.0;

/// Half-width of the band by region, dB, before the genre's scale.
fn base_tolerance_db(freq: f32) -> f32 {
    if freq < 60.0 {
        4.5
    } else if freq < 250.0 {
        3.0
    } else if freq <= 4_000.0 {
        2.0
    } else if freq <= 10_000.0 {
        3.0
    } else {
        4.5
    }
}

/// Number of ISO 1/3-octave bands a target is defined on, 20 Hz–20 kHz.
pub const NUM_TARGET_BANDS: usize = 31;

/// Exact centre of target band `i`: `1000 · 2^((i − 17) / 3)` Hz, the ISO
/// 266 series whose rounded names are 20, 25, 31.5, … 20 000 Hz.
pub fn target_band_center_hz(i: usize) -> f32 {
    1_000.0 * 2f32.powf((i as f32 - 17.0) / 3.0)
}

/// A target band: the lowest and highest on-target level per 1/3-octave
/// band, dB, relative (midline 0 dB at 1 kHz).
#[derive(Debug, Clone, PartialEq)]
pub struct TargetBands {
    pub lo_db: [f32; NUM_TARGET_BANDS],
    pub hi_db: [f32; NUM_TARGET_BANDS],
}

impl TargetBands {
    /// The band's midline at band `i`.
    pub fn mid_db(&self, i: usize) -> f32 {
        0.5 * (self.lo_db[i] + self.hi_db[i])
    }

    /// `(lo, hi)` at an arbitrary frequency: linear in log-frequency
    /// between band centres, held flat past either end.
    pub fn at_hz(&self, freq: f32) -> (f32, f32) {
        let pos = 17.0 + 3.0 * (freq.max(1.0) / 1_000.0).log2();
        if pos <= 0.0 {
            return (self.lo_db[0], self.hi_db[0]);
        }
        let last = NUM_TARGET_BANDS - 1;
        if pos >= last as f32 {
            return (self.lo_db[last], self.hi_db[last]);
        }
        let i = pos.floor() as usize;
        let t = pos - i as f32;
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        (
            lerp(self.lo_db[i], self.lo_db[i + 1]),
            lerp(self.hi_db[i], self.hi_db[i + 1]),
        )
    }

    /// The band resampled onto the assistant's 1/6-octave analysis grid
    /// ([`band_center_hz`]): `(lo, hi)` per analysis bin.
    pub fn on_analysis_grid(&self) -> ([f32; NUM_SPECTRUM_BINS], [f32; NUM_SPECTRUM_BINS]) {
        let mut lo = [0.0_f32; NUM_SPECTRUM_BINS];
        let mut hi = [0.0_f32; NUM_SPECTRUM_BINS];
        for i in 0..NUM_SPECTRUM_BINS {
            let (l, h) = self.at_hz(band_center_hz(i));
            lo[i] = l;
            hi[i] = h;
        }
        (lo, hi)
    }
}

/// The midline of a genre's band at `freq`, dB relative to 1 kHz.
fn midline_db(shape: &GenreShape, freq: f32) -> f32 {
    let s = shape.slope_db_per_oct;
    let oct = |a: f32, b: f32| (a / b).log2();
    let mut m = if freq < SLOPE_LO_HZ {
        let at_lo = s * oct(SLOPE_LO_HZ, 1_000.0);
        let mut v = at_lo + LOW_SLOPE_DB_PER_OCT * oct(freq, SLOPE_LO_HZ);
        if freq < SUB_CORNER_HZ {
            v -= SUB_ROLLOFF_DB_PER_OCT * oct(SUB_CORNER_HZ, freq);
        }
        v
    } else if freq > SLOPE_HI_HZ {
        let at_hi = s * oct(SLOPE_HI_HZ, 1_000.0);
        let mut v = at_hi + TOP_SLOPE_DB_PER_OCT * oct(freq, SLOPE_HI_HZ);
        if freq > AIR_CORNER_HZ {
            v -= AIR_ROLLOFF_DB_PER_OCT * oct(freq, AIR_CORNER_HZ);
        }
        v
    } else {
        s * oct(freq, 1_000.0)
    };
    m += shape.low_offset_db * fade(freq, 150.0, 60.0);
    m += shape.top_offset_db * fade(freq, 3_000.0, 8_000.0);
    m
}

/// 0 at `from`, 1 at `to`, linear in log-frequency and clamped outside —
/// `from` and `to` may be in either order.
fn fade(freq: f32, from: f32, to: f32) -> f32 {
    let t = (freq / from).log2() / (to / from).log2();
    t.clamp(0.0, 1.0)
}

/// The built-in target band for `genre`.
pub fn genre_bands(genre: Genre) -> TargetBands {
    let shape = genre.shape();
    let mut lo = [0.0_f32; NUM_TARGET_BANDS];
    let mut hi = [0.0_f32; NUM_TARGET_BANDS];
    for i in 0..NUM_TARGET_BANDS {
        let f = target_band_center_hz(i);
        let m = midline_db(&shape, f);
        let tol = base_tolerance_db(f) * shape.tolerance_scale;
        lo[i] = m - tol;
        hi[i] = m + tol;
    }
    TargetBands {
        lo_db: lo,
        hi_db: hi,
    }
}

/// A genre's band on the 1/6-octave analysis grid: `(lo, hi)`, indexed
/// like [`band_center_hz`], from the lowest bin (20–22.4 Hz) at `[0]` to
/// the highest (17.8–20 kHz) at `[NUM_SPECTRUM_BINS - 1]`.
pub fn target_band(genre: Genre) -> ([f32; NUM_SPECTRUM_BINS], [f32; NUM_SPECTRUM_BINS]) {
    genre_bands(genre).on_analysis_grid()
}

/// The midline of a genre's band on the 1/6-octave analysis grid — the
/// single curve a spectrum display can draw for the target.
pub fn target_curve(genre: Genre) -> [f32; NUM_SPECTRUM_BINS] {
    let (lo, hi) = target_band(genre);
    let mut curve = [0.0_f32; NUM_SPECTRUM_BINS];
    for i in 0..NUM_SPECTRUM_BINS {
        curve[i] = 0.5 * (lo[i] + hi[i]);
    }
    curve
}

/// Centre frequency of the `i`th bin of the 1/6-octave analysis grid:
/// the geometric centre of the bin's edges, exactly as the LTAS bins
/// them (`resonance_metering`'s [`OctaveTable::center`]). The 60 bins
/// tile 20 Hz–20 kHz edge to edge, so the centres run from ≈21.2 Hz at
/// `[0]` to ≈18.9 kHz at `[NUM_SPECTRUM_BINS - 1]`. (This used to return
/// the bin's lower edge, half a bin (1/12 octave) low.)
pub fn band_center_hz(i: usize) -> f32 {
    static TABLE: OnceLock<OctaveTable> = OnceLock::new();
    TABLE.get_or_init(OctaveTable::new).center(i)
}
