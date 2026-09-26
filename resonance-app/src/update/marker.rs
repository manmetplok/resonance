//! Arrangement-marker reducers.
//!
//! Mutating variants edit `Resonance::markers` (kept sorted by start
//! sample) and rely on the undo classifier in `undo.rs` recording a
//! pre-dispatch snapshot — markers ride the same `ProjectFile`
//! save/replay path the snapshot machinery uses, so no bespoke undo
//! bookkeeping is needed here. Navigation variants only drive the
//! transport (seek / play) and emit no marker state change.

use iced::Task;
use resonance_audio::types::AudioCommand;

use crate::message::Message;
use crate::state::ArrangementMarker;
use crate::Resonance;

#[derive(Debug, Clone)]
pub enum MarkerMessage {
    /// Drop a new point marker at the current playhead (snapped to the
    /// grid via `snap_sample_to_grid_tempo`).
    AddAtPlayhead,
    /// Replace all section-seeded markers with a fresh set derived from
    /// the current Compose section placements — one ranged marker per
    /// placement, named/coloured from its section definition. Markers the
    /// user placed by hand are left untouched.
    SeedFromSections,
    /// Rename the marker with the given id.
    Rename(u64, String),
    /// Recolor the marker with the given id.
    Recolor(u64, [u8; 3]),
    /// Delete the marker with the given id.
    Delete(u64),
    /// Move a marker's start to a new sample position (snapped to the
    /// grid). The collection re-sorts after the move.
    MoveStart(u64, u64),
    /// Set (or clear, with `None`) a marker's region end, turning a
    /// point marker into a ranged region and back.
    SetRegionEnd(u64, Option<u64>),
    /// Move the playhead to the next marker after the playhead.
    JumpToNext,
    /// Move the playhead to the previous marker before the playhead.
    JumpToPrev,
    /// Move the playhead to a specific marker.
    JumpTo(u64),
    /// Set the loop range to a marker's region and enable looping. A
    /// ranged marker loops over `[start, end]`; a point marker loops
    /// from its start to the next marker's start.
    LoopToRegion(u64),
    /// Seek to a marker and start playback.
    PlayFromMarker(u64),
}

impl MarkerMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::{CoalesceKey, UndoAction};
        match self {
            // Mutating edits: record an undo entry capturing the
            // pre-edit marker set (markers ride the ProjectFile
            // snapshot/replay path). `LoopToRegion` mutates the loop
            // range, matching `ToggleLoop`'s classification.
            Self::AddAtPlayhead
            | Self::Rename(_, _)
            | Self::Recolor(_, _)
            | Self::Delete(_)
            | Self::LoopToRegion(_)
            | Self::SeedFromSections => UndoAction::Record,
            // Drag gestures: a marker move or a region-edge resize fires
            // one message per pointer step, so coalesce each gesture into a
            // single undo entry keyed by marker id (mirrors fader / knob
            // bursts). A one-off convert-to-region / point still records a
            // lone entry — nothing to coalesce it with.
            Self::MoveStart(id, _) => UndoAction::RecordCoalesced(CoalesceKey::MarkerMove(*id)),
            Self::SetRegionEnd(id, _) => {
                UndoAction::RecordCoalesced(CoalesceKey::MarkerResize(*id))
            }
            // Navigation only — moves the playhead / starts playback,
            // no project mutation, mirroring `SeekToSample` / `Play`.
            Self::JumpToNext | Self::JumpToPrev | Self::JumpTo(_) | Self::PlayFromMarker(_) => {
                UndoAction::Skip
            }
        }
    }
}

/// Default colours handed out to freshly-added markers, cycled by the
/// current marker count so a sequence of `AddAtPlayhead` actions yields
/// visually distinct flags without the user picking a colour each time.
const MARKER_PALETTE: [[u8; 3]; 6] = [
    [0xE5, 0x4B, 0x4B], // red
    [0xE5, 0x9B, 0x33], // orange
    [0xE5, 0xD0, 0x33], // yellow
    [0x5C, 0xC4, 0x6B], // green
    [0x3D, 0x8B, 0xE5], // blue
    [0x9B, 0x5C, 0xE5], // violet
];

pub fn handle(r: &mut Resonance, m: MarkerMessage) -> Task<Message> {
    match m {
        MarkerMessage::AddAtPlayhead => {
            let start = snap(r, r.transport.playhead);
            let id = r.markers.allocate_id();
            let color = MARKER_PALETTE[r.markers.len() % MARKER_PALETTE.len()];
            let name = format!("Marker {id}");
            r.markers
                .add(ArrangementMarker::new_point(id, name, color, start));
        }
        MarkerMessage::SeedFromSections => seed_from_sections(r),
        MarkerMessage::Rename(id, name) => {
            if let Some(marker) = r.markers.get_mut(id) {
                marker.name = name;
            }
        }
        MarkerMessage::Recolor(id, color) => {
            if let Some(marker) = r.markers.get_mut(id) {
                marker.color = color;
            }
        }
        MarkerMessage::Delete(id) => {
            r.markers.remove(id);
        }
        MarkerMessage::MoveStart(id, sample) => {
            let snapped = snap(r, sample);
            r.markers.move_start(id, snapped);
        }
        MarkerMessage::SetRegionEnd(id, end_sample) => {
            r.markers.set_region_end(id, end_sample);
        }
        MarkerMessage::JumpToNext => {
            if let Some(pos) = r.markers.next_marker(r.transport.playhead).map(|m| m.start_sample)
            {
                seek(r, pos);
            }
        }
        MarkerMessage::JumpToPrev => {
            if let Some(pos) = r.markers.prev_marker(r.transport.playhead).map(|m| m.start_sample)
            {
                seek(r, pos);
            }
        }
        MarkerMessage::JumpTo(id) => {
            if let Some(pos) = r.markers.get(id).map(|m| m.start_sample) {
                seek(r, pos);
            }
        }
        MarkerMessage::LoopToRegion(id) => {
            loop_to_region(r, id);
        }
        MarkerMessage::PlayFromMarker(id) => {
            if let Some(pos) = r.markers.get(id).map(|m| m.start_sample) {
                seek(r, pos);
                let _ = r.engine.send(AudioCommand::Play);
                r.transport.playing = true;
            }
        }
    }
    Task::none()
}

/// Replace every section-seeded marker with a fresh set derived from the
/// current Compose section placements: one ranged marker per placement,
/// name + colour copied from the referenced section definition and span
/// computed from `start_bar` + `length_bars` via the tempo map. Markers
/// the user placed by hand (`seeded == false`) are left untouched, so a
/// re-seed after editing the arrangement stays idempotent. Placements
/// whose definition is missing are skipped.
fn seed_from_sections(r: &mut Resonance) {
    // Resolve each placement to a ranged-marker spec up front so the
    // marker mutation below doesn't overlap the compose / tempo-map borrows.
    let seeds: Vec<(String, [u8; 3], u64, u64)> = r
        .compose
        .placements
        .iter()
        .filter_map(|p| {
            let def = r.compose.find_definition(p.definition_id)?;
            let start = r.tempo_map.bar_to_sample(p.start_bar);
            let end = r.tempo_map.bar_to_sample(p.start_bar + def.length_bars);
            Some((def.name.clone(), def.color, start, end))
        })
        .collect();

    // Clear the previously-seeded markers, keeping hand-placed ones, then
    // rebuild from the freshly-resolved specs.
    r.markers.markers.retain(|m| !m.seeded);
    for (name, color, start, end) in seeds {
        let id = r.markers.allocate_id();
        r.markers.add(ArrangementMarker::new_seeded_region(
            id, name, color, start, end,
        ));
    }
}

/// Snap a raw sample position to the grid using the live tempo / zoom,
/// the same helper the transport loop-drag and clip handlers use.
fn snap(r: &Resonance, sample: u64) -> u64 {
    crate::view::timeline::snap_sample_to_grid_tempo(
        sample,
        r.transport.bpm,
        r.transport.time_sig_num,
        r.sample_rate,
        r.viewport.zoom,
        &r.tempo_map,
    )
}

/// Move the playhead to `pos` and tell the engine to seek there.
fn seek(r: &mut Resonance, pos: u64) {
    let _ = r.engine.send(AudioCommand::SeekTo(pos));
    r.transport.playhead = pos;
}

/// Set the loop range to a marker's region and enable looping. A ranged
/// marker loops over `[start, end]`; a point marker loops from its start
/// to the next marker's start. No-op when the marker is missing, or when
/// a point marker has no following marker to bound the loop.
fn loop_to_region(r: &mut Resonance, id: u64) {
    let Some((start, end_sample)) = r.markers.get(id).map(|m| (m.start_sample, m.end_sample))
    else {
        return;
    };
    let loop_out = match end_sample {
        Some(end) => end,
        None => {
            // Point marker: loop to the next marker's start. `markers`
            // is sorted, so the first start strictly greater than this
            // one is the next flag.
            match r
                .markers
                .iter()
                .find(|m| m.start_sample > start)
                .map(|m| m.start_sample)
            {
                Some(next) => next,
                None => return,
            }
        }
    };
    if loop_out <= start {
        return;
    }
    r.transport.loop_in = start;
    r.transport.loop_out = loop_out;
    r.transport.loop_enabled = true;
    r.transport.loop_range_set = true;
    let _ = r.engine.send(AudioCommand::SetLoopRange {
        enabled: true,
        loop_in: start,
        loop_out,
    });
}
