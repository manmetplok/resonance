//! `depth` detail through the real offline measure path
//! (warmth-width-depth.md §7.6, W11): each return a track sends to is
//! rendered once from its send-only feeders, its gain measured, and each
//! track's DRR estimate follows from its sends and those gains.
//!
//! Plugin-free: the "rooms" are return busses at different faders, so
//! every return's gain is known exactly (its fader) and the estimate can
//! be checked numerically, not only for ordering.

use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};

use resonance_audio::test_support::{
    measure_mix_detailed, AutomationSnapshot, MeasureSource, MixMeasurement, SharedState,
    StemSource,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const FRAMES: usize = SR as usize * 4;
const SHORT_ROOM: BusId = 50;
const LONG_ROOM: BusId = 51;

struct Project {
    shared: Arc<SharedState>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    tx: Sender<AudioEvent>,
    rx: Receiver<AudioEvent>,
    sends: Vec<AuxSend>,
}

impl Project {
    fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let p = Self {
            shared: Arc::new(SharedState::default()),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
            tx,
            rx,
            sends: Vec::new(),
        };
        // A "short room" return 6 dB down, a "long room" at unity.
        for (id, volume) in [(SHORT_ROOM, 0.5f32), (LONG_ROOM, 1.0)] {
            let bus = Bus::new(id, format!("room {id}"));
            bus.set_volume(volume);
            bus.set_is_return(true);
            p.shared.edit_busses(|b| {
                b.insert(id, Arc::new(bus));
            });
        }
        p
    }

    /// A track playing its own noise, at `volume` on the fader.
    fn add_track(&self, id: TrackId, volume: f32, seed: u32) {
        let track = Track::new(id, format!("track {id}"));
        track.set_volume(volume);
        self.shared.edit_tracks(|m| {
            m.insert(id, Arc::new(track));
        });
        let mut state = seed | 1;
        let pcm: Vec<f32> = (0..FRAMES * 2)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / 8_388_608.0 - 1.0) * 0.2
            })
            .collect();
        self.shared.edit_clips(|c| {
            c.push(Arc::new(AudioClip {
                id,
                track_id: id,
                start_sample: 0,
                source: ClipSource::memory(pcm),
                name: "noise".into(),
                trim_start_frames: 0,
                trim_end_frames: 0,
                fade_in_frames: 0,
                fade_in_curve: FadeCurve::default(),
                fade_out_frames: 0,
                fade_out_curve: FadeCurve::default(),
                gain_db: 0.0,
                vocal_tuning: None,
                warp_enabled: false,
                original_bpm: None,
                transpose_semitones: 0.0,
                warp_algorithm: WarpAlgorithm::default(),
                warp_markers: Vec::new(),
                tuning_render_cache: None,
            }))
        });
    }

    fn send(&mut self, from: TrackId, to: BusId, level_db: f32, pre_fader: bool) {
        self.sends.push(AuxSend {
            id: 1_000 + self.sends.len() as SendId,
            source: SendSource::Track(from),
            dest: to,
            level_db,
            pre_fader,
            enabled: true,
        });
        self.shared.aux_sends.store(Arc::new(self.sends.clone()));
    }

    fn measure(&self, targets: Vec<StemSource>) -> Vec<MixMeasurement> {
        let detail = DetailSet {
            depth: true,
            ..DetailSet::default()
        };
        measure_mix_detailed(
            9,
            targets,
            None,
            MeasureSource::Render,
            detail,
            &self.shared,
            &self.tempo_map,
            &AutomationSnapshot::default(),
            SR,
            &self.tx,
        );
        let events: Vec<AudioEvent> = self.rx.try_iter().collect();
        match events.as_slice() {
            [AudioEvent::MixMeasured { results, .. }] => results.clone(),
            other => panic!("expected one MixMeasured, got {other:?}"),
        }
    }
}

fn depth(m: &MixMeasurement) -> &DepthDetail {
    m.detail.depth.as_ref().expect("depth was asked for")
}

fn tracks(ids: &[TrackId]) -> Vec<StemSource> {
    ids.iter().map(|&id| StemSource::Track(id)).collect()
}

#[test]
fn three_layers_order_front_middle_back() {
    let mut p = Project::new();
    p.add_track(1, 1.0, 11); // front: a whisper of the short room
    p.add_track(2, 1.0, 22); // middle: a real send to the short room
    p.add_track(3, 1.0, 33); // back: a big send to the long room
    p.add_track(4, 1.0, 44); // dry: no send at all
    p.send(1, SHORT_ROOM, -30.0, false);
    p.send(2, SHORT_ROOM, -10.0, false);
    p.send(3, LONG_ROOM, 0.0, false);

    let mut targets = vec![StemSource::Master];
    targets.extend(tracks(&[1, 2, 3, 4]));
    let r = p.measure(targets);
    let drr = |i: usize| depth(&r[i]).drr_db_estimate.expect("a track with a send");

    let (front, middle, back) = (drr(1), drr(2), drr(3));
    assert!(front > middle && middle > back, "front > middle > back: {front} {middle} {back}");
    // Exact here: a passthrough return's gain is its fader.
    assert!((drr(1) - 36.0).abs() < 0.5, "front {}", drr(1));
    assert!((drr(2) - 16.0).abs() < 0.5, "middle {}", drr(2));
    assert!(drr(3).abs() < 0.3, "back {}", drr(3));

    let short = depth(&r[2]).sends[0];
    assert_eq!(short.bus_id, SHORT_ROOM);
    assert!((short.return_gain_db.unwrap() - -6.02).abs() < 0.5, "{short:?}");

    let dry = depth(&r[4]);
    assert!(dry.dry_only && dry.drr_db_estimate.is_none() && dry.sends.is_empty());
    assert!(depth(&r[0]).drr_db_estimate.is_none(), "the master has no DRR");
    assert!(depth(&r[1]).hf_tilt_db.is_some());
}

#[test]
fn a_post_fader_estimate_ignores_the_fader_and_a_pre_fader_one_adds_it() {
    let mut p = Project::new();
    p.add_track(1, 0.5, 11); // post-fader, fader at -6 dB
    p.add_track(2, 0.5, 22); // pre-fader, fader at -6 dB
    p.send(1, LONG_ROOM, -12.0, false);
    p.send(2, SHORT_ROOM, -12.0, true);
    let r = p.measure(tracks(&[1, 2]));
    let post = depth(&r[0]).drr_db_estimate.unwrap();
    let pre = depth(&r[1]).drr_db_estimate.unwrap();
    // Post: dry and wet both carry the fader, DRR = 12 dB.
    assert!((post - 12.0).abs() < 0.3, "post-fader {post}");
    // Pre: the send skips the -6 dB fader, and the short room is another
    // -6 dB: DRR = -6 + 12 + 6 = 12 dB.
    assert!((pre - 12.0).abs() < 0.3, "pre-fader {pre}");
}

#[test]
fn a_feeder_outside_the_targets_is_rendered_for_its_return() {
    // Only track 1 is measured, but track 2 also feeds the short room:
    // the return's gain still comes out right.
    let mut p = Project::new();
    p.add_track(1, 1.0, 11);
    p.add_track(2, 1.0, 22);
    p.send(1, SHORT_ROOM, -6.0, false);
    p.send(2, SHORT_ROOM, -6.0, false);
    let r = p.measure(tracks(&[1]));
    let gain = depth(&r[0]).sends[0].return_gain_db.unwrap();
    assert!((gain - -6.02).abs() < 0.5, "return gain {gain}");
}

#[test]
fn without_depth_nothing_extra_is_rendered_or_reported() {
    let mut p = Project::new();
    p.add_track(1, 1.0, 11);
    p.send(1, SHORT_ROOM, -6.0, false);
    measure_mix_detailed(
        9,
        tracks(&[1]),
        None,
        MeasureSource::Render,
        DetailSet::default(),
        &p.shared,
        &p.tempo_map,
        &AutomationSnapshot::default(),
        SR,
        &p.tx,
    );
    let events: Vec<AudioEvent> = p.rx.try_iter().collect();
    let [AudioEvent::MixMeasured { results, .. }] = events.as_slice() else {
        panic!("{events:?}");
    };
    assert_eq!(results[0].detail.depth, None);
}
