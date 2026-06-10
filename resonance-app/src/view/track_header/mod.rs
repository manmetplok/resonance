//! Track header column shown on the left of the Arrange view. Matches
//! the redesign: 28×28 instrument glyph + name/kind stacked column +
//! a slim 4-button row (mute / solo / arm / monitor). Selection paints a
//! 2px lavender stripe on the left edge.
//!
//! Mono toggle, FX bypass, and bounce-in-place stay on the mixer strip
//! where channel-strip controls live; the Arrange header is for arranging.
//!
//! The header column mirrors the timeline canvas's vertical layout
//! row-for-row so each header stays glued to its lane during vertical
//! scrolling. Internally it's a two-layer `stack`:
//!
//! - **Lane subtree** (base layer): a `chrome_h`-tall transparent
//!   spacer followed by the lane area. The lane area applies
//!   `scroll_offset_y` as a negative top padding so the partial top
//!   row scrolls smoothly instead of snapping to row boundaries.
//!   Manual virtualization drops tracks above `scroll_offset_y` AND
//!   below `scroll_offset_y + viewport_lane_h` from the widget tree,
//!   so a 200-track session only allocates a handful of
//!   `view_track_header` subtrees.
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
use iced::widget::{button, column, container, mouse_area, pick_list, row, stack, text, Space};
use iced::{alignment, Color, Element, Length, Padding};

use crate::message::*;
use crate::state::{self, TrackState};
use crate::theme::{self, fa};
use crate::view::controls::{
    delete_button, monitor_button, mute_button, record_arm_button, solo_button,
};
use crate::Resonance;

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
    // Manual track-list virtualization (`build_track_headers`) drops
    // tracks below `scroll_offset_y + viewport_height` from the widget
    // tree; the cache must invalidate on window resize so previously-
    // off-screen rows materialize when the viewport grows.
    r.viewport.viewport_height.to_bits().hash(&mut h);
    // Whether the section band exists changes the fixed-header height
    // of the column, so it must invalidate the lazy cache.
    (!r.compose.placements.is_empty()).hash(&mut h);
    r.interaction.selected_track.hash(&mut h);
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
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::BG_1)),
                ..Default::default()
            }),
    );
    if has_section_band {
        chrome = chrome.push(
            container(Space::new().width(Length::Fill))
                .width(Length::Fill)
                .height(theme::SECTION_BAND_HEIGHT)
                .style(|_theme| container::Style {
                    background: Some(iced::Background::Color(theme::BG_1)),
                    ..Default::default()
                }),
        );
    }
    chrome = chrome.push(build_global_shelf_header(expanded));
    if expanded {
        chrome = chrome.push(view_chord_lane_header(r));
        chrome = chrome.push(view_tempo_lane_header(r));
        chrome = chrome.push(view_signature_lane_header(r));
    }
    chrome = chrome.push(Space::new().height(Length::Fill));

    // ---- Lane subtree ----
    // The lane subtree starts with an opaque `chrome_h`-tall spacer (so
    // the lane area's Y origin lines up under the chrome layer) and
    // then renders the same negative-padding fractional-scroll lane
    // area as before. The chrome on the top layer masks anything that
    // overflows above the lane area's natural top edge.
    let sorted_tracks: Vec<&TrackState> = r
        .sorted_tracks()
        .iter()
        .filter(|t| t.sub_track.is_none())
        .collect();

    let scroll_y = r.viewport.scroll_offset_y.max(0.0);
    let first_visible = (scroll_y / theme::TRACK_HEIGHT).floor() as usize;
    let frac_offset = scroll_y - first_visible as f32 * theme::TRACK_HEIGHT;

    // Bottom-side virtualization: drop tracks that sit entirely below
    // the on-screen viewport. The lane area's drawn height is the full
    // canvas viewport height minus the chrome (ruler + section band +
    // global shelf + lane labels), so we cap `last_visible` by the
    // number of rows that fit inside `viewport_lane_h` plus one
    // overscan row to cover the partial bottom edge on sub-pixel
    // scroll. When the canvas hasn't reported a viewport size yet
    // (`viewport_height == 0`), fall back to "show everything from
    // `first_visible` onward" so the initial paint isn't blank.
    //
    // The chrome subtree paints opaque backgrounds on its own
    // (non-empty) Z-layer above the lane subtree — virtualizing the
    // lane column doesn't affect that masking invariant. See the
    // module doc-comment and `track_header_no_bleed_into_chrome_*`.
    let viewport_lane_h = (r.viewport.viewport_height - chrome_h).max(0.0);
    let last_visible = if r.viewport.viewport_height > 0.0 {
        let rows_visible = (viewport_lane_h / theme::TRACK_HEIGHT).ceil() as usize + 1;
        first_visible.saturating_add(rows_visible)
    } else {
        sorted_tracks.len()
    };

    let mut lane_col = column![].spacing(0);
    let selected_track = r.interaction.selected_track;
    for (i, track) in sorted_tracks.iter().enumerate() {
        if i < first_visible {
            continue;
        }
        if i >= last_visible {
            break;
        }
        let is_selected = selected_track == Some(track.id);
        lane_col = lane_col.push(view_track_header(r, track, is_selected));
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

fn view_track_header(
    _r: &Resonance,
    track: &TrackState,
    is_selected: bool,
) -> Element<'static, Message> {
    let track_id = track.id;

    // ---- Glyph (28×28 rounded BG_2 square with the track's instrument icon) ----
    let glyph_char = glyph_for_track(track);
    let glyph = container(
        theme::icon(glyph_char)
            .size(13)
            .color(theme::TEXT_2),
    )
    .width(28)
    .height(28)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    });

    // ---- Name (top) + kind (bottom) ----
    let name = text(track.name.clone())
        .size(13)
        .font(theme::UI_FONT_MEDIUM)
        .color(theme::TEXT_1)
        .wrapping(iced::widget::text::Wrapping::None);
    let kind_str = kind_label_for_track(track);
    let kind = text(kind_str)
        .size(10)
        .color(theme::TEXT_3)
        .wrapping(iced::widget::text::Wrapping::None);

    let name_col = column![
        container(name).width(Length::Fill).clip(true),
        container(kind).width(Length::Fill).clip(true),
    ]
    .spacing(2);

    // ---- 4 mini buttons: Mute / Solo / Arm / Monitor ----
    let buttons = row![
        mute_button(
            track.muted,
            Message::Track(TrackMessage::ToggleMute(track.id)),
            12,
        ),
        solo_button(
            track.soloed,
            Message::Track(TrackMessage::ToggleSolo(track.id)),
            12,
        ),
        record_arm_button(track.record_armed, track.id, 12),
        monitor_button(track.monitor_enabled, track.id, 12),
    ]
    .spacing(2)
    .align_y(alignment::Vertical::Center);

    // ---- Top-right delete (tiny, hugs the corner) ----
    let del = delete_button(
        Message::Track(TrackMessage::RequestRemoveTrack(track.id)),
        11,
    );

    // Top of the cell: name + kind, with delete in the corner.
    let top_row = row![
        glyph,
        Space::new().width(10),
        name_col,
        Space::new().width(6),
        del,
    ]
    .spacing(0)
    .align_y(alignment::Vertical::Center);

    // Bottom of the cell: 4-button row, right-aligned to keep the glyph +
    // name visually the dominant element.
    let button_row = row![Space::new().width(Length::Fill), buttons]
        .align_y(alignment::Vertical::Center);

    let body_col = column![top_row, Space::new().height(8), button_row,]
        .spacing(0)
        .height(Length::Fill);

    // ---- Background, left selection stripe, hairline bottom ----
    let bg = if track.record_armed {
        theme::PANEL_ARMED
    } else if is_selected {
        theme::BG_2
    } else {
        theme::BG_1
    };
    let stripe_color = if track.record_armed {
        theme::BAD
    } else if is_selected {
        theme::ACCENT
    } else {
        Color::TRANSPARENT
    };

    let body = container(body_col)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: 10.0,
            right: 10.0,
            bottom: 10.0,
            left: 12.0,
        });

    let body_with_bg = container(body)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(bg)),
            ..Default::default()
        });

    let stripe = container(Space::new().height(Length::Fill))
        .width(2)
        .height(Length::Fill)
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(stripe_color)),
            ..Default::default()
        });

    // The cell is `TRACK_HEIGHT - 1` so that `cell + hairline` together
    // sum to exactly `TRACK_HEIGHT` — matching the canvas's per-row
    // pitch. Without this trim, every column row was 1 px taller than
    // the canvas row and headers drifted down 1 px per track.
    let cell = row![stripe, body_with_bg].height(theme::TRACK_HEIGHT - 1.0);

    // 1px hairline below each cell so rows separate without a heavy border.
    let hairline = container(Space::new().width(Length::Fill))
        .height(1)
        .style(theme::separator_bg);

    let stack = column![cell, hairline].spacing(0);

    mouse_area(stack)
        .on_press(Message::Ui(UiMessage::SelectTrack(Some(track_id))))
        .into()
}

/// Pick a Font Awesome glyph for the given track. Uses the persisted
/// `instrument_icon` for instrument tracks; audio tracks get a microphone.
fn glyph_for_track(track: &TrackState) -> char {
    match track.track_type {
        resonance_audio::types::TrackType::Audio => fa::MICROPHONE,
        resonance_audio::types::TrackType::Instrument => track.instrument_icon.glyph(),
        resonance_audio::types::TrackType::Vocal => fa::MICROPHONE,
    }
}

/// Build the small descriptor line under the track name. Examples:
/// - Audio track: "Audio · MIC 1" (or just "Audio" when no input is set)
/// - Instrument track with plugin: "Resonance Wave"
/// - Drum track: "Kit · Resonance Drums"
/// - Track with no plugin yet: "Instrument" or "Audio"
fn kind_label_for_track(track: &TrackState) -> String {
    use resonance_audio::types::TrackType;
    let plugin_name = track
        .plugins
        .first()
        .map(|p| p.plugin_name.clone())
        .unwrap_or_default();
    // The track-list column is ~140px wide once the glyph + button row
    // are accounted for, so the kind line cannot exceed ~22 chars at 10px
    // before it wraps. `short()` enforces an ellipsis well within that.
    match track.track_type {
        TrackType::Audio => match track.input_device_name.as_deref() {
            Some(dev) if !dev.is_empty() => format!("Audio · {}", short(dev, 14)),
            _ => "Audio".to_string(),
        },
        TrackType::Instrument => {
            if plugin_name.is_empty() {
                if track.instrument_type == state::InstrumentType::Drum {
                    "Drum kit".to_string()
                } else {
                    "Instrument".to_string()
                }
            } else if track.instrument_type == state::InstrumentType::Drum {
                format!("Kit · {}", short(&plugin_name, 14))
            } else {
                short(&plugin_name, 22)
            }
        }
        TrackType::Vocal => "Vocal".to_string(),
    }
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// Numerator wrapper for the pick_list (needs Display + PartialEq).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Numerator(u8);
impl std::fmt::Display for Numerator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Denominator(u8);
impl std::fmt::Display for Denominator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Time-signature numerator options (1..=16) cached as a static slice.
/// `pick_list` takes its options by value (a `Borrow<[T]>`), so building
/// `(1..=16).map(Numerator).collect()` every frame allocates a fresh
/// `Vec<Numerator>` per repaint while the signature row is visible.
fn numerator_options() -> &'static [Numerator] {
    static V: std::sync::OnceLock<Vec<Numerator>> = std::sync::OnceLock::new();
    V.get_or_init(|| (1..=16).map(Numerator).collect())
}

/// Time-signature denominator options (powers of two from 2 to 16),
/// cached as a static slice. See `numerator_options` for the rationale.
fn denominator_options() -> &'static [Denominator] {
    static V: std::sync::OnceLock<Vec<Denominator>> = std::sync::OnceLock::new();
    V.get_or_init(|| [2, 4, 8, 16].into_iter().map(Denominator).collect())
}

/// Build the always-visible 32 px global-shelf header strip on the
/// column side. Contains the caret toggle, `GLOBAL` tag, and a small
/// count badge ("3" = chords + tempo + sig). Clicking anywhere on the
/// strip toggles the shelf open / closed.
fn build_global_shelf_header(expanded: bool) -> Element<'static, Message> {
    let caret = if expanded {
        fa::CARET_DOWN
    } else {
        fa::CARET_RIGHT
    };
    let caret_el = container(theme::icon(caret).size(9).color(theme::TEXT_3))
        .width(12)
        .height(12)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    let global_tag = text("GLOBAL")
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_2);

    // Small `3` count pill — mirrors the design's `gsTagCount`.
    let count_pill = container(
        text("3")
            .size(9)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3),
    )
    .padding(iced::Padding {
        top: 1.0,
        right: 5.0,
        bottom: 1.0,
        left: 5.0,
    })
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: 3.0.into(),
        },
        ..Default::default()
    });

    // Right-side: small `+` button — adds a new instrument/audio track.
    // Lives here (in the column-side shelf header) so it's reachable
    // regardless of whether the shelf is expanded. Replaces the previous
    // standalone "TRACKS · +" row that used to sit between the ruler and
    // the lane area before this redesign.
    let add_btn = button(text("+").size(13).color(theme::TEXT_3))
        .on_press(Message::Ui(UiMessage::OpenAddTrackMenu))
        .style(|_theme, status| theme::ghost_button_style(status))
        .padding(iced::Padding {
            top: 0.0,
            right: 6.0,
            bottom: 2.0,
            left: 6.0,
        })
        .width(22)
        .height(22);

    let inner = row![
        Space::new().width(10),
        caret_el,
        Space::new().width(6),
        global_tag,
        Space::new().width(6),
        count_pill,
        Space::new().width(Length::Fill),
        add_btn,
        Space::new().width(8),
    ]
    .align_y(alignment::Vertical::Center)
    .height(theme::GLOBAL_SHELF_HEADER_HEIGHT);

    let strip = container(inner)
        .width(Length::Fill)
        .height(theme::GLOBAL_SHELF_HEADER_HEIGHT)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        });

    mouse_area(strip)
        .on_press(Message::Ui(UiMessage::ToggleGlobalTracks))
        .into()
}

/// Common chrome for a single global-shelf lane label. Renders a
/// 22 px rounded-square glyph + name (12 px Medium, TEXT_1) + sub-line
/// (10 px Mono, TEXT_3), with optional warm tint on the glyph for the
/// tempo lane (matching the canvas-side automation curve color).
fn build_global_lane_label(
    glyph: char,
    name: &'static str,
    sub: String,
    height: f32,
    warm: bool,
) -> Element<'static, Message> {
    let glyph_color = if warm { theme::WARM } else { theme::TEXT_2 };
    let glyph_box = container(theme::icon(glyph).size(11).color(glyph_color))
        .width(theme::GLOBAL_TRACK_GLYPH_SIZE)
        .height(theme::GLOBAL_TRACK_GLYPH_SIZE)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 5.0.into(),
            },
            ..Default::default()
        });

    let name_el = text(name)
        .size(12)
        .font(theme::UI_FONT_MEDIUM)
        .color(theme::TEXT_1);
    let sub_el = text(sub).size(10).font(theme::MONO_FONT).color(theme::TEXT_3);
    let name_col = column![name_el, sub_el].spacing(1);

    // Mini M / Lock control cluster — placeholders for parity with the
    // design. Wired to no-ops via a `small_button_style` ghost.
    let m_btn = button(
        text("M")
            .size(9)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_4),
    )
    .style(|_theme, status| theme::small_button_style(status))
    .padding([0, 3])
    .width(16)
    .height(16);
    let lock_btn = button(
        text("L")
            .size(9)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_4),
    )
    .style(|_theme, status| theme::small_button_style(status))
    .padding([0, 3])
    .width(16)
    .height(16);
    let controls = row![m_btn, Space::new().width(2), lock_btn].spacing(0);

    let inner = row![
        Space::new().width(14),
        glyph_box,
        Space::new().width(9),
        name_col,
        Space::new().width(Length::Fill),
        controls,
        Space::new().width(8),
    ]
    .align_y(alignment::Vertical::Center)
    .height(height);

    container(inner)
        .width(Length::Fill)
        .height(height)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::GLOBAL_TRACK_BG)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Chord lane label — name "Chords", sub "from sections · N chords".
fn view_chord_lane_header(r: &Resonance) -> Element<'static, Message> {
    let total: usize = r
        .compose
        .definitions
        .iter()
        .map(|d| d.chords.len())
        .sum();
    let sub = if total == 0 {
        "from sections".to_string()
    } else if total == 1 {
        "from sections · 1 chord".to_string()
    } else {
        format!("from sections · {} chords", total)
    };
    build_global_lane_label(
        fa::MUSIC,
        "Chords",
        sub,
        theme::GLOBAL_TRACK_CHORD_HEIGHT,
        false,
    )
}

/// Tempo lane label — name "Tempo", sub "{BPM} BPM · automated" when
/// >1 tempo event, else "{BPM} BPM".
fn view_tempo_lane_header(r: &Resonance) -> Element<'static, Message> {
    let bpm = r.transport.bpm;
    let sub = if r.tempo_events.len() > 1 {
        format!("{:.1} BPM · automated", bpm)
    } else {
        format!("{:.1} BPM", bpm)
    };
    build_global_lane_label(
        fa::WAVE_SQUARE,
        "Tempo",
        sub,
        theme::GLOBAL_TRACK_TEMPO_HEIGHT,
        true,
    )
}

/// Signature lane label — name "Signature", sub "{n}/{d}" (or
/// "Mixed" when multiple distinct signatures exist in the project).
/// When a signature event is selected, surfaces inline pick_lists so
/// the user can edit numerator and denominator without leaving the
/// shelf — keeps the pre-redesign editing affordance intact.
fn view_signature_lane_header(r: &Resonance) -> Element<'static, Message> {
    let row_h = theme::GLOBAL_TRACK_SIG_HEIGHT;

    let selected = r.interaction.selected_global_event.and_then(|sel| {
        if sel.kind == state::GlobalTrackKind::Signature {
            r.signature_events.get(sel.index).map(|ev| (sel.index, ev))
        } else {
            None
        }
    });

    // Header: glyph + "Signature" + sub-line OR inline pickers.
    let glyph_box = container(
        theme::icon(fa::SLIDERS)
            .size(11)
            .color(theme::TEXT_2),
    )
    .width(theme::GLOBAL_TRACK_GLYPH_SIZE)
    .height(theme::GLOBAL_TRACK_GLYPH_SIZE)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 0.0,
            radius: 5.0.into(),
        },
        ..Default::default()
    });

    let name_el = text("Signature")
        .size(12)
        .font(theme::UI_FONT_MEDIUM)
        .color(theme::TEXT_1);

    let inner: Element<'static, Message> = if let Some((idx, event)) = selected {
        let num = event.numerator;
        let den = event.denominator;
        let num_picker = pick_list(numerator_options(), Some(Numerator(num)), move |n: Numerator| {
            Message::GlobalTrack(GlobalTrackMessage::UpdateSignatureEvent {
                index: idx,
                numerator: n.0,
                denominator: den,
            })
        })
        .text_size(10)
        .width(38);
        let den_picker =
            pick_list(denominator_options(), Some(Denominator(den)), move |d: Denominator| {
                Message::GlobalTrack(GlobalTrackMessage::UpdateSignatureEvent {
                    index: idx,
                    numerator: num,
                    denominator: d.0,
                })
            })
            .text_size(10)
            .width(38);
        let slash = text("/").size(11).color(theme::TEXT_3);
        row![
            Space::new().width(14),
            glyph_box,
            Space::new().width(9),
            name_el,
            Space::new().width(8),
            num_picker,
            slash,
            den_picker,
            Space::new().width(Length::Fill),
            Space::new().width(8),
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center)
        .height(row_h)
        .into()
    } else {
        let sub_text = format!(
            "{}/{}",
            r.transport.time_sig_num, r.transport.time_sig_den
        );
        let sub_el = text(sub_text)
            .size(10)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3);
        let name_col = column![name_el, sub_el].spacing(1);
        row![
            Space::new().width(14),
            glyph_box,
            Space::new().width(9),
            name_col,
            Space::new().width(Length::Fill),
            Space::new().width(8),
        ]
        .align_y(alignment::Vertical::Center)
        .height(row_h)
        .into()
    };

    container(inner)
        .width(Length::Fill)
        .height(row_h)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::GLOBAL_TRACK_BG)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}
