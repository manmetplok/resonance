//! Grid snap with one tempo point but a signature change (review FU-V1a).
//!
//! `snap_sample_to_grid_tempo`'s single-tempo shortcut built a flat grid
//! from the *transport* numerator — which follows the playhead — so after
//! a signature change the snap grid (clip drags, loop drags, ruler seek,
//! markers) drifted off the bar lines the ruler draws from the map.

use resonance_app::view::timeline::snap_sample_to_grid_tempo;
use resonance_audio::types::{SignaturePoint, TempoMap, TempoPoint};

const SR: u32 = 48_000;
/// 120 BPM: a quarter note is 0.5 s.
const Q: u64 = SR as u64 / 2;
/// 15 px/s → a 4/4 bar is 30 px and a 3/4 bar 22.5 px: bar-level snap.
const ZOOM: f32 = 15.0;

/// 4/4 for bars 0-1, then 3/4: bar lines at 0, 4, 8, 11, 14 quarters.
fn map() -> TempoMap {
    let mut map = TempoMap::default();
    map.tempo_points = vec![TempoPoint { bar: 0, bpm: 120.0 }];
    map.signature_points = vec![
        SignaturePoint {
            bar: 0,
            numerator: 4,
            denominator: 4,
        },
        SignaturePoint {
            bar: 2,
            numerator: 3,
            denominator: 4,
        },
    ];
    map.rebuild_bar_table(SR);
    map
}

#[test]
fn single_tempo_snap_follows_the_signature_map() {
    let map = map();
    let near_bar_3 = 11 * Q + Q / 5;
    // Whatever meter the playhead is in, the snap lands on the map's bar.
    for transport_num in [3u8, 4] {
        let snapped = snap_sample_to_grid_tempo(near_bar_3, 120.0, transport_num, SR, ZOOM, &map);
        assert_eq!(
            snapped,
            map.bar_to_sample(3),
            "transport numerator {transport_num}: expected bar 3 at 11 quarters"
        );
    }
}

#[test]
fn single_tempo_single_signature_is_unchanged() {
    let mut map = TempoMap::default();
    map.tempo_points = vec![TempoPoint { bar: 0, bpm: 120.0 }];
    map.signature_points = vec![SignaturePoint {
        bar: 0,
        numerator: 4,
        denominator: 4,
    }];
    map.rebuild_bar_table(SR);
    assert_eq!(snap_sample_to_grid_tempo(9 * Q, 120.0, 4, SR, ZOOM, &map), 8 * Q);
}
