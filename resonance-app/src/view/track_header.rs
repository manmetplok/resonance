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
//! scrolling:
//!
//! - ruler row (`RULER_HEIGHT`)
//! - section-band placeholder (`SECTION_BAND_HEIGHT` when sections exist)
//! - global tracks area (`2 * GLOBAL_TRACK_ROW_HEIGHT` when expanded)
//! - lane area: clipped, with `scroll_offset_y` applied as a negative
//!   top inset so the partial top row scrolls smoothly instead of
//!   snapping to row boundaries.
use iced::widget::{button, column, container, mouse_area, pick_list, row, text, Space};
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
    // Whether the section band exists changes the fixed-header height
    // of the column, so it must invalidate the lazy cache.
    (!r.compose.placements.is_empty()).hash(&mut h);
    r.interaction.selected_track.hash(&mut h);
    r.transport.time_sig_num.hash(&mut h);
    r.transport.time_sig_den.hash(&mut h);
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
    let mut headers = column![].spacing(0);

    // Header row matching the timeline ruler height — global tracks
    // toggle + add-track button. Designed as a tiny "Tracks" label row
    // with a dashed `+` button on the right (per design).
    let expanded = r.viewport.global_tracks_expanded;
    let caret = if expanded {
        fa::CARET_DOWN
    } else {
        fa::CARET_RIGHT
    };
    let toggle_btn = button(theme::icon(caret).size(10).color(theme::TEXT_3))
        .on_press(Message::Ui(UiMessage::ToggleGlobalTracks))
        .style(|_theme, status| theme::small_button_style(status))
        .padding([2, 4]);

    let add_btn = button(text("+").size(13).color(theme::TEXT_3))
        .on_press(Message::Ui(UiMessage::OpenAddTrackMenu))
        .style(|_theme, status| theme::ghost_button_style(status))
        .padding([0, 6])
        .width(22)
        .height(22);

    let header_row = row![
        Space::new().width(10),
        toggle_btn,
        Space::new().width(4),
        text("TRACKS")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::new().width(Length::Fill),
        add_btn,
        Space::new().width(8),
    ]
    .align_y(alignment::Vertical::Center)
    .height(theme::RULER_HEIGHT);
    headers = headers.push(
        container(header_row)
            .width(Length::Fill)
            .height(theme::RULER_HEIGHT)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(theme::BG_1)),
                border: iced::Border {
                    color: theme::LINE_2,
                    width: 0.0,
                    radius: 0.0.into(),
                },
                ..Default::default()
            }),
    );

    // Section-band placeholder — the timeline canvas reserves
    // `SECTION_BAND_HEIGHT` under the ruler when at least one compose
    // section is placed (see `TimelineCanvas::section_band_height`).
    // The header column has no section pills of its own, so we render
    // a blank strip of the same height to keep the lane area's Y
    // origin synced with the canvas. Without this, every track header
    // drifts up by 22 px from its lane whenever sections exist.
    if !r.compose.placements.is_empty() {
        headers = headers.push(
            container(Space::new().width(Length::Fill))
                .width(Length::Fill)
                .height(theme::SECTION_BAND_HEIGHT)
                .style(|_theme| container::Style {
                    background: Some(iced::Background::Color(theme::BG_1)),
                    ..Default::default()
                }),
        );
    }

    // Global tracks area (tempo + time signature) — visible when expanded
    if expanded {
        let row_h = theme::GLOBAL_TRACK_ROW_HEIGHT;
        let gt_style = |_theme: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(theme::GLOBAL_TRACK_BG)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        };

        let tempo_row = container(
            row![
                Space::new().width(10),
                text("Tempo").size(11).color(theme::TEXT_2),
            ]
            .align_y(alignment::Vertical::Center)
            .height(row_h),
        )
        .width(Length::Fill)
        .height(row_h)
        .style(gt_style);
        headers = headers.push(tempo_row);

        let sig_row = view_signature_header(r, row_h);
        headers = headers.push(
            container(sig_row)
                .width(Length::Fill)
                .height(row_h)
                .style(gt_style),
        );
    }

    let sorted_tracks: Vec<&TrackState> = r
        .sorted_tracks()
        .into_iter()
        .filter(|t| t.sub_track.is_none())
        .collect();

    // Build the lane area as its own clipped sub-container. The canvas
    // applies `scroll_offset_y` as a fractional pixel offset to track
    // lanes; the column has to do the same or rows snap to integer
    // multiples of `TRACK_HEIGHT` and drift on sub-row scroll.
    //
    // Strategy: skip tracks fully above the viewport (perf), then
    // shift the remaining column up by the sub-row remainder using a
    // negative top padding. The outer container's `clip(true)` hides
    // the partial row that bleeds above the lane origin.
    let scroll_y = r.viewport.scroll_offset_y.max(0.0);
    let first_visible = (scroll_y / theme::TRACK_HEIGHT).floor() as usize;
    let frac_offset = scroll_y - first_visible as f32 * theme::TRACK_HEIGHT;

    let mut lane_col = column![].spacing(0);
    let selected_track = r.interaction.selected_track;
    for (i, track) in sorted_tracks.iter().enumerate() {
        if i < first_visible {
            continue;
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
    headers = headers.push(lane_area);

    container(headers)
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

/// Render the Time Sig header row. When a signature event is selected,
/// show inline pick_lists for editing numerator and denominator.
fn view_signature_header(r: &Resonance, row_h: f32) -> Element<'static, Message> {
    let label = text("Time Sig").size(11).color(theme::TEXT_2);

    let selected = r.interaction.selected_global_event.and_then(|sel| {
        if sel.kind == state::GlobalTrackKind::Signature {
            r.signature_events.get(sel.index).map(|ev| (sel.index, ev))
        } else {
            None
        }
    });

    if let Some((idx, event)) = selected {
        let num = event.numerator;
        let den = event.denominator;

        let nums: Vec<Numerator> = (1..=16).map(Numerator).collect();
        let dens: Vec<Denominator> = [2, 4, 8, 16].iter().copied().map(Denominator).collect();

        let num_picker = pick_list(nums, Some(Numerator(num)), move |n: Numerator| {
            Message::GlobalTrack(GlobalTrackMessage::UpdateSignatureEvent {
                index: idx,
                numerator: n.0,
                denominator: den,
            })
        })
        .text_size(11)
        .width(42);

        let den_picker = pick_list(dens, Some(Denominator(den)), move |d: Denominator| {
            Message::GlobalTrack(GlobalTrackMessage::UpdateSignatureEvent {
                index: idx,
                numerator: num,
                denominator: d.0,
            })
        })
        .text_size(11)
        .width(42);

        let slash = text("/").size(12).color(theme::TEXT_3);

        row![
            Space::new().width(10),
            label,
            Space::new().width(8),
            num_picker,
            slash,
            den_picker,
        ]
        .spacing(2)
        .align_y(alignment::Vertical::Center)
        .height(row_h)
        .into()
    } else {
        row![Space::new().width(10), label,]
            .align_y(alignment::Vertical::Center)
            .height(row_h)
            .into()
    }
}
