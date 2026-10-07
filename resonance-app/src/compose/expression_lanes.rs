//! Vocal expression curves as automation lanes on the timeline
//! (vocal-mcp-control-and-quality.md §10).
//!
//! A curve is stored once per section (in the vocal lane's config,
//! section-relative); the timeline shows it at **every placement** of the
//! section. [`project_vocal_lanes`] builds that view: per vocal track and
//! curve kind a [`VocalLane`] — an ordinary [`AutomationLane`] on an
//! [`AutomationTarget::VocalCurve`] target whose points sit in song frames,
//! plus, for every point, the section point it is a copy of
//! ([`PointOrigin`]) and, for every placement, the window where the row is
//! live ([`CurveWindow`]). Outside every window the row is inert.
//!
//! The projection is derived state: rebuilt from the compose model after
//! every update, never saved, never undone, never sent to the engine (the
//! target is render-only). Edits go the other way — a frame is mapped to
//! its placement, then the section, then a beat offset — in
//! `update::compose::expression`; [`plan_set_lane`] is the pure half of
//! the whole-lane replace `automation.*` writes with.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use resonance_audio::types::{TempoMap, TrackId, TICKS_PER_QUARTER_NOTE};
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind as Shape, LaneId};
use resonance_music_theory::{VocalCurvePoint, VocalParams};

use super::expression::{curve, upsert_point};
use super::lane_generator::LaneGeneratorKind;
use super::section::{SectionDefinitionState, SectionPlacementState};
use super::vocal_svs::CurveKind;

/// Lane ids at and above this are vocal-curve projections, derived from
/// `(track, curve)` ([`vocal_lane_id`]); the app's lane allocator counts up
/// from 1 and never gets near it.
pub const VOCAL_LANE_ID_BASE: LaneId = 1 << 62;

/// The stable lane id of `track`'s `kind` curve row.
pub fn vocal_lane_id(track: TrackId, kind: CurveKind) -> LaneId {
    VOCAL_LANE_ID_BASE | (track << 2) | kind.index() as u64
}

/// One placement of a section with a vocal lane on the row's track: where
/// the row is live.
#[derive(Debug, Clone, PartialEq)]
pub struct CurveWindow {
    pub placement_id: u64,
    pub definition_id: u64,
    /// First frame of the placement.
    pub start_frame: u64,
    /// First frame after it.
    pub end_frame: u64,
    /// Absolute tick of `start_frame` — beat 0 of the section.
    pub start_tick: u64,
    /// The section's length in beats (quarter notes).
    pub len_beats: f64,
    /// Whether the lane's bank renders this curve (see
    /// [`super::expression::curve_audible`]); an unsupported window is
    /// drawn greyed and ignores pointer input.
    pub supported: bool,
    /// The bank's display name, for "not supported by <bank>" (shared
    /// by every window the bank sings, so a rebuild allocates no text).
    pub bank: Arc<str>,
}

impl CurveWindow {
    /// Whether `frame` falls inside the placement (half-open).
    pub fn contains(&self, frame: u64) -> bool {
        frame >= self.start_frame && frame < self.end_frame
    }

    /// The section beat `frame` sits at, clamped to the section.
    pub fn beat_at(&self, tempo_map: &TempoMap, sample_rate: u32, frame: u64) -> f64 {
        let tick = tempo_map
            .sample_to_abs_tick(frame, sample_rate)
            .saturating_sub(self.start_tick);
        (tick as f64 / TICKS_PER_QUARTER_NOTE as f64).clamp(0.0, self.len_beats)
    }

    /// The frame section beat `beat` sits at in this placement.
    pub fn frame_at(&self, tempo_map: &TempoMap, sample_rate: u32, beat: f64) -> u64 {
        let ticks = (beat.max(0.0) * TICKS_PER_QUARTER_NOTE as f64).round() as u64;
        tempo_map.tick_to_abs_sample(self.start_frame, ticks, sample_rate)
    }
}

/// Which section point a projected point is a copy of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PointOrigin {
    pub placement_id: u64,
    pub definition_id: u64,
    /// Index into the section's curve.
    pub index: usize,
}

impl PointOrigin {
    /// The section point, regardless of placement.
    fn section_point(&self) -> (u64, usize) {
        (self.definition_id, self.index)
    }
}

/// One vocal curve row of one track.
#[derive(Debug, Clone, PartialEq)]
pub struct VocalLane {
    /// The projected lane: song-frame points, always enabled.
    pub lane: AutomationLane,
    /// `origins[i]` is where `lane.points[i]` comes from.
    pub origins: Vec<PointOrigin>,
    /// Every placement window, by start frame.
    pub windows: Vec<CurveWindow>,
}

impl VocalLane {
    /// The placement window containing `frame`; a frame exactly at the end
    /// of a window no other window starts at counts as inside it (a point
    /// on the section's last beat).
    pub fn window_at(&self, frame: u64) -> Option<&CurveWindow> {
        self.windows
            .iter()
            .find(|w| w.contains(frame))
            .or_else(|| self.windows.iter().find(|w| w.end_frame == frame))
    }

    /// The window of `placement_id`.
    pub fn window_of(&self, placement_id: u64) -> Option<&CurveWindow> {
        self.windows.iter().find(|w| w.placement_id == placement_id)
    }

    /// Every projected index that is a copy of the same section point as
    /// one of `indices` — deleting a point at one placement deletes it at
    /// all of them.
    pub fn copies_of(&self, indices: &HashSet<usize>) -> HashSet<usize> {
        let wanted: HashSet<(u64, usize)> = indices
            .iter()
            .filter_map(|&i| self.origins.get(i))
            .map(PointOrigin::section_point)
            .collect();
        self.origins
            .iter()
            .enumerate()
            .filter(|(_, o)| wanted.contains(&o.section_point()))
            .map(|(i, _)| i)
            .collect()
    }
}

/// Every vocal curve row in the project.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VocalLanes {
    lanes: HashMap<AutomationTarget, VocalLane>,
}

impl VocalLanes {
    /// The row on `target`.
    pub fn get(&self, target: &AutomationTarget) -> Option<&VocalLane> {
        self.lanes.get(target)
    }

    /// The row whose projected lane has id `id`.
    pub fn by_id(&self, id: LaneId) -> Option<&VocalLane> {
        self.lanes.values().find(|l| l.lane.id == id)
    }

    /// `track`'s rows in display order (empty for a track with no vocal
    /// lane in any section).
    pub fn track_lanes(&self, track: TrackId) -> Vec<&VocalLane> {
        CurveKind::ALL
            .iter()
            .filter_map(|&curve| {
                self.lanes
                    .get(&AutomationTarget::VocalCurve { track, curve })
            })
            .collect()
    }

    /// Every row, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = &VocalLane> {
        self.lanes.values()
    }

    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }
}

/// What [`project_vocal_lanes`] reads.
pub struct ProjectionInputs<'a> {
    pub definitions: &'a [SectionDefinitionState],
    pub placements: &'a [SectionPlacementState],
    pub tempo_map: &'a TempoMap,
    pub sample_rate: u32,
    /// The lane's bank: which curves it renders (by [`CurveKind::index`])
    /// and its display name.
    pub bank: &'a dyn Fn(&VocalParams) -> ([bool; 4], Arc<str>),
}

/// The vocal lane config of `track` in `definition`.
pub fn vocal_params(definition: &SectionDefinitionState, track: TrackId) -> Option<&VocalParams> {
    match &definition.lane_generators.get(&track)?.kind {
        LaneGeneratorKind::Vocal(params) => Some(params),
        _ => None,
    }
}

/// Build every vocal curve row: one per curve kind for each track that has
/// a vocal lane in at least one section.
pub fn project_vocal_lanes(inputs: &ProjectionInputs<'_>) -> VocalLanes {
    let mut placements: Vec<&SectionPlacementState> = inputs.placements.iter().collect();
    placements.sort_by_key(|p| (p.start_bar, p.id));
    // Per track: (window, the lane's params) for every placement.
    let mut per_track: HashMap<TrackId, Vec<(CurveWindow, &VocalParams, [bool; 4])>> =
        HashMap::new();
    let mut tracks: HashSet<TrackId> = HashSet::new();
    for def in inputs.definitions {
        for (&track, cfg) in &def.lane_generators {
            if matches!(cfg.kind, LaneGeneratorKind::Vocal(_)) {
                tracks.insert(track);
            }
        }
    }
    for placement in &placements {
        let Some(def) = inputs
            .definitions
            .iter()
            .find(|d| d.id == placement.definition_id)
        else {
            continue;
        };
        let start_frame = inputs.tempo_map.bar_to_sample(placement.start_bar);
        let end_frame = inputs
            .tempo_map
            .bar_to_sample(placement.start_bar + def.length_bars);
        let start_tick = inputs
            .tempo_map
            .sample_to_abs_tick(start_frame, inputs.sample_rate);
        let end_tick = inputs
            .tempo_map
            .sample_to_abs_tick(end_frame, inputs.sample_rate);
        let len_beats = end_tick.saturating_sub(start_tick) as f64 / TICKS_PER_QUARTER_NOTE as f64;
        let mut lane_tracks: Vec<TrackId> = def.lane_generators.keys().copied().collect();
        lane_tracks.sort_unstable();
        for track in lane_tracks {
            let Some(params) = vocal_params(def, track) else {
                continue;
            };
            let (support, bank) = (inputs.bank)(params);
            per_track.entry(track).or_default().push((
                CurveWindow {
                    placement_id: placement.id,
                    definition_id: def.id,
                    start_frame,
                    end_frame,
                    start_tick,
                    len_beats,
                    supported: true,
                    bank,
                },
                params,
                support,
            ));
        }
    }

    let mut lanes = HashMap::new();
    for track in tracks {
        let placed = per_track.get(&track).map(Vec::as_slice).unwrap_or(&[]);
        for kind in CurveKind::ALL {
            let mut points: Vec<(Breakpoint, PointOrigin)> = Vec::new();
            let mut windows = Vec::with_capacity(placed.len());
            for (window, params, support) in placed {
                for (index, p) in curve(&params.curves, kind).iter().enumerate() {
                    if p.beat > window.len_beats {
                        continue;
                    }
                    let frame = window.frame_at(inputs.tempo_map, inputs.sample_rate, p.beat);
                    let shape = if p.stepped { Shape::Stepped } else { Shape::Linear };
                    points.push((
                        Breakpoint::new(frame, p.value, shape),
                        PointOrigin {
                            placement_id: window.placement_id,
                            definition_id: window.definition_id,
                            index,
                        },
                    ));
                }
                windows.push(CurveWindow {
                    supported: support[kind.index()],
                    ..window.clone()
                });
            }
            points.sort_by_key(|(p, _)| p.time_frames);
            let target = AutomationTarget::VocalCurve { track, curve: kind };
            let (points, origins): (Vec<Breakpoint>, Vec<PointOrigin>) = points.into_iter().unzip();
            let mut lane = AutomationLane::new(vocal_lane_id(track, kind), target.clone(), Vec::new());
            lane.points = points;
            lanes.insert(
                target,
                VocalLane {
                    lane,
                    origins,
                    windows,
                },
            );
        }
    }
    VocalLanes { lanes }
}

/// Why a whole-lane replace cannot be applied.
#[derive(Debug, Clone, PartialEq)]
pub enum PlanError {
    /// A new point at this frame lies outside every placement of a section
    /// with a vocal lane on the track.
    OutsidePlacements { frame: u64 },
    /// Two new points land on the same beat of the same section with
    /// different values (two placements of one section share one curve).
    Conflict { first: u64, second: u64 },
}

/// The section curves a whole-lane replace of `lane` with `new_points`
/// (song frames, sorted or not) amounts to, as `(definition, curve)` for
/// every section the row covers.
///
/// The new list is compared with the projection: a section point none of
/// whose copies survives is deleted; a new point equal to a surviving copy
/// changes nothing; any other new point is mapped to its placement's
/// section and beat and upserted there. So a caller that keeps a point at
/// one placement and drops its copy at another keeps it, and a caller that
/// changes a point at one placement changes it everywhere.
pub fn plan_set_lane(
    lane: &VocalLane,
    new_points: &[Breakpoint],
    current: &dyn Fn(u64) -> Vec<VocalCurvePoint>,
    tempo_map: &TempoMap,
    sample_rate: u32,
) -> Result<Vec<(u64, Vec<VocalCurvePoint>)>, PlanError> {
    let same = |a: &Breakpoint, b: &Breakpoint| {
        a.time_frames == b.time_frames && a.value.to_bits() == b.value.to_bits() && a.curve == b.curve
    };
    let mut present: HashSet<(u64, usize)> = HashSet::new();
    let mut additions: Vec<&Breakpoint> = Vec::new();
    for p in new_points {
        let copies: Vec<(u64, usize)> = lane
            .lane
            .points
            .iter()
            .zip(&lane.origins)
            .filter(|(old, _)| same(old, p))
            .map(|(_, o)| o.section_point())
            .collect();
        if copies.is_empty() {
            additions.push(p);
        } else {
            present.extend(copies);
        }
    }
    // (definition, beat bits) → (frame, point)
    let mut added: HashMap<(u64, u64), (u64, VocalCurvePoint)> = HashMap::new();
    for p in additions {
        let window = lane
            .window_at(p.time_frames)
            .ok_or(PlanError::OutsidePlacements {
                frame: p.time_frames,
            })?;
        let beat = window.beat_at(tempo_map, sample_rate, p.time_frames);
        let point = VocalCurvePoint {
            beat,
            value: p.value,
            stepped: p.curve == Shape::Stepped,
        };
        let key = (window.definition_id, beat.to_bits());
        if let Some((frame, existing)) = added.get(&key) {
            if *existing != point {
                return Err(PlanError::Conflict {
                    first: *frame,
                    second: p.time_frames,
                });
            }
            continue;
        }
        added.insert(key, (p.time_frames, point));
    }
    let mut definitions: Vec<u64> = lane.windows.iter().map(|w| w.definition_id).collect();
    definitions.sort_unstable();
    definitions.dedup();
    let mut out = Vec::with_capacity(definitions.len());
    for def in definitions {
        let old = current(def);
        // Points past the section end are not projected, so they have no
        // copy to keep them: they survive untouched. A section placed
        // under different meters is longer at some placements than at
        // others; a point is projected wherever it fits, so the longest
        // placement decides.
        let len = lane
            .windows
            .iter()
            .filter(|w| w.definition_id == def)
            .map(|w| w.len_beats)
            .fold(f64::NEG_INFINITY, f64::max);
        let mut curve: Vec<VocalCurvePoint> = old
            .iter()
            .enumerate()
            .filter(|(i, p)| p.beat > len || present.contains(&(def, *i)))
            .map(|(_, p)| *p)
            .collect();
        let mut mine: Vec<&(u64, VocalCurvePoint)> = added
            .iter()
            .filter(|((d, _), _)| *d == def)
            .map(|(_, v)| v)
            .collect();
        mine.sort_by(|a, b| a.1.beat.total_cmp(&b.1.beat));
        for (_, point) in mine {
            upsert_point(&mut curve, *point);
        }
        out.push((def, curve));
    }
    Ok(out)
}
