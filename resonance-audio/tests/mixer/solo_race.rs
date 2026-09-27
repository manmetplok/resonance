//! FU-B3a: a block used to compute `any_solo` once from a scan over every
//! track, then re-read each track's own `soloed()` again later while
//! deciding that track's disposition. A solo toggle landing between the
//! two reads could make the aggregate and the single track's fresh read
//! disagree — `any_solo == true` (this track *was* soloed when the scan
//! ran) but `track.soloed() == false` (it flipped off by the time its own
//! disposition was decided) — silencing a track that should always have
//! been audible, or the reverse. With only one soloed candidate ever in
//! play, that candidate must be audible in *every* possible interleaving:
//! soloed, it is the one solo lets through; not soloed, nothing is soloed
//! at all, so mute/solo gates nothing. `render_blocks` below asserts
//! exactly that per block (`data.iter().any(|&s| s != 0.0)`), which is why
//! the older concurrent-edit test in `render_graph_publish.rs` pointedly
//! did *not* use it for its own solo-toggling editor thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use resonance_audio::test_support::MixAudioHarness;
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const BUS_BLOCKS: usize = 20_000;

/// A minute of a constant 0.1 on `track_id`, from the top — loud enough
/// that "not silent" is an unambiguous check.
fn audio_clip(id: ClipId, track_id: TrackId) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::Memory(vec![0.1; 2 * SR as usize * 60]),
        name: "a".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::Linear,
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: WarpAlgorithm::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

/// Two audio tracks straight to master, both unmuted and never soloed
/// except track 1, which the test hammers. Track 2 exists so `any_solo`
/// is a real aggregate over more than one track rather than a
/// single-track special case.
fn harness() -> MixAudioHarness {
    // `Track::new` already defaults to `TrackOutput::Master`.
    let tracks: Vec<Track> = [1u64, 2].into_iter().map(|id| Track::new(id, format!("t{id}"))).collect();
    let clips = vec![audio_clip(1, 1), audio_clip(2, 2)];
    let h = MixAudioHarness::new(
        tracks,
        Vec::new(),
        clips,
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    h.shared().playing.store(true, Ordering::Relaxed);
    h
}

/// Render `BUS_BLOCKS` blocks, asserting each one carries signal — the
/// same per-block check `render_blocks` makes in `render_graph_publish.rs`.
fn render_blocks_asserting_audible(h: &mut MixAudioHarness) {
    for block in 0..BUS_BLOCKS {
        let data = h.render();
        assert!(
            data.iter().any(|&s| s != 0.0),
            "block {block} rendered all-silent — track 1 should always be audible \
             (either soloed, or nothing is soloed)"
        );
    }
}

/// Track 1 toggles between soloed and not-soloed as fast as the editor
/// thread can manage while the callback renders. Track 1 is audible in
/// both states — soloed, it is the one solo admits; not soloed, no track
/// is soloed at all, so mute/solo gates nothing — so no rendered block may
/// ever be silent. Before FU-B3a's fix this could fail (or flake): the
/// block's `any_solo` scan and track 1's own later `soloed()` read were
/// two independent atomic loads that a toggle landing between them could
/// disagree about.
#[test]
fn solo_toggle_on_the_only_soloed_track_never_silences_a_block() {
    let mut h = harness();
    let shared = h.shared_arc();
    let stop = Arc::new(AtomicBool::new(false));
    let editor = {
        let shared = Arc::clone(&shared);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut toggles = 0u64;
            while !stop.load(Ordering::Acquire) {
                let graph = shared.graph.load();
                let one = graph.track(1).expect("track 1");
                one.set_soloed(!one.soloed());
                drop(graph);
                toggles += 1;
            }
            toggles
        })
    };

    render_blocks_asserting_audible(&mut h);

    stop.store(true, Ordering::Release);
    let toggles = editor.join().expect("editor");
    assert!(toggles > 0, "the editor actually raced the callback");
    assert_eq!(
        h.shared().render_skip_cycles.load(Ordering::Relaxed),
        0,
        "no block was skipped across {toggles} concurrent solo toggles"
    );
}
