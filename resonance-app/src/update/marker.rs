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

use crate::message::{MarkerMessage, Message};
use crate::state::ArrangementMarker;
use crate::Resonance;

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
