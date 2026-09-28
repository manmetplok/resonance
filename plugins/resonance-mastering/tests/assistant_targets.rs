//! The built-in genre target bands (warmth-width-depth.md §7.4, D6).

use resonance_mastering::assistant::analyze::NUM_SPECTRUM_BINS;
use resonance_mastering::assistant::targets::{
    band_center_hz, genre_bands, target_band, target_band_center_hz, target_curve, Genre,
    NUM_TARGET_BANDS,
};

fn band_index(hz: f32) -> usize {
    (0..NUM_TARGET_BANDS)
        .min_by(|&a, &b| {
            let da = (target_band_center_hz(a) / hz).log2().abs();
            let db = (target_band_center_hz(b) / hz).log2().abs();
            da.partial_cmp(&db).unwrap()
        })
        .unwrap()
}

#[test]
fn band_centres_are_the_iso_third_octave_series() {
    assert_eq!(NUM_TARGET_BANDS, 31);
    assert!((target_band_center_hz(0) - 19.69).abs() < 0.01);
    assert!((target_band_center_hz(17) - 1_000.0).abs() < 1e-3);
    assert!((target_band_center_hz(30) - 20_159.0).abs() < 1.0);
}

/// The analysis grid's centres are the geometric centres of the bins the
/// LTAS measures (review finding M8: they were the lower edges, half a
/// bin low, so every target value was read 1/12 octave off).
#[test]
fn analysis_grid_centres_are_the_ltas_bin_centres() {
    use resonance_metering::spectrum::octave::OctaveTable;
    let table = OctaveTable::new();
    for i in 0..NUM_SPECTRUM_BINS {
        assert_eq!(band_center_hz(i), table.center(i), "bin {i}");
        let (lo, hi) = (table.edges[i], table.edges[i + 1]);
        assert!(lo < band_center_hz(i) && band_center_hz(i) < hi, "bin {i}");
    }
    assert!((band_center_hz(0) - 21.19).abs() < 0.01, "{}", band_center_hz(0));
    let last = band_center_hz(NUM_SPECTRUM_BINS - 1);
    assert!((last - 18_881.2).abs() < 1.0, "{last}");
}

#[test]
fn every_band_is_finite_ordered_and_centred_on_1k() {
    for &g in Genre::ALL {
        let b = genre_bands(g);
        for i in 0..NUM_TARGET_BANDS {
            assert!(b.lo_db[i].is_finite() && b.hi_db[i].is_finite());
            assert!(b.lo_db[i] < b.hi_db[i], "{g:?} band {i} is empty");
        }
        assert!(b.mid_db(17).abs() < 1e-4, "{g:?} midline is not 0 dB at 1 kHz");
    }
}

/// The midline follows the genre's Pestana slope (−4.5..−5 dB/oct) over
/// 100 Hz–4 kHz.
#[test]
fn midline_has_a_pestana_slope_over_100_hz_to_4_khz() {
    for &g in Genre::ALL {
        let b = genre_bands(g);
        let (i100, i4k) = (band_index(100.0), band_index(4_000.0));
        let octaves = (target_band_center_hz(i4k) / target_band_center_hz(i100)).log2();
        let slope = (b.mid_db(i4k) - b.mid_db(i100)) / octaves;
        assert!(
            (-5.05..=-4.45).contains(&slope),
            "{g:?} slope {slope} dB/oct is outside the published range"
        );
    }
}

/// Tightest through the midrange, widest at the extremes.
#[test]
fn tolerance_is_tightest_in_the_midrange() {
    for &g in Genre::ALL {
        let b = genre_bands(g);
        let width = |hz: f32| {
            let i = band_index(hz);
            b.hi_db[i] - b.lo_db[i]
        };
        assert!(width(1_000.0) < width(100.0), "{g:?}");
        assert!(width(100.0) < width(25.0), "{g:?}");
        assert!(width(1_000.0) < width(6_300.0), "{g:?}");
        assert!(width(6_300.0) < width(16_000.0), "{g:?}");
    }
}

#[test]
fn genre_offsets_point_the_documented_way() {
    let sub = band_index(40.0);
    let air = band_index(12_500.0);
    let pop = genre_bands(Genre::Pop);
    let rock = genre_bands(Genre::Rock);
    let acoustic = genre_bands(Genre::Acoustic);
    let jazz = genre_bands(Genre::Jazz);
    assert!(pop.mid_db(sub) > rock.mid_db(sub));
    assert!(acoustic.mid_db(sub) < rock.mid_db(sub));
    assert!(jazz.mid_db(air) < rock.mid_db(air));
    assert!(pop.mid_db(air) > rock.mid_db(air));
}

#[test]
fn the_analysis_grid_curve_is_the_band_midline() {
    for &g in Genre::ALL {
        let (lo, hi) = target_band(g);
        let c = target_curve(g);
        assert_eq!(c.len(), NUM_SPECTRUM_BINS);
        for i in 0..NUM_SPECTRUM_BINS {
            assert!(c[i].is_finite());
            assert!(lo[i] < c[i] && c[i] < hi[i]);
        }
        // The grid band agrees with the 1/3-octave band where they meet.
        let b = genre_bands(g);
        let i = (0..NUM_SPECTRUM_BINS)
            .find(|&i| band_center_hz(i) >= 1_000.0)
            .unwrap();
        let (l, h) = b.at_hz(band_center_hz(i));
        assert!((l - lo[i]).abs() < 1e-5 && (h - hi[i]).abs() < 1e-5);
    }
}

#[test]
fn genre_ids_round_trip() {
    for &g in Genre::ALL {
        assert_eq!(Genre::from_id(g.id()), Some(g));
        assert_eq!(Genre::from_id(&g.label().to_uppercase()), Some(g));
    }
    assert_eq!(Genre::from_id("polka"), None);
}

#[test]
fn target_lufs_is_genre_dependent() {
    assert!(Genre::Rock.target_lufs() > Genre::Acoustic.target_lufs());
    assert!(Genre::Pop.target_lufs() > Genre::Jazz.target_lufs());
}
