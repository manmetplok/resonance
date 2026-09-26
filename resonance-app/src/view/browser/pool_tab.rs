//! Pool tab body and audition transport for the media browser
//! (todos #603 and #604).
//!
//! The Pool tab shows a scrollable list of the project's imported audio
//! assets, each annotated with a usage badge and — for missing files — an
//! inline `relink` chip.
//!
//! The audition transport is pinned to the bottom of the browser panel and
//! rendered regardless of which tab is active; it is co-located here because
//! the pool is its primary use-case and it references pool-asset thumbnails.

use std::cell::Cell;
use std::path::Path;

use iced::widget::canvas::{self, Geometry};
use iced::widget::text::LineHeight;
use iced::widget::{button, column, container, row, scrollable, text, Canvas, Space};
use iced::{alignment, mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::message::*;
use crate::state::pool::PoolAsset;
use crate::theme;
use crate::view::relink_dialog::relink_chip;
use crate::Resonance;

use super::style::{format_duration, format_label, WaveThumbnail, THUMB_H, THUMB_W};

// ---------------------------------------------------------------------------
// Pool tab body
// ---------------------------------------------------------------------------

/// The Pool tab body: a scrollable list of the project's imported audio
/// assets. Each row shows a type glyph, a mini waveform thumbnail, the
/// filename, a format chip, the duration, and a usage badge. Missing rows
/// carry an inline `relink` chip instead of (or alongside) the usage badge.
pub(super) fn pool_body<'a>(r: &'a Resonance) -> Element<'a, Message> {
    if r.pool.assets.is_empty() {
        return text("No assets imported yet. Drag audio onto the timeline or use Import audio\u{2026}")
            .size(12)
            .color(theme::TEXT_3)
            .into();
    }

    let sample_rate = r.sample_rate.max(1);
    let mut list = column![].spacing(4);
    for asset in &r.pool.assets {
        let usage = r.pool.usage_count(asset.id);
        list = list.push(pool_asset_row(asset, usage, sample_rate));
    }

    scrollable(list).height(Length::Fill).into()
}

/// One pool-asset row. Layout (left → right):
///
/// ```text
/// [♪] [~~waveform~~] [filename     ] [wav] [0:04] [used ×2]
///                                                  [relink ]  ← only if missing
/// ```
fn pool_asset_row<'a>(asset: &'a PoolAsset, usage: u32, sample_rate: u32) -> Element<'a, Message> {
    // --- Type glyph -------------------------------------------------------
    let (glyph_char, glyph_color) = if asset.missing {
        (theme::fa::TRIANGLE_EXCLAMATION, theme::BAD)
    } else {
        (theme::fa::MUSIC, theme::WARM)
    };
    let glyph = text(String::from(glyph_char))
        .font(theme::ICON_FONT)
        .size(11)
        .color(glyph_color)
        .line_height(LineHeight::Relative(1.0));

    // --- Mini waveform thumbnail ------------------------------------------
    let thumbnail = Canvas::new(WaveThumbnail {
        peaks: std::borrow::Cow::Borrowed(&asset.thumbnail_peaks),
        muted: asset.missing,
    })
    .width(Length::Fixed(THUMB_W))
    .height(Length::Fixed(THUMB_H));

    // --- Filename ---------------------------------------------------------
    let fname = asset_filename(asset).into_owned();
    let name_label = text(fname)
        .size(11)
        .font(theme::UI_FONT_MEDIUM)
        .color(if asset.missing { theme::TEXT_3 } else { theme::TEXT_1 })
        .line_height(LineHeight::Relative(1.0));

    // --- Format chip ------------------------------------------------------
    let fmt_str = format_label(asset.format);
    let fmt_chip = container(
        text(fmt_str)
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(if asset.missing { theme::TEXT_3 } else { theme::WARM })
            .line_height(LineHeight::Relative(1.0)),
    )
    .padding([2, 5])
    .style(move |_theme| {
        let border_color = if asset.missing { theme::LINE } else { theme::WARM_LINE };
        container::Style {
            border: iced::Border {
                color: border_color,
                width: 1.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        }
    });

    // --- Duration ---------------------------------------------------------
    let dur_secs = asset.duration_frames as f64 / sample_rate as f64;
    let dur_label = format_duration(dur_secs);
    let duration = text(dur_label)
        .size(10)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0));

    // --- Name + chips row -------------------------------------------------
    let meta_row =
        row![name_label, Space::new().width(4), fmt_chip, Space::new().width(4), duration]
            .align_y(alignment::Vertical::Center)
            .spacing(0);

    // --- Usage badge / relink chip ----------------------------------------
    let badge: Element<'_, Message> = if asset.missing {
        relink_chip()
    } else if usage > 0 {
        text(format!("used \u{d7}{usage}"))
            .size(10)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::ACCENT_SOFT)
            .line_height(LineHeight::Relative(1.0))
            .into()
    } else {
        text("unused")
            .size(10)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_3)
            .line_height(LineHeight::Relative(1.0))
            .into()
    };

    // --- Compose the full row ---------------------------------------------
    let inner = column![meta_row, Space::new().height(3), badge].spacing(0);

    let content = row![
        glyph,
        Space::new().width(6),
        thumbnail,
        Space::new().width(8),
        inner,
    ]
    .align_y(alignment::Vertical::Center)
    .spacing(0);

    container(content)
        .width(Length::Fill)
        .padding([5, 6])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_3)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

/// The filename (final path component) of an asset's original source path,
/// falling back to the whole path string when there is no component.
fn asset_filename(asset: &PoolAsset) -> std::borrow::Cow<'_, str> {
    Path::new(&asset.original_path)
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or(std::borrow::Cow::Borrowed(&asset.original_path))
}

// ---------------------------------------------------------------------------
// Audition transport (todo #604)
// ---------------------------------------------------------------------------

/// Height of the scrub waveform strip in the audition transport.
const SCRUB_H: f32 = 34.0;

/// The audition target's waveform peaks and length, resolved from the
/// playing (preferred) or selected row. Borrows the peaks straight off the
/// browser scan / media pool so the transport never clones them per frame.
struct AuditionTarget<'a> {
    /// Downsampled `(min, max)` peaks for the scrub strip. Empty when the
    /// row's waveform could not be decoded — the strip then draws a bare
    /// baseline.
    peaks: &'a [(f32, f32)],
    /// Total source frames — the scrub / playhead denominator.
    total_frames: u64,
    /// Sample rate used to turn frames into the `M:SS` time readout.
    sample_rate: u32,
}

/// Resolve the row the transport should reflect: the currently-playing row
/// if any, else the selected-to-audition row. Looks the path up first in the
/// Files-tab scan (filesystem rows) and then in the media pool (imported
/// assets) so both audition sources drive the same transport. `None` when
/// nothing is selected or the path is not a known row.
fn audition_target(r: &Resonance) -> Option<AuditionTarget<'_>> {
    let audition = &r.browser.audition;
    let path = audition.playing.as_ref().or(audition.selected.as_ref())?;
    let path_str = path.to_string_lossy();

    if let Some(entry) = r.browser.scan.files.iter().find(|e| e.path == path_str) {
        return Some(AuditionTarget {
            peaks: r.browser.scan.thumbnail(&entry.path),
            total_frames: entry.info.frames,
            sample_rate: entry.info.sample_rate.max(1),
        });
    }

    if let Some(asset) = r
        .pool
        .assets
        .iter()
        .find(|a| a.original_path == path_str || a.project_relative_path == path_str)
    {
        return Some(AuditionTarget {
            peaks: &asset.thumbnail_peaks,
            // `duration_frames` is the project-rate imported WAV — the frames
            // the engine preview scrubs over — so read it back at the project
            // rate for the time readout.
            total_frames: asset.duration_frames,
            sample_rate: r.sample_rate.max(1),
        });
    }

    None
}

/// The audition transport pinned to the bottom of the browser panel: a
/// play / stop button, a scrubbable waveform with a live playhead, an
/// `M:SS / M:SS` time readout, and the Auto-play / Loop / Sync-to-tempo
/// toggles.
///
/// The playhead and the position half of the time readout are **live** — they
/// track `audition.position_frame`, which is refreshed from throttled
/// `AuditionPosition` events. The whole panel is rebuilt each frame (no
/// `lazy` wrapper), and the scrub Canvas re-rasterises whenever the position
/// changes (its cache fingerprint includes it), so the moving playhead is
/// never trapped behind a stale fingerprint — per the view-performance rules
/// (doc #175).
pub(super) fn audition_transport(r: &Resonance) -> Element<'_, Message> {
    let audition = &r.browser.audition;
    let target = audition_target(r);
    let playing = audition.playing.is_some();

    // Play / stop. Stop while a preview sounds; otherwise play the selected
    // row. Disabled (no handler, dim glyph) when nothing is selected.
    let (glyph, on_press): (char, Option<Message>) = if playing {
        (theme::fa::STOP, Some(Message::Browser(BrowserMessage::Stop)))
    } else if let Some(path) = audition.selected.clone() {
        (
            theme::fa::PLAY,
            Some(Message::Browser(BrowserMessage::Play(path))),
        )
    } else {
        (theme::fa::PLAY, None)
    };
    let enabled = on_press.is_some();
    let mut play_btn = button(
        text(String::from(glyph))
            .font(theme::ICON_FONT)
            .size(12)
            .color(if enabled { theme::TEXT_1 } else { theme::TEXT_4 })
            .line_height(LineHeight::Relative(1.0)),
    )
    .padding([8, 11])
    .style(|_theme, status| theme::transport_button_style(status));
    if let Some(msg) = on_press {
        play_btn = play_btn.on_press(msg);
    }

    // Scrub strip + live playhead.
    let (peaks, total_frames): (&[(f32, f32)], u64) = target
        .as_ref()
        .map(|t| (t.peaks, t.total_frames))
        .unwrap_or((&[], 0));
    let scrub = Canvas::new(AuditionScrub {
        peaks,
        position_frame: audition.position_frame,
        total_frames,
        playing,
    })
    .width(Length::Fill)
    .height(Length::Fixed(SCRUB_H));

    // Time readout `M:SS / M:SS` (position / total). The position half is live.
    let sample_rate = target.as_ref().map(|t| t.sample_rate).unwrap_or(1);
    let pos_secs = audition.position_frame as f64 / sample_rate as f64;
    let total_secs = total_frames as f64 / sample_rate as f64;
    let readout = text(format!(
        "{} / {}",
        format_duration(pos_secs),
        format_duration(total_secs)
    ))
    .size(10)
    .font(theme::MONO_FONT)
    .color(theme::TEXT_2)
    .line_height(LineHeight::Relative(1.0));

    let top = row![play_btn, Space::new().width(10), scrub, Space::new().width(10), readout]
        .align_y(alignment::Vertical::Center)
        .spacing(0);

    // Auto-play / Loop / Sync toggles.
    let toggles = row![
        toggle_chip("Auto-play", audition.auto_play, BrowserMessage::ToggleAutoPlay),
        toggle_chip("Loop", audition.loop_enabled, BrowserMessage::ToggleLoop),
        toggle_chip("Sync", audition.sync_to_tempo, BrowserMessage::ToggleSync),
    ]
    .spacing(6);

    // A top hairline separates the transport from the listing above it.
    let divider =
        container(Space::new().width(Length::Fill).height(1)).style(theme::separator_bg);

    column![
        divider,
        Space::new().height(12),
        top,
        Space::new().height(10),
        toggles,
    ]
    .spacing(0)
    .into()
}

/// A small WARM toggle chip for the audition transport (Auto-play / Loop /
/// Sync). Reads WARM-filled when `active`, neutral otherwise.
fn toggle_chip<'a>(label: &'a str, active: bool, msg: BrowserMessage) -> Element<'a, Message> {
    button(
        text(label)
            .size(10)
            .font(theme::UI_FONT_MEDIUM)
            .line_height(LineHeight::Relative(1.0)),
    )
    .on_press(Message::Browser(msg))
    .padding([5, 9])
    .style(move |_theme, status| theme::toggle_button_style(active, theme::WARM, true, status))
    .into()
}

// ---------------------------------------------------------------------------
// Audition scrub Canvas — live peak display with click-to-scrub
// ---------------------------------------------------------------------------

struct AuditionScrub<'a> {
    peaks: &'a [(f32, f32)],
    position_frame: u64,
    total_frames: u64,
    playing: bool,
}

#[derive(Default)]
struct ScrubState {
    cache: canvas::Cache,
    fingerprint: Cell<u64>,
}

impl AuditionScrub<'_> {
    /// Fraction `[0, 1]` of a source frame along the strip. `0` when the
    /// length is unknown so nothing is drawn off-canvas.
    fn fraction(&self, frame: u64) -> f32 {
        if self.total_frames == 0 {
            0.0
        } else {
            (frame as f32 / self.total_frames as f32).clamp(0.0, 1.0)
        }
    }

    /// Cheap order-sensitive hash of everything that affects the drawn pixels
    /// so the cache invalidates on a real change (including a moved playhead)
    /// but survives a pure hover / resize repaint.
    fn fingerprint(&self) -> u64 {
        let mut h: u64 = 1469598103934665603; // FNV-1a offset basis
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(1099511628211);
        };
        mix(self.peaks.len() as u64);
        mix(self.position_frame);
        mix(self.total_frames);
        mix(self.playing as u64);
        h
    }
}

impl canvas::Program<Message> for AuditionScrub<'_> {
    type State = ScrubState;

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        // Click anywhere on the strip seeks the preview to the matching frame.
        if self.total_frames == 0 {
            return None;
        }
        if let iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            if let Some(pos) = cursor.position_in(bounds) {
                let fraction = (pos.x / bounds.width).clamp(0.0, 1.0);
                let frame = (fraction * self.total_frames as f32) as u64;
                return Some(
                    canvas::Action::publish(Message::Browser(BrowserMessage::Scrub(frame)))
                        .and_capture(),
                );
            }
        }
        None
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let fp = self.fingerprint();
        if state.fingerprint.get() != fp {
            state.cache.clear();
            state.fingerprint.set(fp);
        }
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            let w = bounds.width;
            let h = bounds.height;
            let mid = h / 2.0;
            let head_x = self.fraction(self.position_frame) * w;

            // Backdrop + centre zero line.
            frame.fill_rectangle(Point::ORIGIN, Size::new(w, h), theme::BG_1);
            frame.fill_rectangle(Point::new(0.0, mid - 0.5), Size::new(w, 1.0), theme::LINE_2);

            // Peak columns. The portion left of the playhead reads WARM (the
            // played span); the rest stays a muted neutral.
            if !self.peaks.is_empty() {
                let col_w = w / self.peaks.len() as f32;
                let bar_w = col_w.max(1.0);
                let played = Color { a: 0.85, ..theme::WARM };
                let ahead = Color { a: 0.45, ..theme::TEXT_2 };
                for (i, &(min_v, max_v)) in self.peaks.iter().enumerate() {
                    let x = i as f32 * col_w;
                    let top = mid - max_v.clamp(-1.0, 1.0) * mid;
                    let bot = mid - min_v.clamp(-1.0, 1.0) * mid;
                    let bar_h = (bot - top).max(1.0);
                    let color = if self.total_frames > 0 && x + bar_w * 0.5 <= head_x {
                        played
                    } else {
                        ahead
                    };
                    frame.fill_rectangle(Point::new(x, top), Size::new(bar_w, bar_h), color);
                }
            }

            // Live playhead — a WARM hairline. Only drawn once a length is
            // known (otherwise it pins to the left edge and reads as noise).
            if self.total_frames > 0 {
                frame.fill_rectangle(
                    Point::new(head_x - 0.75, 0.0),
                    Size::new(1.5, h),
                    theme::WARM,
                );
            }
        });
        vec![geometry]
    }
}
