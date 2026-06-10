/// Shared track/bus control widgets used by both the Arrange track header
/// and the Mixer channel strip. Factoring these here keeps icon choice,
/// color, and button style consistent across both surfaces.
use std::cell::Cell;

use iced::widget::canvas::{self, Frame, Geometry, Path};
use iced::widget::text::LineHeight;
use iced::widget::{
    button, canvas as canvas_widget, column, container, row, text, vertical_slider,
};
use iced::{alignment, mouse, Element, Font, Length, Point, Rectangle, Renderer, Size, Theme};
use resonance_audio::types::{BusId, TrackId};

use crate::message::*;
use crate::theme::{self, fa};
use crate::util::format_db;

/// Build a small icon button with the icon centered in a fixed-size
/// square. `size` is the icon glyph size in px; the button is sized at
/// `size + 8` so a size=12 icon yields a 20×20 square with the glyph
/// vertically + horizontally centered.
fn icon_button<'a>(
    glyph: char,
    color: iced::Color,
    size: u16,
) -> iced::widget::Container<'a, Message> {
    let cell = (size + 8) as f32;
    container(
        theme::icon(glyph)
            .size(f32::from(size))
            .color(color)
            .line_height(LineHeight::Relative(1.0)),
    )
    .width(cell)
    .height(cell)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
}

/// Record-arm toggle button (filled circle — red when armed).
pub fn record_arm_button<'a>(
    armed: bool,
    track_id: TrackId,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let color = if armed { theme::BAD } else { theme::TEXT_3 };
    button(icon_button(fa::CIRCLE, color, size))
        .on_press(Message::Track(TrackMessage::ToggleRecordArm(track_id)))
        .style(move |_theme, status| {
            if armed {
                theme::record_armed_button_style(status)
            } else {
                theme::small_button_style(status)
            }
        })
        .padding(0)
}

/// Mute toggle button (speaker-with-X — accent when muted).
pub fn mute_button<'a>(
    muted: bool,
    on_press: Message,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let color = if muted { theme::BAD } else { theme::TEXT_3 };
    button(icon_button(fa::VOLUME_XMARK, color, size))
        .on_press(on_press)
        .style(|_theme, status| theme::small_button_style(status))
        .padding(0)
}

/// Solo toggle button (headphones — yellow when soloed).
pub fn solo_button<'a>(
    soloed: bool,
    on_press: Message,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let color = if soloed { theme::WARM } else { theme::TEXT_3 };
    button(icon_button(fa::HEADPHONES, color, size))
        .on_press(on_press)
        .style(|_theme, status| theme::small_button_style(status))
        .padding(0)
}

/// Input-monitor toggle button (eye — green when monitoring).
pub fn monitor_button<'a>(
    enabled: bool,
    track_id: TrackId,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let color = if enabled { theme::GOOD } else { theme::TEXT_3 };
    button(icon_button(fa::EYE, color, size))
        .on_press(Message::Track(TrackMessage::ToggleMonitor(track_id)))
        .style(move |_theme, status| {
            theme::toggle_button_style(enabled, theme::GOOD, true, status)
        })
        .padding(0)
}

/// Mono/Stereo toggle button. One hollow circle for mono, two for stereo.
pub fn mono_button<'a>(
    is_mono: bool,
    track_id: TrackId,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let glyph = if is_mono {
        fa::CIRCLE_HOLLOW
    } else {
        fa::CIRCLE_HOLLOW_DOUBLE
    };
    let color = if is_mono { theme::TEXT_2 } else { theme::ACCENT_SOFT };
    button(icon_button(glyph, color, size))
        .on_press(Message::Track(TrackMessage::ToggleTrackMono(track_id)))
        .style(move |_theme, status| theme::mono_button_style(is_mono, status))
        .padding(0)
}

/// FX bypass toggle button. Small "FX" text label; dim when the
/// effects chain is active (normal state) and tinted with the accent
/// colour when bypassed (so the user can see at a glance which strips
/// have their chain disabled).
pub fn fx_bypass_button<'a>(
    bypassed: bool,
    on_press: Message,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let color = if bypassed { theme::ACCENT } else { theme::TEXT_3 };
    let cell = (size + 8) as f32;
    let label = container(
        text("FX")
            .size(f32::from(size - 1))
            .font(theme::UI_FONT_SEMIBOLD)
            .color(color)
            .line_height(LineHeight::Relative(1.0)),
    )
    .width(cell)
    .height(cell)
    .center_x(Length::Fill)
    .center_y(Length::Fill);
    button(label)
        .on_press(on_press)
        .style(|_theme, status| theme::small_button_style(status))
        .padding(0)
}

/// Trash/delete button (gray trash can).
pub fn delete_button<'a>(on_press: Message, size: u16) -> iced::widget::Button<'a, Message> {
    button(icon_button(fa::TRASH, theme::TEXT_3, size))
        .on_press(on_press)
        .style(|_theme, status| theme::small_button_style(status))
        .padding(0)
}

/// "Bounce in place" trigger — uses the audio-waveform glyph to imply
/// rendering MIDI/synth output to a flat audio track. Greyed out when
/// `enabled` is false (typically: source has no MIDI clips, or no
/// internal synth + no MIDI Out).
pub fn bounce_button<'a>(
    track_id: TrackId,
    enabled: bool,
    size: u16,
) -> iced::widget::Button<'a, Message> {
    let color = if enabled { theme::TEXT_2 } else { theme::TEXT_4 };
    let mut b = button(icon_button(fa::WAVE_SQUARE, color, size))
        .style(|_theme, status| theme::small_button_style(status))
        .padding(0);
    if enabled {
        b = b.on_press(Message::Track(TrackMessage::BounceInPlace(track_id)));
    }
    b
}

/// Bus remove button — same style as delete, used on bus strips.
pub fn bus_remove_button<'a>(bus_id: BusId, size: u16) -> iced::widget::Button<'a, Message> {
    delete_button(Message::Bus(BusMessage::RemoveBus(bus_id)), size)
}

/// Convert linear amplitude to meter bar height (logarithmic/dB scale).
fn level_to_bar_height(level: f32, max_height: f32) -> f32 {
    if level < 0.0001 {
        return 0.0;
    }
    let db = 20.0 * level.log10();
    let normalized = (db + 60.0) / 66.0; // -60dB=0, +6dB=1
    normalized.clamp(0.0, 1.0) * max_height
}

/// Get meter color based on signal level (green / yellow / red).
fn level_color(level: f32) -> iced::Color {
    if level < 0.0001 {
        return theme::METRONOME_ON;
    }
    let db = 20.0 * level.log10();
    if db > 0.0 {
        theme::RECORD_RED
    } else if db > -6.0 {
        theme::WARM
    } else {
        theme::METRONOME_ON
    }
}

/// Stereo vertical VU meter rendered as a `Canvas` so the bars + bg
/// live in a `canvas::Cache` that survives across redraws. Without
/// this, every strip's meter rebuilt a stack of `container`s every
/// frame — a major contributor to slow resize before the cache landed.
///
/// The cache is invalidated only when either level changes; pure
/// hover / window-resize redraws hit the cached geometry.
struct StereoMeterCanvas {
    level_l: f32,
    level_r: f32,
}

#[derive(Default)]
struct StereoMeterState {
    cache: canvas::Cache,
    cached_levels: Cell<(u32, u32)>,
}

impl<Message> canvas::Program<Message> for StereoMeterCanvas {
    type State = StereoMeterState;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        // Compare bit patterns rather than f32 == f32 — NaN-safe and
        // distinguishes `+0.0` from `-0.0` without false cache misses
        // on values that are visually identical.
        let fp = (self.level_l.to_bits(), self.level_r.to_bits());
        if state.cached_levels.get() != fp {
            state.cache.clear();
            state.cached_levels.set(fp);
        }
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            let bar_w = 6.0;
            let gap = 1.0;
            let h = bounds.height;
            draw_v_bar(frame, 0.0, bar_w, h, self.level_l);
            draw_v_bar(frame, bar_w + gap, bar_w, h, self.level_r);
        });
        vec![geometry]
    }
}

fn draw_v_bar(frame: &mut Frame, x: f32, w: f32, h: f32, level: f32) {
    let bg_rect = Path::rectangle(Point::new(x, 0.0), Size::new(w, h));
    frame.fill(&bg_rect, theme::BG_1);
    let bar_h = level_to_bar_height(level, h);
    if bar_h > 0.0 {
        let bar = Path::rectangle(Point::new(x, h - bar_h), Size::new(w, bar_h));
        frame.fill(&bar, level_color(level));
    }
}

/// Stereo vertical VU meter (L + R bars side-by-side). Fixed width
/// matches the previous `container`-based meter: 6+1+6 = 13 px.
pub fn meter_v<'a>(level_l: f32, level_r: f32, height: f32) -> Element<'a, Message> {
    canvas_widget(StereoMeterCanvas { level_l, level_r })
        .width(Length::Fixed(13.0))
        .height(Length::Fixed(height))
        .into()
}

/// Render the shared fader + meter + dB label block used by tracks, busses,
/// and master. The caller supplies the message factory for the slider —
/// this lets the same widget drive track/bus/master volumes.
pub fn fader_section<'a, F>(
    level_l: f32,
    level_r: f32,
    volume_db: f32,
    on_change: F,
) -> Element<'a, Message>
where
    F: 'a + Fn(f32) -> Message,
{
    let fader = vertical_slider(-60.0..=6.0f32, volume_db, on_change)
        .height(theme::FADER_HEIGHT)
        .step(0.1);
    let meters = meter_v(level_l, level_r, theme::FADER_HEIGHT);
    let label = text(format_db(volume_db))
        .size(9)
        .font(Font::MONOSPACE)
        .color(theme::TEXT_DIM);
    column![
        container(
            row![meters, fader]
                .spacing(4)
                .align_y(alignment::Vertical::Center)
        )
        .width(Length::Fill)
        .center_x(Length::Fill),
        label,
    ]
    .spacing(2)
    .align_x(alignment::Horizontal::Center)
    .into()
}
