//! Performance-mode chord derivation (epic #11, todo #304).
//!
//! A pure, headless function that maps the transport playhead onto the
//! project's placed sections and their chord progressions, returning the
//! chord currently under the playhead, the next few upcoming chords, and
//! the beat/bar telemetry the Performance view needs.
//!
//! The function is intentionally free of any view or engine state: it
//! takes the placed sections, their definitions, the [`TempoMap`], and a
//! query (sample position + loop region) and returns a [`ChordReadout`].
//! All of the time math goes through the tempo map's bar table so tempo
//! and time-signature changes mid-take are honored.
//!
//! ## Beat model
//!
//! Chord positions ([`ChordState::start_beat`](crate::compose::ChordState)
//! / `duration_beats`) are in *grid beats* — one beat per time-signature
//! numerator slot, the same unit the Compose chord lane draws with (see
//! `view::compose::chord_lane`). A bar therefore contributes
//! `TempoMap::numerator_at_bar(bar)` grid beats. Mapping the playhead
//! sample onto this grid is done with [`TempoMap::sample_to_bar`], so the
//! readout stays correct under tempo ramps and time-signature changes.

use resonance_audio::types::TempoMap;
use resonance_music_theory::Chord;

use crate::compose::{SectionDefinitionState, SectionPlacementState};
use crate::state::ArrangementMarker;

/// How many upcoming chords the readout looks ahead by (design doc #151
/// calls for "the next 2–3 chords").
pub const UPCOMING_COUNT: usize = 3;

/// Small epsilon so a query sitting exactly on a slot boundary classifies
/// as "inside the later slot", not "just before the next one".
const BEAT_EPS: f64 = 1e-9;

/// A single chord slot resolved onto the absolute project bar grid.
#[derive(Debug, Clone, PartialEq)]
pub struct ChordSlot {
    /// Placement this slot belongs to.
    pub placement_id: u64,
    /// Section definition this slot's chord came from.
    pub definition_id: u64,
    /// Stable id of the [`ChordState`](crate::compose::ChordState).
    pub chord_id: u64,
    /// The chord itself (root / quality / slash bass).
    pub chord: Chord,
    /// Absolute, 0-based project bar where the slot begins.
    pub start_bar: u32,
    /// Beat offset of the slot within its section, in grid beats.
    pub start_beat_in_section: u32,
    /// Length of the slot in grid beats.
    pub duration_beats: u32,
    /// Absolute grid-beat position of the slot's start, measured from
    /// project bar 0. Lets the view place a slot on the global timeline
    /// without re-walking the bar grid.
    pub global_start_beat: u64,
}

/// The derived live-chord readout for one playhead position.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChordReadout {
    /// Chord under the playhead, or `None` in a gap / empty project.
    pub current: Option<ChordSlot>,
    /// The next [`UPCOMING_COUNT`] chords, nearest first. Honors the loop
    /// region (wraps at `loop_out` back to `loop_in`).
    pub upcoming: Vec<ChordSlot>,
    /// 1-based beat within the current bar.
    pub beat_in_bar: u32,
    /// Number of beats in the current bar (time-signature numerator).
    pub beats_per_bar: u32,
    /// Fractional position within the current beat, `0.0..1.0`.
    pub beat_phase: f64,
    /// Whole bars from the query position to the next chord change.
    /// `None` when there is no upcoming chord.
    pub bars_until_next: Option<u32>,
    /// Grid beats (fractional) until the next chord change. `None` when
    /// there is no upcoming chord.
    pub beats_until_next: Option<f64>,
    /// `true` when the readout was derived at a primed / pre-count
    /// position rather than the live playhead (count-in priming).
    pub priming: bool,
}

/// Inputs that vary per query (everything that isn't the section/tempo
/// data). Keeping this in a struct lets the call sites grow new transport
/// context without churning the function signature.
#[derive(Debug, Clone, Copy)]
pub struct ChordQuery {
    /// Live transport playhead, in samples.
    pub playhead: u64,
    /// Project sample rate (the one the bar table was built with).
    pub sample_rate: u32,
    /// During count-in, the sample position where the first chord will
    /// sound. When `Some`, the readout is derived at this position
    /// instead of `playhead`, and [`ChordReadout::priming`] is set — so
    /// the caller never special-cases the pre-count.
    pub primed_position: Option<u64>,
    /// `(loop_in, loop_out)` in samples when looping is active. Upcoming
    /// chords wrap at `loop_out` back to `loop_in`. `None` disables wrap.
    pub loop_region: Option<(u64, u64)>,
}

impl ChordQuery {
    /// A non-looping, non-priming query at `playhead`.
    pub fn at(playhead: u64, sample_rate: u32) -> Self {
        Self {
            playhead,
            sample_rate,
            primed_position: None,
            loop_region: None,
        }
    }

    /// The sample position the readout is actually evaluated at — the
    /// primed position during count-in, otherwise the live playhead.
    fn query_sample(&self) -> u64 {
        self.primed_position.unwrap_or(self.playhead)
    }
}

/// Sum of grid beats in bars `[0, bar)` — the absolute grid-beat position
/// of the start of `bar`.
fn bars_to_global_beats(tempo_map: &TempoMap, bar: u32) -> u64 {
    (0..bar).map(|b| tempo_map.numerator_at_bar(b) as u64).sum()
}

/// Absolute 0-based bar that contains the given absolute grid beat.
fn global_beat_to_bar(tempo_map: &TempoMap, global_beat: u64) -> u32 {
    let mut remaining = global_beat;
    let mut bar = 0u32;
    loop {
        let n = tempo_map.numerator_at_bar(bar) as u64;
        if n == 0 || remaining < n {
            return bar;
        }
        remaining -= n;
        bar += 1;
    }
}

/// Convert a sample position to a fractional absolute grid-beat position.
fn sample_to_global_beat(tempo_map: &TempoMap, sample: u64, sample_rate: u32) -> f64 {
    let (bar, frac) = tempo_map.sample_to_bar(sample, sample_rate);
    let num = tempo_map.numerator_at_bar(bar) as f64;
    bars_to_global_beats(tempo_map, bar) as f64 + frac * num
}

/// Build every chord slot across all placements, sorted by absolute
/// grid-beat start. A placement whose definition is missing is skipped.
fn resolve_slots(
    placements: &[SectionPlacementState],
    definitions: &[SectionDefinitionState],
    tempo_map: &TempoMap,
) -> Vec<ChordSlot> {
    let mut slots = Vec::new();
    for placement in placements {
        let Some(def) = definitions.iter().find(|d| d.id == placement.definition_id) else {
            continue;
        };
        let section_base = bars_to_global_beats(tempo_map, placement.start_bar);
        for chord in &def.chords {
            let global_start = section_base + chord.start_beat as u64;
            slots.push(ChordSlot {
                placement_id: placement.id,
                definition_id: def.id,
                chord_id: chord.id,
                chord: chord.chord,
                start_bar: global_beat_to_bar(tempo_map, global_start),
                start_beat_in_section: chord.start_beat,
                duration_beats: chord.duration_beats,
                global_start_beat: global_start,
            });
        }
    }
    slots.sort_by_key(|s| s.global_start_beat);
    slots
}

/// Derive the live-chord readout for one playhead position.
///
/// `placements` + `definitions` describe the arrangement; `tempo_map` maps
/// samples to bars/beats; `query` carries the playhead, sample rate, an
/// optional primed (count-in) position, and an optional loop region.
///
/// Returns the chord under the (effective) playhead, the next
/// [`UPCOMING_COUNT`] chords (loop-aware), the current beat-in-bar, and the
/// distance to the next chord change. In a gap — or an empty project —
/// `current` is `None` while `upcoming` still reports what comes next.
pub fn chord_readout(
    placements: &[SectionPlacementState],
    definitions: &[SectionDefinitionState],
    tempo_map: &TempoMap,
    query: ChordQuery,
) -> ChordReadout {
    let sr = query.sample_rate;
    let q_sample = query.query_sample();

    // Beat-in-bar telemetry, derived once from the effective sample.
    let (bar0, frac) = tempo_map.sample_to_bar(q_sample, sr);
    let beats_per_bar = tempo_map.numerator_at_bar(bar0) as u32;
    let beats_into_bar = frac * beats_per_bar as f64;
    let beat_in_bar = beats_into_bar.floor() as u32 + 1;
    let beat_phase = beats_into_bar.fract();
    let q_global = bars_to_global_beats(tempo_map, bar0) as f64 + beats_into_bar;

    let mut readout = ChordReadout {
        beat_in_bar,
        beats_per_bar,
        beat_phase,
        priming: query.primed_position.is_some(),
        ..ChordReadout::default()
    };

    let slots = resolve_slots(placements, definitions, tempo_map);
    if slots.is_empty() {
        return readout;
    }

    // Current chord: the slot whose [start, start + duration) grid-beat
    // span contains the query position.
    let current = slots.iter().find(|s| {
        let start = s.global_start_beat as f64;
        let end = (s.global_start_beat + s.duration_beats as u64) as f64;
        q_global + BEAT_EPS >= start && q_global + BEAT_EPS < end
    });
    let current_id = current.map(|s| s.chord_id);
    readout.current = current.cloned();

    // Loop region as grid beats, if active and non-empty.
    let loop_beats = query.loop_region.and_then(|(lo, hi)| {
        (hi > lo).then(|| {
            (
                sample_to_global_beat(tempo_map, lo, sr),
                sample_to_global_beat(tempo_map, hi, sr),
            )
        })
    });

    // Forward upcoming slots: strictly after the query position, never the
    // current slot, and (when looping) before the loop end.
    let mut upcoming: Vec<ChordSlot> = Vec::new();
    for s in &slots {
        if upcoming.len() >= UPCOMING_COUNT {
            break;
        }
        let start = s.global_start_beat as f64;
        if start <= q_global + BEAT_EPS || Some(s.chord_id) == current_id {
            continue;
        }
        if let Some((_, hi)) = loop_beats {
            if start >= hi - BEAT_EPS {
                break;
            }
        }
        upcoming.push(s.clone());
    }

    // Loop wrap: if we ran out before reaching the look-ahead count, restart
    // at loop_in and pick up the slots from the top of the loop (those at or
    // after the query position were already taken in the forward pass).
    if let Some((lo, _)) = loop_beats {
        for s in &slots {
            if upcoming.len() >= UPCOMING_COUNT {
                break;
            }
            let start = s.global_start_beat as f64;
            if start + BEAT_EPS < lo || start > q_global + BEAT_EPS {
                continue;
            }
            if Some(s.chord_id) == current_id {
                continue;
            }
            upcoming.push(s.clone());
        }
    }

    // Distance to the next chord change, measured from the query position to
    // the first upcoming slot — wrapping through the loop seam when the next
    // slot lies behind the playhead (a loop-around).
    if let Some(next) = upcoming.first() {
        let next_start = next.global_start_beat as f64;
        let beats_until = if next_start + BEAT_EPS >= q_global {
            next_start - q_global
        } else if let Some((lo, hi)) = loop_beats {
            (hi - q_global) + (next_start - lo)
        } else {
            next_start - q_global
        }
        .max(0.0);
        readout.beats_until_next = Some(beats_until);
        readout.bars_until_next = Some((beats_until / beats_per_bar.max(1) as f64).floor() as u32);
    }

    readout.upcoming = upcoming;
    readout
}

// -- Arrangement-marker section readout (todo #372) ---------------------------
//
// The Performance teleprompter also wants song *structure*: the section the
// playhead is inside and the one coming up. That comes straight from the
// arrangement markers (point flags or ranged section regions) — no engine or
// tempo math, just the markers' sample positions versus the playhead, with the
// same loop-aware windowing the chord look-ahead uses (wrap at `loop_out` back
// to `loop_in`).

/// A marker resolved into a teleprompter label.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionLabel {
    /// Stable id of the source [`ArrangementMarker`].
    pub id: u64,
    /// Section / marker name as shown to the performer.
    pub name: String,
    /// Marker colour, so the view can tint the label to match the timeline.
    pub color: [u8; 3],
}

impl SectionLabel {
    fn from_marker(m: &ArrangementMarker) -> Self {
        Self {
            id: m.id,
            name: m.name.clone(),
            color: m.color,
        }
    }
}

/// The derived section readout for one playhead position: where we are in the
/// arrangement and what comes next.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SectionReadout {
    /// Section under the playhead, or `None` in a gap / empty arrangement. A
    /// point marker stays current until a later-starting marker supersedes it;
    /// a ranged region is only current within `[start, effective_end]`.
    pub current: Option<SectionLabel>,
    /// The next upcoming section, honoring the loop region (wraps at
    /// `loop_out` back to the first section at/after `loop_in`). `None` when
    /// nothing comes next.
    pub next: Option<SectionLabel>,
}

/// Derive the current/next section labels from arrangement markers.
///
/// `markers` is the arrangement's marker list (need not be pre-sorted);
/// `playhead` is the transport position in samples; `loop_region` is
/// `(loop_in, loop_out)` in samples when looping is active (`None` disables the
/// wrap). Mirrors [`chord_readout`]'s loop windowing: the "next" section is the
/// nearest marker strictly after the playhead and before `loop_out`, falling
/// back to the first marker at/after `loop_in` when the look-ahead would
/// otherwise run off the end of the loop.
pub fn section_readout(
    markers: &[ArrangementMarker],
    playhead: u64,
    loop_region: Option<(u64, u64)>,
) -> SectionReadout {
    // Work on references sorted by start position so the windowing is stable
    // regardless of the caller's ordering.
    let mut sorted: Vec<&ArrangementMarker> = markers.iter().collect();
    sorted.sort_by_key(|m| m.start_sample);
    if sorted.is_empty() {
        return SectionReadout::default();
    }

    // Current: the latest marker that has started at or before the playhead.
    // A ranged region drops to "no section" once the playhead passes its end
    // (a true gap); a point marker has no extent, so it stays current until a
    // later marker takes over.
    let current = sorted
        .iter()
        .filter(|m| m.start_sample <= playhead)
        .next_back()
        .copied()
        .filter(|m| !(m.is_region() && playhead > m.effective_end()));
    let current_id = current.map(|m| m.id);

    // Loop window, only when active and non-empty.
    let loop_window = loop_region.and_then(|(lo, hi)| (hi > lo).then_some((lo, hi)));

    // Forward look-ahead: nearest marker strictly after the playhead, and —
    // when looping — strictly before the loop end.
    let forward = sorted.iter().copied().find(|m| {
        m.start_sample > playhead && loop_window.map_or(true, |(_, hi)| m.start_sample < hi)
    });

    // Loop wrap: if the forward pass found nothing left inside the loop,
    // restart at the top of the loop. The sections that come around again are
    // those in `[loop_in, playhead]` (the part of the loop already played this
    // cycle); take the first such section that isn't the one we're already in.
    let next = forward.or_else(|| {
        let (lo, _) = loop_window?;
        sorted
            .iter()
            .copied()
            .find(|m| m.start_sample >= lo && m.start_sample <= playhead && Some(m.id) != current_id)
    });

    SectionReadout {
        current: current.map(SectionLabel::from_marker),
        next: next.map(SectionLabel::from_marker),
    }
}
