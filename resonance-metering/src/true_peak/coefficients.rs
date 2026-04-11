//! 48-tap polyphase FIR used for 4× true-peak oversampling.
//!
//! Coefficients are the ones defined in **ITU-R BS.1770-4 Annex 2** and
//! are identical to the table shipped with `libebur128` (which in turn
//! is validated against the EBU Tech 3341 test set). Split into four
//! 12-tap polyphase sub-filters so one input sample is convolved against
//! each phase once to produce four output samples at 4× the input rate.
//!
//! Do **not** regenerate these values — they are part of the metering
//! spec and any rounding drift will break the reference test vectors.

/// Number of polyphase sub-filters (oversampling factor).
pub const PHASES: usize = 4;
/// Taps per polyphase sub-filter. `TAPS * PHASES == 48` total coefficients.
pub const TAPS: usize = 12;

/// Four polyphase sub-filters, each with 12 taps.
///
/// `FIR[p][j]` is the coefficient for phase `p` at history index `j`,
/// where `j = 0` multiplies the most recent input sample.
pub const FIR: [[f32; TAPS]; PHASES] = [
    [
        0.001_708_984_375,
        0.010_986_328_125,
        -0.019_653_320_312_5,
        0.033_203_125,
        -0.059_448_242_187_5,
        0.137_329_101_562_5,
        0.972_167_968_75,
        -0.102_294_921_875,
        0.047_607_421_875,
        -0.026_611_328_125,
        0.014_892_578_125,
        -0.008_300_781_25,
    ],
    [
        -0.029_174_804_687_5,
        0.029_296_875,
        -0.051_757_812_5,
        0.089_111_328_125,
        -0.166_503_906_25,
        0.465_087_890_625,
        0.779_785_156_25,
        -0.200_317_382_812_5,
        0.101_562_5,
        -0.058_227_539_062_5,
        0.033_081_054_687_5,
        -0.018_920_898_437_5,
    ],
    [
        -0.018_920_898_437_5,
        0.033_081_054_687_5,
        -0.058_227_539_062_5,
        0.101_562_5,
        -0.200_317_382_812_5,
        0.779_785_156_25,
        0.465_087_890_625,
        -0.166_503_906_25,
        0.089_111_328_125,
        -0.051_757_812_5,
        0.029_296_875,
        -0.029_174_804_687_5,
    ],
    [
        -0.008_300_781_25,
        0.014_892_578_125,
        -0.026_611_328_125,
        0.047_607_421_875,
        -0.102_294_921_875,
        0.972_167_968_75,
        0.137_329_101_562_5,
        -0.059_448_242_187_5,
        0.033_203_125,
        -0.019_653_320_312_5,
        0.010_986_328_125,
        0.001_708_984_375,
    ],
];
