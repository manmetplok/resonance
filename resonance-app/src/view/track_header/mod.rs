//! Track header column shown on the left of the Arrange view. Matches
//! the redesign: 28×28 instrument glyph + name/kind stacked column +
//! a slim 5-button row (freeze / mute / solo / arm / monitor). Selection
//! paints a 2px lavender stripe on the left edge.
//!
//! Mono toggle, FX bypass, and bounce-in-place stay on the mixer strip
//! where channel-strip controls live; the Arrange header is for arranging.
//!
//! The header column mirrors the timeline canvas's vertical layout
//! row-for-row so each header stays glued to its lane during vertical
//! scrolling. Internally it's a two-layer `stack`:
//!
//! - **Lane subtree** (base layer): a `chrome_h`-tall transparent
//!   spacer followed by the lane area. The lane area applies the
//!   partial-top fractional offset as a negative top padding so the
//!   first visible row scrolls smoothly instead of snapping to row
//!   boundaries. The rows themselves come from the shared
//!   [`ArrangeRowLayout`](crate::view::arrange_layout) (todo #729),
//!   which interleaves 60 px group-header rows with 96 px track lanes
//!   and hides the members of collapsed groups. Variable-height
//!   virtualization selects rows by cumulative Y — dropping rows fully
//!   above `scroll_offset_y` AND fully below
//!   `scroll_offset_y + viewport_lane_h` — so a 200-track session only
//!   allocates a handful of `view_group_header` / `view_track_header`
//!   subtrees.
//! - **Chrome subtree** (top layer): ruler + section-band placeholder
//!   (when sections exist) + always-visible 32 px global-shelf header +
//!   per-lane labels (chords / tempo / signature) when expanded, then
//!   `Space::Fill` for the remainder. The chrome rows all paint opaque
//!   backgrounds, so they mask any track-row bleed coming from the
//!   negatively-padded lane subtree below.
//!
//! Layering matters: iced 0.14's `container.clip(true)` narrows the
//! `viewport` passed to child `draw()` calls, but does **not** clip
//! child `fill_quad` backgrounds. A negatively-padded `view_track_header`
//! would otherwise paint its background and clip body over the ruler /
//! section band / global shelf at sub-row scroll positions. See
//! `tests/track_header_alignment.rs::track_header_no_bleed_into_chrome_expanded`
//! for the regression that locks this in.
//!
//! Sub-modules split the column per sub-region:
//! - [`track`] — the per-track header cells in the lane area.
//! - [`shelf`] — the global-shelf header strip + the lane labels
//!   (chords / tempo / signature) in the chrome.
pub(crate) mod group_header;
mod shelf;
pub(crate) mod track;

use iced::widget::{column, container, stack, Space};
use iced::{Element, Length, Padding};

use crate::message::*;
use crate::state::TrackState;
use crate::theme;
use crate::view::arrange_layout::{ArrangeRowKind, ArrangeRowLayout};
use crate::Resonance;

/// Approximate height of the open track context menu (todo #581): six
/// ~23 px rows + two separators + border. Used only to clamp the menu's
/// anchor so a menu opened on a bottom-edge row stays fully visible.
const TRACK_MENU_HEIGHT: f32 = 160.0;

/// Whether the header-cap "Freeze all" button (todo #581) has anything to
/// do: some freezable track is still live (idle or failed) — the same
/// filter `FreezeMessage::FreezeAllTracks` applies when building its batch.
fn freeze_all_enabled(r: &Resonance) -> bool {
    crate::update::freeze::freezable_tracks(r).iter().any(|id| {
        matches!(
            r.freeze.status(*id),
            crate::state::FreezeStatus::Idle | crate::state::FreezeStatus::Failed { .. }
        )
    })
}

pub(crate) fn view_track_headers(r: &Resonance) -> Element<'_, Message> {
    let fp = track_headers_fingerprint(r);
    // Lazy-cache the entire column: nothing in the arrange track-header
    // strip updates per audio tick, so the closure only re-runs on user
    // input (mute/solo/scroll/etc.). A continuous window resize reuses
    // the cached tree across every paint.
    iced::widget::lazy(fp, move |_: &u64| -> Element<'static, Message> {
        build_track_headers(r)
    })
    .into()
}

/// Hashes every arrange-track-header-visible piece of state. Levels
/// aren't shown in the arrange header column, so the level fields
/// don't enter the fingerprint and the lazy widget skips redraws when
/// only meters tick.
fn track_headers_fingerprint(r: &Resonance) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    r.viewport.global_tracks_expanded.hash(&mut h);
    r.viewport.scroll_offset_y.to_bits().hash(&mut h);
    // The browser panel offsets the arrange-area x of the whole column;
    // the per-row context-menu anchor (todo #581) is baked into each row's
    // `OpenTrackMenu` message, so a browser toggle must rebuild the tree.
    r.browser.visible.hash(&mut h);
    // Manual track-list virtualization (`build_track_headers`) drops
    // tracks below `scroll_offset_y + viewport_height` from the widget
    // tree; the cache must invalidate on window resize so previously-
    // off-screen rows materialize when the viewport grows.
    r.viewport.viewport_height.to_bits().hash(&mut h);
    // Whether the section band exists changes the fixed-header height
    // of the column, so it must invalidate the lazy cache.
    (!r.compose.placements.is_empty()).hash(&mut h);
    r.interaction.selected_track.hash(&mut h);
    // Multi-track selection drives both the per-row highlight and the
    // group-member indent, so the lazy cache must invalidate when it
    // changes or when a group is (un)formed from it.
    r.interaction.selected_tracks.hash(&mut h);
    for g in r.track_groups.get_all_groups_sorted() {
        g.id.hash(&mut h);
        g.ordered_members.hash(&mut h);
        g.nesting_parent.hash(&mut h);
        // Group-header rows now render inline in the column (todo #730), so
        // everything `view_group_header` paints — plus collapse state, which
        // drives the variable-height ArrangeRowLayout (collapsed groups hide
        // their members) — must invalidate the lazy cache.
        g.is_collapsed.hash(&mut h);
        g.name.hash(&mut h);
        g.identity_color.hash(&mut h);
        g.macro_mute.hash(&mut h);
        g.macro_solo.hash(&mut h);
        g.macro_level.to_bits().hash(&mut h);
    }
    r.interaction.selected_global_event.hash(&mut h);
    r.transport.time_sig_num.hash(&mut h);
    r.transport.time_sig_den.hash(&mut h);
    r.transport.bpm.to_bits().hash(&mut h);
    // tempo / signature event positions affect the global-tracks header
    // row when expanded; cheap to hash because the lists are small.
    r.tempo_events.len().hash(&mut h);
    for ev in &r.tempo_events {
        ev.bar.hash(&mut h);
        ev.bpm.to_bits().hash(&mut h);
    }
    r.signature_events.len().hash(&mut h);
    for ev in &r.signature_events {
        ev.bar.hash(&mut h);
        ev.numerator.hash(&mut h);
        ev.denominator.hash(&mut h);
    }
    // Chord-lane sub-line shows the total chord count across sections —
    // hash so the column-side label refreshes when chords are added or
    // re-rolled in Compose.
    let chord_total: usize = r
        .compose
        .definitions
        .iter()
        .map(|d| d.chords.len())
        .sum();
    chord_total.hash(&mut h);
    // The header-cap "Freeze all" button's enabled state (todo #581) folds
    // in freeze statuses beyond the per-track `is_frozen` bit below (e.g.
    // `Freezing` vs `Idle`), so hash the derived bool directly.
    freeze_all_enabled(r).hash(&mut h);
    for t in &r.registry.tracks {
        if t.sub_track.is_some() {
            continue;
        }
        t.id.hash(&mut h);
        t.name.hash(&mut h);
        t.muted.hash(&mut h);
        t.soloed.hash(&mut h);
        t.record_armed.hash(&mut h);
        t.monitor_enabled.hash(&mut h);
        t.track_type.hash(&mut h);
        t.instrument_icon.hash(&mut h);
        t.instrument_type.hash(&mut h);
        t.input_device_name.hash(&mut h);
        // Freeze state drives the frost treatment (frosted header, FROZEN
        // pill, locked chip, dimmed arm/monitor), so it must enter the
        // lazy-cache key or a freeze/unfreeze wouldn't repaint the column.
        r.freeze.status(t.id).is_frozen().hash(&mut h);
        if let Some(p) = t.plugins.first() {
            p.plugin_name.hash(&mut h);
        }
    }
    h.finish()
}

fn build_track_headers(r: &Resonance) -> Element<'static, Message> {
    // ---- Chrome heights ----
    // The "chrome" is everything above the track-lane area: the ruler,
    // the section band (when any sections are placed), the always-visible
    // global-shelf header strip, and — when expanded — the three lane
    // labels (chords / tempo / signature). These mirror the timeline
    // canvas's `fixed_header_height` so the lane origin lines up
    // row-for-row with the canvas's first track lane.
    let has_section_band = !r.compose.placements.is_empty();
    let expanded = r.viewport.global_tracks_expanded;
    let section_band_h = if has_section_band {
        theme::SECTION_BAND_HEIGHT
    } else {
        0.0
    };
    let lane_labels_h = if expanded {
        theme::GLOBAL_TRACK_CHORD_HEIGHT
            + theme::GLOBAL_TRACK_TEMPO_HEIGHT
            + theme::GLOBAL_TRACK_SIG_HEIGHT
    } else {
        0.0
    };
    let chrome_h = theme::RULER_HEIGHT
        + section_band_h
        + theme::GLOBAL_SHELF_HEADER_HEIGHT
        + lane_labels_h;

    // ---- Chrome subtree ----
    // The chrome lives on the TOP layer of a `stack` so its opaque
    // backgrounds mask any track-lane content that bleeds upward through
    // negative-padding scroll offsets (iced 0.14 `container.clip(true)`
    // only narrows the viewport passed to children — it does not clip
    // child `fill_quad` backgrounds, so without this stack layering the
    // lane area's first row paints over the ruler / section band /
    // global shelf at sub-row scroll positions).
    //
    // A trailing `Space::new().height(Length::Fill)` sits below the
    // chrome rows. `Space` has no `mouse_area` / `on_press`, so events
    // dropped onto it fall through the stack down to the lane subtree.
    let mut chrome = column![].spacing(0);
    chrome = chrome.push(
        container(Space::new().width(Length::Fill))
            .width(Length::Fill)
            .height(theme::RULER_HEIGHT)
            .style(theme::base_bg),
    );
    if has_section_band {
        chrome = chrome.push(
            container(Space::new().width(Length::Fill))
                .width(Length::Fill)
                .height(theme::SECTION_BAND_HEIGHT)
                .style(theme::base_bg),
        );
    }
    chrome = chrome.push(shelf::build_global_shelf_header(
        expanded,
        freeze_all_enabled(r),
    ));
    if expanded {
        chrome = chrome.push(shelf::view_chord_lane_header(r));
        chrome = chrome.push(shelf::view_tempo_lane_header(r));
        chrome = chrome.push(shelf::view_signature_lane_header(r));
    }
    chrome = chrome.push(Space::new().height(Length::Fill));

    // ---- Lane subtree ----
    // The lane subtree starts with an opaque `chrome_h`-tall spacer (so
    // the lane area's Y origin lines up under the chrome layer) and
    // then renders the same negative-padding fractional-scroll lane
    // area as before. The chrome on the top layer masks anything that
    // overflows above the lane area's natural top edge.
    //
    // The arrange view is no longer a uniform stack of 96 px lanes:
    // group/folder tracks interleave 60 px group-header rows, and a
    // collapsed group hides its members. The shared `ArrangeRowLayout`
    // (todo #729) is the single source of truth for that heterogeneous
    // layout — both this column and the canvas consume it instead of
    // re-deriving row Y from a fixed pitch. We iterate its rows here so
    // the column stays row-for-row aligned with the canvas.
    let sorted_tracks: Vec<&TrackState> = r
        .sorted_tracks()
        .iter()
        .filter(|t| t.sub_track.is_none())
        .collect();
    let layout = ArrangeRowLayout::build(&sorted_tracks, &r.track_groups);

    let scroll_y = r.viewport.scroll_offset_y.max(0.0);

    // Variable-height virtualization via cumulative-Y range selection.
    // The lane area's drawn height is the full canvas viewport height
    // minus the chrome (ruler + section band + global shelf + lane
    // labels). We keep only rows whose vertical span intersects
    // `[scroll_y, scroll_y + viewport_lane_h)`:
    //
    //   - drop rows fully above (`y_bottom <= scroll_y`),
    //   - stop at the first row fully below (`y_top >= lane_bottom`).
    //
    // Because rows carry their own `y_top`/`height`, a partial bottom
    // row is naturally included (its `y_top` is still `< lane_bottom`),
    // so no separate overscan row is needed. When the canvas hasn't
    // reported a viewport size yet (`viewport_height == 0`), show every
    // row from the first visible one onward so the initial paint isn't
    // blank.
    //
    // The chrome subtree paints opaque backgrounds on its own
    // (non-empty) Z-layer above the lane subtree — virtualizing the
    // lane column doesn't affect that masking invariant. See the
    // module doc-comment and `track_header_no_bleed_into_chrome_*`.
    let viewport_lane_h = (r.viewport.viewport_height - chrome_h).max(0.0);
    let lane_bottom = if r.viewport.viewport_height > 0.0 {
        scroll_y + viewport_lane_h
    } else {
        f32::INFINITY
    };

    // Cumulative-Y window of visible rows. The partial-top fractional
    // offset is measured against the first visible row's `y_top` (which
    // may be a 60 px group header or a 96 px lane), so smooth sub-row
    // scrolling works across the mixed row pitches.
    let visible_rows: Vec<&crate::view::arrange_layout::ArrangeRow> = layout
        .rows()
        .iter()
        .skip_while(|row| row.y_bottom() <= scroll_y)
        .take_while(|row| row.y_top < lane_bottom)
        .collect();
    let frac_offset = visible_rows
        .first()
        .map(|row| scroll_y - row.y_top)
        .unwrap_or(0.0);

    let by_id: std::collections::HashMap<_, _> =
        sorted_tracks.iter().map(|t| (t.id, *t)).collect();
    let selected_tracks = &r.interaction.selected_tracks;
    let mut lane_col = column![].spacing(0);
    for row in visible_rows {
        match row.kind {
            ArrangeRowKind::GroupHeader(group_id) => {
                if let Some(group) = r.track_groups.get_group(group_id) {
                    // `member_count` mirrors the mixer group strip: the
                    // flattened (recursive) member count from the registry.
                    let member_count = r.track_groups.get_all_member_ids(group_id).len();
                    lane_col =
                        lane_col.push(group_header::view_group_header(group, member_count));
                }
            }
            ArrangeRowKind::Track(track_id) => {
                if let Some(track) = by_id.get(&track_id) {
                    let is_selected = selected_tracks.contains(&track_id);
                    // Context-menu anchor for this row (todo #581), in
                    // arrange-area space: a widget `mouse_area` press has no
                    // cursor position, so the menu anchors a fixed inset into
                    // the row instead. The y is clamped so a menu opened near
                    // the bottom edge stays fully on screen.
                    let menu_x = if r.browser.visible {
                        theme::BROWSER_WIDTH
                    } else {
                        0.0
                    } + 56.0;
                    let mut menu_y = chrome_h + (row.y_top - scroll_y) + 30.0;
                    if r.viewport.viewport_height > 0.0 {
                        menu_y = menu_y
                            .min(r.viewport.viewport_height - TRACK_MENU_HEIGHT)
                            .max(chrome_h);
                    }
                    lane_col = lane_col.push(track::view_track_header(
                        r,
                        track,
                        is_selected,
                        (menu_x, menu_y),
                    ));
                }
            }
        }
    }

    let lane_area = container(lane_col)
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .padding(Padding {
            top: -frac_offset,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        });

    let lane_subtree = column![
        Space::new().height(chrome_h).width(Length::Fill),
        lane_area,
    ]
    .spacing(0);

    // Stack chrome above the lane subtree. `stack`'s base layer
    // determines the intrinsic size — set both layers to fill the outer
    // container so they share the same bounds.
    let layered = stack![lane_subtree, chrome]
        .width(Length::Fill)
        .height(Length::Fill);

    container(layered)
        .width(theme::TRACK_HEADER_WIDTH)
        .height(Length::Fill)
        .clip(true)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

