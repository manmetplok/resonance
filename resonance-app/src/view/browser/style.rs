//! Shared constants, canvas helpers, format utilities, and button-style
//! functions used by both the Files and Pool tab submodules.

use std::path::Path;

use iced::widget::canvas::{self, Frame, Geometry};
use iced::widget::text::LineHeight;
use iced::widget::{button, container, text};
use iced::{mouse, Color, Element, Point, Rectangle, Renderer, Size, Theme};

use crate::message::Message;
use crate::theme;

// ---------------------------------------------------------------------------
// Thumbnail canvas
// ---------------------------------------------------------------------------

/// Fixed width of the mini waveform thumbnail in pool / files rows.
pub(crate) const THUMB_W: f32 = 48.0;
/// Fixed height of the mini waveform thumbnail in pool / files rows.
pub(crate) const THUMB_H: f32 = 26.0;

/// A simple stateless [`canvas::Program`] that draws a waveform bar-chart
/// from `(min, max)` peak pairs. Shared by the Pool asset thumbnail and the
/// Files-tab audio rows; not a live visual so no cache is needed. When
/// `muted` (a missing pool asset) it draws in a faint BAD tint; otherwise
/// in WARM amber. An empty `peaks` slice draws just the centre baseline —
/// the idle state for a file whose waveform could not be decoded.
pub(crate) struct WaveThumbnail<'a> {
    pub(crate) peaks: &'a [(f32, f32)],
    pub(crate) muted: bool,
}

impl canvas::Program<Message> for WaveThumbnail<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let w = bounds.width;
        let h = bounds.height;
        let mid = h / 2.0;

        // Backdrop
        frame.fill_rectangle(Point::ORIGIN, Size::new(w, h), theme::BG_3);
        // Centre zero line
        frame.fill_rectangle(Point::new(0.0, mid - 0.5), Size::new(w, 1.0), theme::LINE_2);

        if !self.peaks.is_empty() {
            let bar_color: Color = if self.muted {
                Color { a: 0.45, ..theme::BAD }
            } else {
                Color { a: 0.75, ..theme::WARM }
            };
            let col_w = w / self.peaks.len() as f32;
            let bar_w = col_w.max(1.0);
            for (i, &(min_v, max_v)) in self.peaks.iter().enumerate() {
                let x = i as f32 * col_w;
                let top = mid - max_v.clamp(-1.0, 1.0) * mid;
                let bot = mid - min_v.clamp(-1.0, 1.0) * mid;
                let bar_h = (bot - top).max(1.0);
                frame.fill_rectangle(Point::new(x, top), Size::new(bar_w, bar_h), bar_color);
            }
        }

        vec![frame.into_geometry()]
    }
}

// ---------------------------------------------------------------------------
// Format helpers
// ---------------------------------------------------------------------------

/// A lightly colour-coded format chip (outlined pill) for an audio row.
/// The audio domain reads WARM; each container/codec family gets a subtle
/// hue shift off that base so wav / flac / mp3 / ogg are distinguishable at
/// a glance without leaving the palette.
pub(crate) fn format_chip<'a>(fmt: resonance_common::AudioFormat) -> Element<'a, Message> {
    let tint = format_color(fmt);
    container(
        text(format_label(fmt))
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(tint)
            .line_height(LineHeight::Relative(1.0)),
    )
    .padding([2, 5])
    .style(move |_theme| container::Style {
        border: iced::Border {
            color: Color { a: 0.34, ..tint },
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// The subtle per-format tint for the format chip. All stay muted and
/// close to WARM so the audio domain still reads amber; the small hue
/// shifts just help wav / flac / mp3 / ogg tell each other apart.
pub(crate) fn format_color(fmt: resonance_common::AudioFormat) -> Color {
    use resonance_common::AudioFormat;
    match fmt {
        // WARM amber — the uncompressed baseline.
        AudioFormat::Wav => theme::WARM,
        // Soft sage — lossless-compressed.
        AudioFormat::Flac => Color::from_rgb(0.60, 0.82, 0.69),
        // Soft lavender — lossy MPEG.
        AudioFormat::Mp3 => theme::ACCENT_SOFT,
        // Soft coral — lossy Vorbis.
        AudioFormat::Ogg => Color::from_rgb(0.90, 0.66, 0.56),
        // Everything else falls back to muted neutral.
        AudioFormat::Aac | AudioFormat::Mp4 | AudioFormat::Other => theme::TEXT_2,
    }
}

/// Short label for the container/codec family shown in the format chip.
pub(crate) fn format_label(fmt: resonance_common::AudioFormat) -> &'static str {
    use resonance_common::AudioFormat;
    match fmt {
        AudioFormat::Wav => "wav",
        AudioFormat::Flac => "flac",
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Ogg => "ogg",
        AudioFormat::Aac => "aac",
        AudioFormat::Mp4 => "m4a",
        AudioFormat::Other => "audio",
    }
}

/// Format a duration in seconds as `M:SS` (e.g. `"2:07"`).
pub(crate) fn format_duration(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    let m = total / 60;
    let s = total % 60;
    format!("{m}:{s:02}")
}

// ---------------------------------------------------------------------------
// Path display helpers
// ---------------------------------------------------------------------------

/// The display name of a folder: its final path component, or the whole
/// path (e.g. the filesystem root `/`) when there is no component.
pub(crate) fn folder_display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// The display name of an audio file: the final component of its path.
pub(crate) fn file_display_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string())
}

// ---------------------------------------------------------------------------
// Button styles
// ---------------------------------------------------------------------------

/// Button style for a folder / audio listing row. Neutral raised card by
/// default; a WARM wash when `highlighted` (the playing / selected audio
/// row); a lighter fill on hover.
pub(crate) fn row_button_style(highlighted: bool, status: button::Status) -> button::Style {
    let (bg, border_color) = if highlighted {
        (theme::WARM_DIM, theme::WARM_LINE)
    } else {
        match status {
            button::Status::Hovered | button::Status::Pressed => (theme::BG_3, theme::LINE),
            _ => (theme::BG_3, theme::LINE_2),
        }
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: theme::TEXT_1,
        border: iced::Border {
            color: border_color,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

/// Button style for a favourites / recent shelf pill: a full-round chip
/// that lifts on hover.
pub(crate) fn pill_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => theme::BG_3,
        _ => theme::BG_2,
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: theme::TEXT_1,
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    }
}

