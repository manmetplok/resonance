pub mod canvas;
pub mod pattern_picker;

use iced::widget::{column, container, Canvas, Space};
use iced::{Element, Length};

use resonance_audio::types::{ClipId, TrackId};

use crate::compose::{ArrangementSpan, ComposeState, SectionDefinitionState, SectionPlacementState};
use crate::message::Message;
use crate::state::InstrumentType;
use crate::Resonance;

pub use canvas::{drum_lane_height, sorted_drum_tracks, BarSpanView, ComposeDrumCanvas};
pub use pattern_picker::pattern_picker;

/// Resolve a section's drum arrangement into a `Vec<BarSpanView>` ready for
/// the canvas.
///
/// This is the pure construction step separated from the Iced view function
/// so it can be unit-tested. Logic:
///
/// - If `resolve_arrangement_for` returns no explicit spans (empty arrangement)
///   **or** only spans that leave some bars uncovered, gap bars are filled
///   with the primary pattern's color and groups.
/// - Each resolved span is mapped to a `BarSpanView` that borrows the
///   pattern's groups and color directly from `compose`.
/// - A trailing gap (bars after the last explicit span) is also filled with
///   the primary pattern.
///
/// Single-entry arrangements produce exactly one span identical to the old
/// single-pattern rendering.
pub fn build_bar_spans<'a>(
    compose: &'a ComposeState,
    definition: &'a SectionDefinitionState,
) -> Vec<BarSpanView<'a>> {
    let section_bars = definition.length_bars;
    let resolved = compose.resolve_arrangement_for(definition);
    let primary_pattern = compose.pattern_for_definition(definition);

    let raw_spans: Vec<ArrangementSpan> = if resolved.spans.is_empty() {
        match primary_pattern {
            Some(p) => vec![ArrangementSpan {
                bar_start: 0,
                bar_end: section_bars.max(1),
                pattern_id: p.id,
                is_fill: false,
            }],
            None => Vec::new(),
        }
    } else {
        resolved.spans
    };

    let primary_color = primary_pattern
        .map(|p| p.color)
        .unwrap_or([0x80, 0x80, 0x80]);
    let primary_groups_ref = primary_pattern
        .map(|p| p.groups.as_slice())
        .unwrap_or(&[]);

    let mut bar_spans: Vec<BarSpanView<'a>> = Vec::with_capacity(raw_spans.len() + 1);
    let mut cursor = 0u32;

    for span in &raw_spans {
        // Gap before this span — fill with primary pattern.
        if cursor < span.bar_start {
            bar_spans.push(BarSpanView {
                bar_start: cursor,
                bar_end: span.bar_start,
                pattern_color: primary_color,
                pattern_groups: primary_groups_ref,
                is_fill: false,
            });
        }
        // Look up the span's own pattern for its color + groups.
        let (color, groups_ref) = compose
            .find_pattern(span.pattern_id)
            .map(|p| (p.color, p.groups.as_slice()))
            .unwrap_or((primary_color, primary_groups_ref));
        bar_spans.push(BarSpanView {
            bar_start: span.bar_start,
            bar_end: span.bar_end,
            pattern_color: color,
            pattern_groups: groups_ref,
            is_fill: span.is_fill,
        });
        cursor = span.bar_end;
    }
    // Trailing gap — fill with primary pattern.
    if cursor < section_bars {
        bar_spans.push(BarSpanView {
            bar_start: cursor,
            bar_end: section_bars,
            pattern_color: primary_color,
            pattern_groups: primary_groups_ref,
            is_fill: false,
        });
    }

    bar_spans
}

/// Build the drumroll block. Returns an empty 0-height container when the
/// project has no drum tracks so synth-only projects pay no visual cost.
///
/// Each drum track gets its own grouped canvas. The canvas resolves the
/// section's drum arrangement per-bar so cells in each bar come from that
/// bar's pattern (design item 3 'DRUM GRID context'): a chained
/// arrangement produces distinct pattern tints and separator lines; a
/// single-entry arrangement looks exactly like the old single-pattern
/// rendering.
///
/// Resolution is delegated to
/// [`crate::compose::ComposeState::resolve_arrangement_for`] (the #483
/// resolver). Gap bars (trailing bars not covered by any explicit entry)
/// fall back to the section's primary/default pattern. The bar-span list
/// is built once here and passed into every per-track canvas so each track
/// shares the same pre-resolved geometry.
pub fn view<'a>(
    app: &'a Resonance,
    placement: &'a SectionPlacementState,
    definition: &'a SectionDefinitionState,
) -> Element<'a, Message> {
    let drum_tracks = sorted_drum_tracks(&app.registry.tracks);

    if drum_tracks.is_empty() {
        return container(Space::new().height(0)).width(Length::Fill).into();
    }

    let width = super::workspace_width(
        &app.tempo_map,
        placement.start_bar,
        definition.length_bars,
    );

    let section_bars = definition.length_bars;

    // Primary groups: set the lane height + row structure for the section.
    let section_groups = app.compose.groups_for_definition(definition);
    let total_height = drum_lane_height(section_groups);
    let track_selected = matches!(
        app.compose.selected_lane,
        crate::compose::SelectedLane::Drums(_)
    );
    let selected_track_id = match app.compose.selected_lane {
        crate::compose::SelectedLane::Drums(id) => Some(id),
        _ => None,
    };

    // Resolve the section's drum arrangement into per-bar BarSpanViews.
    // Delegates to the extracted helper so the construction logic can be
    // unit-tested independently of the Iced widget tree.
    let bar_spans = build_bar_spans(&app.compose, definition);

    let picker = pattern_picker(app, definition, width);

    let mut rows: Vec<Element<'a, Message>> = Vec::with_capacity(drum_tracks.len() + 1);
    rows.push(picker);
    for track in &drum_tracks {
        // Each track gets its own canvas but they all share the same
        // resolved bar_spans so the pattern tints + cell data stay in
        // sync. Clone is cheap: bar_spans holds slices into the pattern
        // bank (borrowed from app), no heap allocation per span.
        let spans_clone: Vec<BarSpanView<'a>> = bar_spans
            .iter()
            .map(|s| BarSpanView {
                bar_start: s.bar_start,
                bar_end: s.bar_end,
                pattern_color: s.pattern_color,
                pattern_groups: s.pattern_groups,
                is_fill: s.is_fill,
            })
            .collect();
        let canvas_prog = ComposeDrumCanvas {
            track,
            groups: section_groups,
            selected_group_id: app.compose.drumroll.selected_group_id,
            track_selected: track_selected && selected_track_id == Some(track.id),
            bar_spans: spans_clone,
            section_bars,
        };
        rows.push(
            container(
                Canvas::new(canvas_prog)
                    .width(Length::Fixed(width))
                    .height(Length::Fixed(total_height)),
            )
            .width(Length::Fixed(width))
            .height(Length::Fixed(total_height))
            .into(),
        );
    }

    column(rows).into()
}

/// Look up the first MIDI clip on `track_id` that overlaps the current
/// section. Used by the sidebar controls to decide whether the
/// Apply/Clear buttons can target a clip yet. (Kept for compatibility
/// with the old API — the grouped lane no longer requires a clip to
/// operate, but the inspector still uses this to enable optional
/// per-clip actions.)
pub fn clip_for_track(
    app: &Resonance,
    placement: &SectionPlacementState,
    definition: &SectionDefinitionState,
    track_id: TrackId,
) -> Option<ClipId> {
    let _ = InstrumentType::Drum;
    let section_start = app.tempo_map.bar_to_sample(placement.start_bar);
    let section_end =
        app.tempo_map.bar_to_sample(placement.start_bar + definition.length_bars);
    app.midi_clips.iter().find_map(|clip| {
        if clip.track_id != track_id {
            return None;
        }
        let clip_end = app.tempo_map.tick_to_abs_sample(
            clip.start_sample,
            clip.duration_ticks,
            app.sample_rate,
        );
        (clip_end > section_start && clip.start_sample < section_end).then_some(clip.id)
    })
}
