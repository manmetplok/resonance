//! Handlers for the project-scoped drum-groups state.
//!
//! Owns the modal lifecycle, group-list CRUD, articulation-pad
//! assignment, and the right-rail generator knobs.

use iced::Task;

use resonance_audio::types::{AudioCommand, MidiNote, TrackId, TrackType, TICKS_PER_QUARTER_NOTE};

use crate::compose::drumroll::{groups, DrumGroup, DrumGroupPad, GROUP_PALETTE};
use crate::compose::messages::DrumGroupsMessage;
use crate::message::Message;
use crate::state::InstrumentType;

use super::regenerate::compose_samples_per_bar;

pub(super) fn handle(r: &mut crate::Resonance, msg: DrumGroupsMessage) -> Task<Message> {
    match msg {
        DrumGroupsMessage::SelectGroup { group_id } => {
            r.compose.drumroll.selected_group_id = Some(group_id);
        }

        DrumGroupsMessage::OpenManager => {
            // Default the manager focus to whatever the right rail has
            // selected so the user opens the modal with the same group
            // pre-highlighted.
            let focus = r
                .compose
                .drumroll
                .selected_group_id
                .filter(|id| r.compose.drum_groups.iter().any(|g| g.id == *id))
                .or_else(|| r.compose.drum_groups.first().map(|g| g.id));
            r.compose.drumroll.manager_open = true;
            r.compose.drumroll.managing_group_id = focus;
            r.compose.drumroll.manager_filter.clear();
        }
        DrumGroupsMessage::CloseManager => {
            r.compose.drumroll.manager_open = false;
            r.compose.drumroll.manager_filter.clear();
        }
        DrumGroupsMessage::ManagerSelectGroup { group_id } => {
            if r.compose.drum_groups.iter().any(|g| g.id == group_id) {
                r.compose.drumroll.managing_group_id = Some(group_id);
            }
        }
        DrumGroupsMessage::ManagerSetFilter(s) => {
            r.compose.drumroll.manager_filter = s;
        }
        DrumGroupsMessage::AddGroup => {
            let id = {
                r.compose.next_id += 1;
                r.compose.next_id
            };
            let idx = r.compose.drum_groups.len();
            let color = GROUP_PALETTE[idx % GROUP_PALETTE.len()];
            let group = DrumGroup {
                id,
                name: format!("Group {}", idx + 1),
                color,
                grid: r.compose.drumroll.base_grid.max(2),
                cycle: r.compose.drumroll.base_cycle.max(1),
                phase: 0,
                pads: Vec::new(),
                density: 0.35,
                swing: 0.0,
                accent: 0.45,
                humanize: 0.2,
                fills: 0.0,
                style: "Custom".to_string(),
                seed: id.wrapping_mul(0x9E3779B97F4A7C15),
            };
            r.compose.drum_groups.push(group);
            r.compose.drumroll.managing_group_id = Some(id);
            r.compose.drumroll.selected_group_id = Some(id);
        }
        DrumGroupsMessage::DeleteGroup { group_id } => {
            r.compose.drum_groups.retain(|g| g.id != group_id);
            if r.compose.drumroll.managing_group_id == Some(group_id) {
                r.compose.drumroll.managing_group_id = r.compose.drum_groups.first().map(|g| g.id);
            }
            if r.compose.drumroll.selected_group_id == Some(group_id) {
                r.compose.drumroll.selected_group_id = r.compose.drum_groups.first().map(|g| g.id);
            }
        }
        DrumGroupsMessage::RenameGroup { group_id, name } => {
            if let Some(g) = r.compose.drum_groups.iter_mut().find(|g| g.id == group_id) {
                g.name = name;
            }
        }
        DrumGroupsMessage::SetGroupColor { group_id, color } => {
            if let Some(g) = r.compose.drum_groups.iter_mut().find(|g| g.id == group_id) {
                g.color = color;
            }
        }
        DrumGroupsMessage::TogglePadAssignment { group_id, note } => {
            toggle_pad(&mut r.compose.drum_groups, group_id, note);
        }
        DrumGroupsMessage::ClearGroupPads { group_id } => {
            if let Some(g) = r.compose.drum_groups.iter_mut().find(|g| g.id == group_id) {
                g.pads.clear();
            }
        }

        // ---- Generator knobs ----
        DrumGroupsMessage::SetGroupGrid { group_id, grid } => {
            mutate_group(r, group_id, |g| {
                let grid = grid.clamp(2, 7);
                g.grid = grid;
            });
        }
        DrumGroupsMessage::SetGroupCycle { group_id, cycle } => {
            mutate_group(r, group_id, |g| {
                let cycle = cycle.clamp(1, 64);
                g.cycle = cycle;
                for pad in &mut g.pads {
                    pad.resize_pattern(cycle as usize);
                }
            });
        }
        DrumGroupsMessage::SetGroupMeter {
            group_id,
            grid,
            cycle,
        } => {
            mutate_group(r, group_id, |g| {
                let grid = grid.clamp(2, 7);
                let cycle = cycle.clamp(1, 64);
                g.grid = grid;
                g.cycle = cycle;
                for pad in &mut g.pads {
                    pad.resize_pattern(cycle as usize);
                }
            });
        }
        DrumGroupsMessage::SetGroupPhase { group_id, phase } => {
            mutate_group(r, group_id, |g| {
                let cap = g.cycle.max(1);
                g.phase = phase % cap;
            });
        }
        DrumGroupsMessage::SetGroupDensity { group_id, density } => {
            mutate_group(r, group_id, |g| g.density = density.clamp(0.0, 1.0));
        }
        DrumGroupsMessage::SetGroupSwing { group_id, swing } => {
            mutate_group(r, group_id, |g| g.swing = swing.clamp(0.0, 1.0));
        }
        DrumGroupsMessage::SetGroupAccent { group_id, accent } => {
            mutate_group(r, group_id, |g| g.accent = accent.clamp(0.0, 1.0));
        }
        DrumGroupsMessage::SetGroupHumanize { group_id, humanize } => {
            mutate_group(r, group_id, |g| g.humanize = humanize.clamp(0.0, 1.0));
        }
        DrumGroupsMessage::SetGroupFills { group_id, fills } => {
            mutate_group(r, group_id, |g| g.fills = fills.clamp(0.0, 1.0));
        }
        DrumGroupsMessage::SetPadWeight {
            group_id,
            pad_index,
            weight,
        } => {
            mutate_group(r, group_id, |g| {
                if let Some(p) = g.pads.get_mut(pad_index) {
                    p.weight = weight.clamp(0, 100);
                }
            });
        }
        DrumGroupsMessage::GenerateGroup { group_id } => {
            if let Some(g) = r.compose.drum_groups.iter_mut().find(|g| g.id == group_id) {
                g.seed = g.seed.wrapping_add(1).wrapping_mul(0x9E3779B97F4A7C15);
                generate_group_pattern(g);
            }
            materialize_drum_clips(r);
        }
        DrumGroupsMessage::GenerateAllGroups => {
            for g in &mut r.compose.drum_groups {
                g.seed = g.seed.wrapping_add(1).wrapping_mul(0x9E3779B97F4A7C15);
                generate_group_pattern(g);
            }
            materialize_drum_clips(r);
        }
        DrumGroupsMessage::TogglePadStep {
            group_id,
            pad_index,
            step,
        } => {
            if let Some(g) = r.compose.drum_groups.iter_mut().find(|g| g.id == group_id) {
                let cycle = g.cycle as usize;
                if cycle == 0 {
                    return Task::none();
                }
                let phase = g.phase as usize % cycle;
                let pattern_idx = (step + phase) % cycle;
                if let Some(pad) = g.pads.get_mut(pad_index) {
                    if let Some(cell) = pad.pattern.get_mut(pattern_idx) {
                        *cell = if *cell == 0 { 1 } else { 0 };
                    }
                }
            }
            materialize_drum_clips(r);
        }
    }
    Task::none()
}

/// Mutate one group, looking it up by id. Centralised so callers don't
/// repeat the `iter_mut().find(...)` dance.
fn mutate_group(r: &mut crate::Resonance, group_id: u64, f: impl FnOnce(&mut DrumGroup)) {
    if let Some(g) = r.compose.drum_groups.iter_mut().find(|g| g.id == group_id) {
        f(g);
    }
}

/// Toggle whether a pad (identified by MIDI note) belongs to a group.
/// Adding a pad to one group removes it from any other so a single pad
/// only ever lives in one place.
fn toggle_pad(groups: &mut [DrumGroup], target_group: u64, note: u8) {
    // First find whether the pad already lives in any group.
    let mut already_here = false;
    let mut existing_pad: Option<DrumGroupPad> = None;
    let mut existing_cycle: u32 = 0;
    for g in groups.iter_mut() {
        if let Some(pos) = g.pads.iter().position(|p| p.note == note) {
            if g.id == target_group {
                already_here = true;
            } else {
                existing_pad = Some(g.pads.remove(pos));
                existing_cycle = g.cycle;
                break;
            }
        }
    }

    let target = groups.iter_mut().find(|g| g.id == target_group);
    let Some(target) = target else {
        return;
    };

    if already_here {
        target.pads.retain(|p| p.note != note);
        return;
    }

    let name = kit_pad_name(note).unwrap_or_else(|| format!("Note {}", note));
    let mut pad = if let Some(mut prev) = existing_pad {
        // Reuse the moved pad's weight; resize its pattern so it matches
        // the new group's cycle. Don't carry the old pattern forward —
        // it was authored for a different polymeter and would surprise
        // the user.
        prev.name = name;
        prev.note = note;
        prev.pattern = vec![0u8; target.cycle as usize];
        // Keep the weight intact unless it was zero (zero-weight pads
        // are silent so they'd appear "removed" once moved).
        if prev.weight == 0 {
            prev.weight = 25;
        }
        let _ = existing_cycle;
        prev
    } else {
        DrumGroupPad {
            name,
            note,
            weight: 25,
            pattern: vec![0u8; target.cycle as usize],
        }
    };
    pad.resize_pattern(target.cycle as usize);
    target.pads.push(pad);
}

/// Look up a kit pad name by note number using the built-in kit pad
/// library. Falls back to `None` for unmapped notes so the caller can
/// generate a generic label.
fn kit_pad_name(note: u8) -> Option<String> {
    groups::default_kit_pads()
        .into_iter()
        .find(|p| p.note == note)
        .map(|p| p.name)
}

/// Lightweight pattern generator: fills every pad's `pattern` with a
/// hit density derived from the group's `density` knob, biased by the
/// pad's own articulation weight. Swing, accent, humanize, and fills are
/// surfaced as group-level parameters and stored on the group; they apply
/// at MIDI rendering time inside `regenerate.rs`.
///
/// The algorithm is a deterministic xorshift walk seeded by `seed * pad_index`
/// so repeated presses with the same seed produce identical results, which
/// is useful for testing and for showing "seed · 0x…" in the rail.
pub fn generate_group_pattern(g: &mut DrumGroup) {
    if g.pads.is_empty() || g.cycle == 0 {
        return;
    }
    let cycle = g.cycle as usize;
    let total_weight = g.pads.iter().map(|p| p.weight).sum::<u32>().max(1);
    // Total target hit count across the whole group, rounded up so even
    // tiny densities produce at least one hit. `cycle` slots, density 0..1,
    // and the kick / snare groups don't double-trigger because each pad's
    // share is allocated separately below.
    let base_hits = (cycle as f32 * g.density).round() as usize;
    let base_hits = base_hits.clamp(1, cycle);
    // Distribute hits to pads in proportion to weight.
    let mut allocs: Vec<usize> = g
        .pads
        .iter()
        .map(|p| ((p.weight * base_hits as u32) / total_weight).max(0) as usize)
        .collect();
    let assigned: usize = allocs.iter().sum();
    let mut leftover = base_hits.saturating_sub(assigned);
    if leftover > 0 {
        // Hand leftover hits to the heaviest pad first.
        let mut order: Vec<usize> = (0..g.pads.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(g.pads[i].weight));
        for i in order {
            if leftover == 0 {
                break;
            }
            allocs[i] += 1;
            leftover -= 1;
        }
    }

    for (pad_index, pad) in g.pads.iter_mut().enumerate() {
        pad.pattern = vec![0u8; cycle];
        let n = allocs[pad_index].min(cycle);
        if n == 0 {
            continue;
        }
        // Even-spaced Euclidean placement, rotated by an xorshift draw so
        // pads don't all line up on the same steps.
        let mut state = g.seed.wrapping_mul(pad_index as u64 + 17).max(1);
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let rotation = (state as usize) % cycle;
        for i in 0..n {
            let step = ((i * cycle) / n + rotation) % cycle;
            pad.pattern[step] = 1;
        }
    }
}

/// Materialise the project's drum-groups state into MIDI clips on every
/// drum track for every placement of every section definition.
///
/// Each group emits one note per non-zero pattern step per pad; the step
/// stride is `TICKS_PER_QUARTER_NOTE / group.grid` so triplet- and
/// septuplet-grid groups land at sub-beat positions inside the bar. The
/// pattern repeats every `group.cycle` steps, with `group.phase` shifting
/// the start. Velocity scales with the pad's articulation weight and the
/// group's accent knob on beat-start steps.
///
/// Replaces any prior derived MIDI clip on the (definition, placement,
/// track) triple so repeated Generate presses don't stack duplicates.
pub fn materialize_drum_clips(r: &mut crate::Resonance) {
    // Snapshot what we need so we don't borrow `r.compose` twice across
    // the engine sends.
    let drum_track_ids: Vec<TrackId> = r
        .registry
        .tracks
        .iter()
        .filter(|t| {
            matches!(t.track_type, TrackType::Instrument)
                && t.sub_track.is_none()
                && t.instrument_type == InstrumentType::Drum
        })
        .map(|t| t.id)
        .collect();
    if drum_track_ids.is_empty() {
        return;
    }

    let time_sig_num = r.transport.time_sig_num.max(1);
    let samples_per_bar = compose_samples_per_bar(r.sample_rate, r.transport.bpm, time_sig_num);

    let placements: Vec<(u64, u64, u32, u32, String)> = r
        .compose
        .placements
        .iter()
        .filter_map(|p| {
            let def = r.compose.find_definition(p.definition_id)?;
            Some((
                p.definition_id,
                p.id,
                p.start_bar,
                def.length_bars,
                def.name.clone(),
            ))
        })
        .collect();
    if placements.is_empty() {
        return;
    }

    let groups_snapshot = r.compose.drum_groups.clone();

    for (definition_id, placement_id, start_bar, length_bars, def_name) in placements {
        let start_sample = start_bar as u64 * samples_per_bar;
        let duration_ticks = length_bars as u64 * time_sig_num as u64 * TICKS_PER_QUARTER_NOTE;
        let notes = build_drum_notes(&groups_snapshot, length_bars, time_sig_num);

        for &track_id in &drum_track_ids {
            // Tear down any prior derived clip on this triple so we don't
            // stack copies. The engine collapses the delete + load in the
            // same audio tick so playback doesn't drop out.
            if let Some(old_id) = r
                .compose
                .derived_clips
                .remove(&(definition_id, placement_id, track_id))
            {
                r.engine
                    .send(AudioCommand::DeleteMidiClip { clip_id: old_id });
            }
            let track_name = r
                .registry
                .tracks
                .iter()
                .find(|t| t.id == track_id)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| "Drums".to_string());
            let name = format!("{} · {}", def_name, track_name);
            let clip_id = r.compose.fresh_derived_clip_id();
            r.engine.send(AudioCommand::LoadMidiClipDirect {
                clip_id,
                track_id,
                start_sample,
                duration_ticks,
                notes: notes.clone(),
                name,
                trim_start_ticks: 0,
                trim_end_ticks: 0,
            });
            r.compose
                .derived_clips
                .insert((definition_id, placement_id, track_id), clip_id);
        }
    }
    r.compose.last_error = None;
}

/// Walk every group's pad patterns across `length_bars` and emit a `MidiNote`
/// for each hit. Step stride respects each group's `grid` (steps per beat)
/// and pattern wraps every `cycle` steps with optional `phase` offset.
fn build_drum_notes(
    groups: &[DrumGroup],
    length_bars: u32,
    time_sig_num: u8,
) -> Vec<MidiNote> {
    let mut notes: Vec<MidiNote> = Vec::new();
    let bars = length_bars.max(1) as u64;
    for g in groups {
        if g.pads.is_empty() || g.cycle == 0 || g.grid == 0 {
            continue;
        }
        // Step length in ticks. For grids that don't divide 480 evenly
        // (5, 7, etc.) we rely on integer rounding; the audio engine
        // tolerates the small drift and these are explicitly polyrhythmic
        // grids where exact tick alignment isn't expected.
        let step_ticks = (TICKS_PER_QUARTER_NOTE / g.grid as u64).max(1);
        let steps_per_bar = time_sig_num as u64 * g.grid as u64;
        let total_steps = bars * steps_per_bar;
        let cycle = g.cycle as u64;
        let phase = g.phase as u64 % cycle.max(1);
        for step in 0..total_steps {
            let pattern_idx = ((step + phase) % cycle) as usize;
            let start_tick = step * step_ticks;
            for pad in &g.pads {
                let cell = pad
                    .pattern
                    .get(pattern_idx)
                    .copied()
                    .unwrap_or(0);
                if cell == 0 {
                    continue;
                }
                // Beat-start steps get an accent bump scaled by the
                // group's accent knob.
                let is_beat_start = step % g.grid as u64 == 0;
                let base = 0.70 + (pad.weight as f32 / 200.0).min(0.25);
                let velocity = if is_beat_start {
                    (base + g.accent * 0.25).clamp(0.0, 1.0)
                } else {
                    base.clamp(0.0, 1.0)
                };
                notes.push(MidiNote {
                    note: pad.note,
                    velocity,
                    start_tick,
                    duration_ticks: step_ticks,
                });
            }
        }
    }
    // Sort by start_tick so the engine sees notes in monotonic order —
    // some downstream code assumes this for fast playhead lookups.
    notes.sort_by_key(|n| (n.start_tick, n.note));
    notes
}
