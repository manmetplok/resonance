//! Handlers for the project-scoped drum-pattern bank.
//!
//! Each pattern owns a `Vec<DrumGroup>`; sections pick a pattern via
//! their `SectionDefinitionState::arrangement` (the primary entry). The
//! manager modal targets
//! one *pattern* (`drumroll.managing_pattern_id`) and edits the groups
//! inside it. Section assignment, pattern CRUD (add / rename / delete /
//! duplicate) and the per-group generator knobs all live here.

use iced::Task;

use resonance_audio::types::{AudioCommand, MidiNote, TrackId, TrackType, TICKS_PER_QUARTER_NOTE};

use std::collections::HashMap;

use crate::compose::drumroll::{groups, DrumGroup, DrumGroupPad, DrumPattern, GROUP_PALETTE};
use crate::compose::messages::{ArrangementMessage, DrumGroupsMessage};
use crate::compose::SectionDefinitionState;
use crate::message::Message;
use crate::state::InstrumentType;
use crate::util::{next_seed, seed_from_id};

use super::regenerate::compose_samples_per_bar;

pub(super) fn handle(r: &mut crate::Resonance, msg: DrumGroupsMessage) -> Task<Message> {
    match msg {
        DrumGroupsMessage::SelectGroup { group_id } => {
            r.compose.drumroll.selected_group_id = Some(group_id);
        }

        DrumGroupsMessage::OpenManager => {
            // Default the manager focus to whatever the right rail has
            // selected so the user opens the modal with the same group
            // pre-highlighted. The pattern focus follows the section
            // selection so the modal lands on the same pattern the lane
            // is currently rendering.
            let section_pattern_id = section_pattern_id_for_focus(r);
            r.compose.drumroll.managing_pattern_id =
                section_pattern_id.or(r.compose.default_drum_pattern_id);
            let active = active_groups_for_manager(r);
            let focus = r
                .compose
                .drumroll
                .selected_group_id
                .filter(|id| active.iter().any(|g| g.id == *id))
                .or_else(|| active.first().map(|g| g.id));
            r.compose.drumroll.manager_open = true;
            r.compose.drumroll.managing_group_id = focus;
            r.compose.drumroll.manager_filter.clear();
        }
        DrumGroupsMessage::CloseManager => {
            r.compose.drumroll.manager_open = false;
            r.compose.drumroll.manager_filter.clear();
        }
        DrumGroupsMessage::ManagerSelectGroup { group_id } => {
            if active_groups_for_manager(r).iter().any(|g| g.id == group_id) {
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
            // Copy the defaults out first — the closure below holds the
            // mutable borrow of `r.compose`.
            let grid = r.compose.drumroll.base_grid.max(2);
            let cycle = r.compose.drumroll.base_cycle.max(1);
            let added = with_managing_pattern(r, |pattern| {
                let idx = pattern.groups.len();
                pattern.groups.push(DrumGroup {
                    id,
                    name: format!("Group {}", idx + 1),
                    color: GROUP_PALETTE[idx % GROUP_PALETTE.len()],
                    grid,
                    cycle,
                    phase: 0,
                    pads: Vec::new(),
                    density: 0.35,
                    swing: 0.0,
                    accent: 0.45,
                    humanize: 0.2,
                    fills: 0.0,
                    style: "Custom".to_string(),
                    seed: seed_from_id(id),
                });
            });
            if added.is_some() {
                r.compose.drumroll.managing_group_id = Some(id);
                r.compose.drumroll.selected_group_id = Some(id);
            }
        }
        DrumGroupsMessage::DeleteGroup { group_id } => {
            let fallback = with_managing_pattern(r, |pattern| {
                pattern.groups.retain(|g| g.id != group_id);
                pattern.groups.first().map(|g| g.id)
            })
            .flatten();
            if r.compose.drumroll.managing_group_id == Some(group_id) {
                r.compose.drumroll.managing_group_id = fallback;
            }
            if r.compose.drumroll.selected_group_id == Some(group_id) {
                r.compose.drumroll.selected_group_id = fallback;
            }
        }
        DrumGroupsMessage::RenameGroup { group_id, name } => {
            let _ = with_managing_pattern(r, |pattern| {
                if let Some(g) = pattern.groups.iter_mut().find(|g| g.id == group_id) {
                    g.name = name;
                }
            });
        }
        DrumGroupsMessage::SetGroupColor { group_id, color } => {
            let _ = with_managing_pattern(r, |pattern| {
                if let Some(g) = pattern.groups.iter_mut().find(|g| g.id == group_id) {
                    g.color = color;
                }
            });
        }
        DrumGroupsMessage::TogglePadAssignment { group_id, note } => {
            let _ = with_managing_pattern(r, |pattern| {
                toggle_pad(&mut pattern.groups, group_id, note);
            });
        }
        DrumGroupsMessage::ClearGroupPads { group_id } => {
            let _ = with_managing_pattern(r, |pattern| {
                if let Some(g) = pattern.groups.iter_mut().find(|g| g.id == group_id) {
                    g.pads.clear();
                }
            });
        }

        // ---- Pattern-bank CRUD ----
        DrumGroupsMessage::SelectPattern { pattern_id } => {
            if r.compose.find_pattern(pattern_id).is_some() {
                r.compose.drumroll.managing_pattern_id = Some(pattern_id);
                r.compose.drumroll.selected_group_id = r
                    .compose
                    .find_pattern(pattern_id)
                    .and_then(|p| p.groups.first().map(|g| g.id));
                r.compose.drumroll.managing_group_id = r.compose.drumroll.selected_group_id;
            }
        }
        DrumGroupsMessage::AssignPattern {
            definition_id,
            pattern_id,
        } => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                def.set_primary_pattern(pattern_id);
            }
            materialize_drum_clips(r);
        }
        DrumGroupsMessage::AddPattern => {
            let id = r.compose.fresh_id();
            let idx = r.compose.drum_patterns.len();
            let color = GROUP_PALETTE[idx % GROUP_PALETTE.len()];
            r.compose.drum_patterns.push(DrumPattern {
                id,
                name: format!("Pattern {}", idx + 1),
                color,
                groups: Vec::new(),
                length_bars: 1,
            });
            if r.compose.default_drum_pattern_id.is_none() {
                r.compose.default_drum_pattern_id = Some(id);
            }
            r.compose.drumroll.managing_pattern_id = Some(id);
            r.compose.drumroll.managing_group_id = None;
            r.compose.drumroll.selected_group_id = None;
        }
        DrumGroupsMessage::DuplicatePattern { pattern_id } => {
            let Some(src) = r.compose.find_pattern(pattern_id).cloned() else {
                return Task::none();
            };
            // Clone the groups but reassign every group id so the
            // duplicate doesn't share ids with the source — otherwise
            // toggling a step in the duplicate would also toggle the
            // source.
            let new_pattern_id = r.compose.fresh_id();
            let mut new_groups = src.groups.clone();
            for g in new_groups.iter_mut() {
                g.id = r.compose.fresh_id();
            }
            let copy = DrumPattern {
                id: new_pattern_id,
                name: format!("{} copy", src.name),
                color: src.color,
                groups: new_groups,
                length_bars: src.length_bars,
            };
            r.compose.drum_patterns.push(copy);
            r.compose.drumroll.managing_pattern_id = Some(new_pattern_id);
            r.compose.drumroll.managing_group_id = r
                .compose
                .find_pattern(new_pattern_id)
                .and_then(|p| p.groups.first().map(|g| g.id));
            r.compose.drumroll.selected_group_id = r.compose.drumroll.managing_group_id;
        }
        DrumGroupsMessage::DeletePattern { pattern_id } => {
            // Refuse to delete the last pattern — the lane needs at
            // least one to render.
            if r.compose.drum_patterns.len() <= 1 {
                r.compose.last_error =
                    Some("Cannot delete the last drum pattern".into());
                return Task::none();
            }
            r.compose.drum_patterns.retain(|p| p.id != pattern_id);
            // Drop any arrangement entry (or fill) that referenced the
            // deleted pattern so the lane keeps rendering — emptied
            // arrangements fall back to "use the default".
            for def in &mut r.compose.definitions {
                def.remove_pattern_references(pattern_id);
            }
            if r.compose.default_drum_pattern_id == Some(pattern_id) {
                r.compose.default_drum_pattern_id =
                    r.compose.drum_patterns.first().map(|p| p.id);
            }
            if r.compose.drumroll.managing_pattern_id == Some(pattern_id) {
                r.compose.drumroll.managing_pattern_id =
                    r.compose.default_drum_pattern_id;
            }
            r.compose.drumroll.selected_group_id = r
                .compose
                .drumroll
                .managing_pattern_id
                .and_then(|id| r.compose.find_pattern(id))
                .and_then(|p| p.groups.first().map(|g| g.id));
            r.compose.drumroll.managing_group_id = r.compose.drumroll.selected_group_id;
            materialize_drum_clips(r);
        }
        DrumGroupsMessage::RenamePattern { pattern_id, name } => {
            if let Some(pattern) = r.compose.find_pattern_mut(pattern_id) {
                pattern.name = name;
            }
        }
        DrumGroupsMessage::SetPatternColor { pattern_id, color } => {
            if let Some(pattern) = r.compose.find_pattern_mut(pattern_id) {
                pattern.color = color;
            }
        }
        DrumGroupsMessage::BeginRenamePattern { pattern_id } => {
            let initial = r
                .compose
                .find_pattern(pattern_id)
                .map(|p| p.name.clone())
                .unwrap_or_default();
            r.compose.drumroll.renaming_pattern_id = Some(pattern_id);
            r.compose.drumroll.renaming_pattern_text = initial;
        }
        DrumGroupsMessage::UpdateRenamePatternText(text) => {
            r.compose.drumroll.renaming_pattern_text = text;
        }
        DrumGroupsMessage::CommitRenamePattern => {
            if let Some(pattern_id) = r.compose.drumroll.renaming_pattern_id.take() {
                let text =
                    std::mem::take(&mut r.compose.drumroll.renaming_pattern_text);
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    if let Some(pattern) = r.compose.find_pattern_mut(pattern_id) {
                        pattern.name = trimmed.to_string();
                    }
                }
            }
        }
        DrumGroupsMessage::CancelRenamePattern => {
            r.compose.drumroll.renaming_pattern_id = None;
            r.compose.drumroll.renaming_pattern_text.clear();
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
            if let Some(g) = find_group_mut(r, group_id) {
                g.seed = next_seed(g.seed);
                generate_group_pattern(g);
            }
            materialize_drum_clips(r);
        }
        DrumGroupsMessage::GenerateAllGroups => {
            // Re-roll every group in every pattern. The user reaches
            // this via the lane's "Regenerate all" button so it should
            // refresh the whole bank, not just the focused pattern.
            for pattern in &mut r.compose.drum_patterns {
                for g in &mut pattern.groups {
                    g.seed = next_seed(g.seed);
                    generate_group_pattern(g);
                }
            }
            materialize_drum_clips(r);
        }
        DrumGroupsMessage::TogglePadStep {
            group_id,
            pad_index,
            step,
        } => {
            if let Some(g) = find_group_mut(r, group_id) {
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

/// Handle a [`ArrangementMessage`]: edit one section's ordered drum
/// arrangement. Every arm mutates the targeted section's `arrangement`
/// (via the pure [`SectionDefinitionState`] mutators) and, when something
/// actually changed, re-materializes the drum clips so playback follows.
/// The undo snapshot is taken upstream in `update()` before dispatch.
pub(super) fn handle_arrangement(
    r: &mut crate::Resonance,
    msg: ArrangementMessage,
) -> Task<Message> {
    let changed = match msg {
        ArrangementMessage::AddEntry {
            definition_id,
            pattern_id,
        } => {
            // Only append patterns that actually exist in the bank.
            if r.compose.find_pattern(pattern_id).is_none() {
                false
            } else {
                mutate_definition(r, definition_id, |def| {
                    def.add_entry(pattern_id);
                    true
                })
            }
        }
        ArrangementMessage::RemoveEntry {
            definition_id,
            index,
        } => mutate_definition(r, definition_id, |def| def.remove_entry(index)),
        ArrangementMessage::MoveEntry {
            definition_id,
            from,
            to,
        } => mutate_definition(r, definition_id, |def| def.move_entry(from, to)),
        ArrangementMessage::SetEntryLength {
            definition_id,
            index,
            length,
        } => mutate_definition(r, definition_id, |def| def.set_entry_length(index, length)),
        ArrangementMessage::SetEntryFill {
            definition_id,
            index,
            fill,
        } => {
            // A chosen fill must reference a real pattern; clearing (None)
            // is always allowed.
            if matches!(fill, Some(id) if r.compose.find_pattern(id).is_none()) {
                false
            } else {
                mutate_definition(r, definition_id, |def| def.set_entry_fill(index, fill))
            }
        }
        ArrangementMessage::DuplicateEntry {
            definition_id,
            index,
        } => mutate_definition(r, definition_id, |def| def.duplicate_entry(index)),
        ArrangementMessage::FillToEnd { definition_id } => fill_to_end(r, definition_id),
        ArrangementMessage::TrimToFit { definition_id } => trim_to_fit(r, definition_id),
    };

    if changed {
        materialize_drum_clips(r);
    }
    Task::none()
}

/// Look up a section definition by id and run `f` against it, returning
/// whatever `f` reports (and `false` when the section is gone). Centralises
/// the find-mut-or-bail dance every arrangement arm would otherwise repeat.
fn mutate_definition(
    r: &mut crate::Resonance,
    definition_id: u64,
    f: impl FnOnce(&mut SectionDefinitionState) -> bool,
) -> bool {
    r.compose
        .find_definition_mut(definition_id)
        .map(f)
        .unwrap_or(false)
}

/// Snapshot the bank's per-pattern bar lengths so the arrangement mutators
/// (which take `&mut` the section) can resolve `RepeatN` spans without
/// holding a second borrow of the pattern bank.
fn pattern_bar_lengths(r: &crate::Resonance) -> HashMap<u64, u32> {
    r.compose
        .drum_patterns
        .iter()
        .map(|p| (p.id, p.bar_span()))
        .collect()
}

fn fill_to_end(r: &mut crate::Resonance, definition_id: u64) -> bool {
    let lens = pattern_bar_lengths(r);
    // The fallback pattern (used when the arrangement is empty) is the
    // section's resolved default, so "fill to end" on a blank section lays
    // down the pattern the lane already renders.
    let fallback = r
        .compose
        .find_definition(definition_id)
        .and_then(|def| r.compose.pattern_for_definition(def))
        .map(|p| p.id);
    mutate_definition(r, definition_id, |def| {
        def.fill_to_end(|id| lens.get(&id).copied().unwrap_or(1), fallback)
    })
}

fn trim_to_fit(r: &mut crate::Resonance, definition_id: u64) -> bool {
    let lens = pattern_bar_lengths(r);
    mutate_definition(r, definition_id, |def| {
        def.trim_to_fit(|id| lens.get(&id).copied().unwrap_or(1))
    })
}

/// Resolve which pattern the manager is currently editing. Falls back
/// through the explicit pick → the project default → the first pattern
/// in the bank.
fn resolve_managing_pattern_id(r: &crate::Resonance) -> Option<u64> {
    r.compose
        .drumroll
        .managing_pattern_id
        .filter(|id| r.compose.drum_patterns.iter().any(|p| p.id == *id))
        .or(r.compose.default_drum_pattern_id)
        .or_else(|| r.compose.drum_patterns.first().map(|p| p.id))
}

/// Resolve the pattern the manager is editing (via
/// [`resolve_managing_pattern_id`]) and run `f` against it. Returns
/// `None` without running `f` when nothing resolves or the resolved id
/// is stale — the resolve → find → early-return dance every group CRUD
/// handler would otherwise repeat.
fn with_managing_pattern<T>(
    r: &mut crate::Resonance,
    f: impl FnOnce(&mut DrumPattern) -> T,
) -> Option<T> {
    let pattern_id = resolve_managing_pattern_id(r)?;
    let pattern = r.compose.find_pattern_mut(pattern_id)?;
    Some(f(pattern))
}

/// Pattern that the section selection points at. Used to open the
/// manager so it lands on the same pattern the lane is currently
/// rendering.
fn section_pattern_id_for_focus(r: &crate::Resonance) -> Option<u64> {
    let def = r
        .compose
        .selected_placement()
        .and_then(|p| r.compose.find_definition(p.definition_id))?;
    r.compose.pattern_for_definition(def).map(|p| p.id)
}

/// Slice of groups visible inside the manager modal — the groups owned
/// by the pattern resolved via [`resolve_managing_pattern_id`].
fn active_groups_for_manager(r: &crate::Resonance) -> &[DrumGroup] {
    let Some(id) = resolve_managing_pattern_id(r) else {
        return &[];
    };
    r.compose.find_pattern(id).map(|p| p.groups.as_slice()).unwrap_or(&[])
}

/// Find a group across every pattern in the bank. Group ids are unique
/// project-wide so this is unambiguous.
fn find_group_mut(r: &mut crate::Resonance, group_id: u64) -> Option<&mut DrumGroup> {
    for pattern in &mut r.compose.drum_patterns {
        if let Some(g) = pattern.groups.iter_mut().find(|g| g.id == group_id) {
            return Some(g);
        }
    }
    None
}

/// Mutate one group, looking it up by id across the full pattern bank.
/// Centralised so callers don't repeat the `iter_mut().find(...)` dance.
fn mutate_group(r: &mut crate::Resonance, group_id: u64, f: impl FnOnce(&mut DrumGroup)) {
    if let Some(g) = find_group_mut(r, group_id) {
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
        .map(|p| ((p.weight * base_hits as u32) / total_weight) as usize)
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

    // Snapshot one tuple per placement so we don't reborrow `r.compose`
    // inside the engine-send loop. Resolves each section to the groups
    // of its assigned drum pattern up front — sections with different
    // patterns produce different note sequences.
    let placements: Vec<(u64, u64, u32, u32, String, Vec<DrumGroup>)> = r
        .compose
        .placements
        .iter()
        .filter_map(|p| {
            let def = r.compose.find_definition(p.definition_id)?;
            let groups = r.compose.groups_for_definition(def).to_vec();
            Some((
                p.definition_id,
                p.id,
                p.start_bar,
                def.length_bars,
                def.name.clone(),
                groups,
            ))
        })
        .collect();
    if placements.is_empty() {
        return;
    }

    for (definition_id, placement_id, start_bar, length_bars, def_name, section_groups)
        in placements
    {
        let start_sample = start_bar as u64 * samples_per_bar;
        let duration_ticks = length_bars as u64 * time_sig_num as u64 * TICKS_PER_QUARTER_NOTE;
        let notes = build_drum_notes(&section_groups, length_bars, time_sig_num);

        for &track_id in &drum_track_ids {
            // Tear down any prior derived clip on this triple so we don't
            // stack copies. The engine collapses the delete + load in the
            // same audio tick so playback doesn't drop out.
            if let Some(old_id) = r
                .compose
                .derived_clips
                .remove(&(definition_id, placement_id, track_id))
            {
                let _ = r.engine
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
            let _ = r.engine.send(AudioCommand::LoadMidiClipDirect {
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
