/// Resonance dark industrial theme.
use iced::font::{Family, Weight};
use iced::widget::text::{Shaping, Text};
use iced::widget::{button, container, text, text_input};
use iced::{Color, Font, Theme};

/// Raw bytes of the bundled Font Awesome Solid font, extended with a custom
/// metronome glyph and renamed to the unique family "Resonance Icons" so
/// that a system-installed Font Awesome cannot shadow our modified copy.
pub const ICON_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/fa-solid-900.otf");

/// Font handle for the bundled, extended icon font.
pub const ICON_FONT: Font = Font {
    family: Family::Name("Resonance Icons"),
    weight: Weight::Black,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

/// Build an icon text element from a Font Awesome codepoint.
pub fn icon<'a>(codepoint: char) -> Text<'a> {
    text(codepoint.to_string())
        .font(ICON_FONT)
        .shaping(Shaping::Basic)
}

// Font Awesome Solid codepoints used in the UI.
pub mod fa {
    pub const PLAY: char = '\u{f04b}';
    pub const PAUSE: char = '\u{f04c}';
    pub const STOP: char = '\u{f04d}';
    pub const BACKWARD_STEP: char = '\u{f048}';
    pub const FORWARD_STEP: char = '\u{f051}';
    pub const CIRCLE: char = '\u{f111}';
    pub const BARS: char = '\u{f0c9}';
    pub const FOLDER_OPEN: char = '\u{f07c}';
    pub const FLOPPY_DISK: char = '\u{f0c7}';
    pub const MAGNIFYING_GLASS_PLUS: char = '\u{f00e}';
    pub const MAGNIFYING_GLASS_MINUS: char = '\u{f010}';
    /// Metronome icon (custom glyph added by tools/add_metronome_glyph.py).
    pub const METRONOME: char = '\u{f8db}';
    /// Single hollow circle — mono channel indicator. Custom glyph added
    /// by tools/add_mono_stereo_glyphs.py.
    pub const CIRCLE_HOLLOW: char = '\u{f8dc}';
    /// Two overlapping hollow circles — stereo channel indicator. Custom
    /// glyph added by tools/add_mono_stereo_glyphs.py.
    pub const CIRCLE_HOLLOW_DOUBLE: char = '\u{f8dd}';
    /// Bullseye — used for the loop (cycle) toggle.
    pub const BULLSEYE: char = '\u{f140}';
    /// Volume/speaker with an X — used for the mute button.
    pub const VOLUME_XMARK: char = '\u{f6a9}';
    /// Headphones — used for the solo button.
    pub const HEADPHONES: char = '\u{f025}';
    /// Microphone — used for audio tracks in the add-track menu.
    pub const MICROPHONE: char = '\u{f130}';
    /// Musical note — used for instrument tracks in the add-track menu.
    pub const MUSIC: char = '\u{f001}';
    pub const DRUM: char = '\u{f569}';
    pub const GUITAR: char = '\u{f7a6}';
    pub const WAVE_SQUARE: char = '\u{f83e}';
    pub const COMPACT_DISC: char = '\u{f51f}';
    pub const SLIDERS: char = '\u{f1de}';
    /// Eye — used for the input-monitor toggle.
    pub const EYE: char = '\u{f06e}';
    /// Trash can — used for the track delete button.
    pub const TRASH: char = '\u{f1f8}';
}

// Core palette
pub const BG: Color = Color::from_rgb(
    0x0f as f32 / 255.0,
    0x0f as f32 / 255.0,
    0x0f as f32 / 255.0,
);

pub const PANEL: Color = Color::from_rgb(
    0x1a as f32 / 255.0,
    0x1a as f32 / 255.0,
    0x1a as f32 / 255.0,
);

pub const PANEL_DARK: Color = Color::from_rgb(
    0x14 as f32 / 255.0,
    0x14 as f32 / 255.0,
    0x14 as f32 / 255.0,
);

pub const SEPARATOR: Color = Color::from_rgb(
    0x2a as f32 / 255.0,
    0x2a as f32 / 255.0,
    0x2a as f32 / 255.0,
);

pub const ACCENT: Color = Color::from_rgb(
    0xe8 as f32 / 255.0,
    0x83 as f32 / 255.0,
    0x2a as f32 / 255.0,
);

pub const SOLO_YELLOW: Color = Color::from_rgb(
    0xe6 as f32 / 255.0,
    0xcc as f32 / 255.0,
    0x1a as f32 / 255.0,
);

pub const CLIP_BODY: Color = Color::from_rgb(
    0x4a as f32 / 255.0,
    0x7f as f32 / 255.0,
    0xa5 as f32 / 255.0,
);

pub const CLIP_HEADER: Color = Color::from_rgb(
    0x3a as f32 / 255.0,
    0x6f as f32 / 255.0,
    0x95 as f32 / 255.0,
);

pub const TEXT: Color = Color::from_rgb(
    0xe0 as f32 / 255.0,
    0xe0 as f32 / 255.0,
    0xe0 as f32 / 255.0,
);

pub const TEXT_DIM: Color = Color::from_rgb(
    0x80 as f32 / 255.0,
    0x80 as f32 / 255.0,
    0x80 as f32 / 255.0,
);

pub const RULER_BG: Color = Color::from_rgb(
    0x18 as f32 / 255.0,
    0x18 as f32 / 255.0,
    0x18 as f32 / 255.0,
);

pub const TRACK_LINE: Color = Color::from_rgb(
    0x22 as f32 / 255.0,
    0x22 as f32 / 255.0,
    0x22 as f32 / 255.0,
);

pub const RECORD_RED: Color = Color::from_rgb(
    0xcc as f32 / 255.0,
    0x33 as f32 / 255.0,
    0x33 as f32 / 255.0,
);

pub const PANEL_ARMED: Color = Color::from_rgb(
    0x1f as f32 / 255.0,
    0x14 as f32 / 255.0,
    0x14 as f32 / 255.0,
);

/// Subtle highlight for the currently selected track header and lane.
pub const PANEL_SELECTED: Color = Color::from_rgb(
    0x1a as f32 / 255.0,
    0x1a as f32 / 255.0,
    0x22 as f32 / 255.0,
);

/// Border accent for the selected track header.
pub const SELECTED_BORDER: Color = Color::from_rgb(
    0x4a as f32 / 255.0,
    0x4a as f32 / 255.0,
    0x6a as f32 / 255.0,
);

pub const METER_BG: Color = Color::from_rgb(
    0x08 as f32 / 255.0,
    0x08 as f32 / 255.0,
    0x08 as f32 / 255.0,
);

pub const BAR_LINE: Color = Color::from_rgb(
    0x30 as f32 / 255.0,
    0x30 as f32 / 255.0,
    0x30 as f32 / 255.0,
);

pub const BEAT_LINE: Color = Color::from_rgb(
    0x20 as f32 / 255.0,
    0x20 as f32 / 255.0,
    0x20 as f32 / 255.0,
);

pub const METRONOME_ON: Color = Color::from_rgb(
    0x4a as f32 / 255.0,
    0xcc as f32 / 255.0,
    0x4a as f32 / 255.0,
);

pub const LOOP_MARKER: Color = Color::from_rgb(
    0xe6 as f32 / 255.0,
    0xb8 as f32 / 255.0,
    0x1a as f32 / 255.0,
);

pub const CLIP_SELECTED_BORDER: Color = Color::from_rgb(
    0xe8 as f32 / 255.0,
    0x83 as f32 / 255.0,
    0x2a as f32 / 255.0,
);

pub const TRACK_HEIGHT: f32 = 80.0;
pub const RULER_HEIGHT: f32 = 30.0;
/// Height of each global track row (tempo, time signature) in the
/// collapsible area between the ruler and the regular tracks.
pub const GLOBAL_TRACK_ROW_HEIGHT: f32 = 40.0;
/// Background for global track rows (slightly distinct from ruler).
pub const GLOBAL_TRACK_BG: Color = Color::from_rgb(
    0x15 as f32 / 255.0,
    0x15 as f32 / 255.0,
    0x15 as f32 / 255.0,
);
pub const TRACK_HEADER_WIDTH: u16 = 180;
pub const MIXER_STRIP_WIDTH: u16 = 160;
pub const MASTER_STRIP_WIDTH: u16 = 140;

/// Height of the vertical fader used in mixer strips and master strip.
pub const FADER_HEIGHT: f32 = 120.0;
/// Pixel radius around a clip's left/right edge that starts a trim (not move).
pub const CLIP_EDGE_THRESHOLD: f32 = 6.0;
/// VU-meter peak decay factor applied per frame tick.
pub const PEAK_DECAY: f32 = 0.85;
/// Tick interval (ms) for the subscription timer that drains engine events.
pub const TICK_INTERVAL_MS: u64 = 16;

pub fn resonance_theme() -> Theme {
    Theme::Dark
}

pub fn transport_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.2, 0.2, 0.2),
        button::Status::Pressed => Color::from_rgb(0.15, 0.15, 0.15),
        _ => Color::from_rgb(0.12, 0.12, 0.12),
    };

    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: TEXT,
        border: iced::Border {
            color: SEPARATOR,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

pub fn record_armed_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.35, 0.12, 0.12),
        button::Status::Pressed => Color::from_rgb(0.25, 0.08, 0.08),
        _ => Color::from_rgb(0.30, 0.10, 0.10),
    };

    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: RECORD_RED,
        border: iced::Border {
            color: RECORD_RED,
            width: 1.0,
            radius: 2.0.into(),
        },
        ..Default::default()
    }
}

pub fn tab_button_style(active: bool, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.22, 0.22, 0.22),
        button::Status::Pressed => Color::from_rgb(0.15, 0.15, 0.15),
        _ => {
            if active {
                Color::from_rgb(0.18, 0.18, 0.18)
            } else {
                Color::TRANSPARENT
            }
        }
    };

    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: if active { ACCENT } else { TEXT_DIM },
        border: iced::Border {
            color: if active { ACCENT } else { Color::TRANSPARENT },
            width: if active { 1.0 } else { 0.0 },
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

/// Button style for a section placement in the Compose tab strip. Fills the
/// button with the section's configured color (dimmed when inactive) and
/// highlights it with an accent border when it is the current selection.
pub fn section_button_style(
    active: bool,
    color: [u8; 3],
    status: button::Status,
) -> button::Style {
    let base = Color::from_rgb(
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
    );
    let panel = PANEL;
    let inactive = Color::from_rgb(
        panel.r * 0.7 + base.r * 0.3,
        panel.g * 0.7 + base.g * 0.3,
        panel.b * 0.7 + base.b * 0.3,
    );
    let active_bg = Color::from_rgb(
        panel.r * 0.4 + base.r * 0.6,
        panel.g * 0.4 + base.g * 0.6,
        panel.b * 0.4 + base.b * 0.6,
    );
    let bg = match status {
        button::Status::Hovered => active_bg,
        button::Status::Pressed => base,
        _ => {
            if active {
                active_bg
            } else {
                inactive
            }
        }
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: TEXT,
        border: iced::Border {
            color: if active { ACCENT } else { SEPARATOR },
            width: if active { 1.5 } else { 1.0 },
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

/// Toggle button style for active/inactive states with a custom active color.
/// Used for monitor, metronome, and punch buttons.
pub fn toggle_button_style(
    active: bool,
    active_color: Color,
    small: bool,
    status: button::Status,
) -> button::Style {
    if active {
        let bg = match status {
            button::Status::Hovered => Color::from_rgb(0.15, 0.25, 0.15),
            button::Status::Pressed => Color::from_rgb(0.10, 0.20, 0.10),
            _ => Color::from_rgb(0.12, 0.20, 0.12),
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: active_color,
            border: iced::Border {
                color: active_color,
                width: 1.0,
                radius: if small { 2.0 } else { 4.0 }.into(),
            },
            ..Default::default()
        }
    } else if small {
        small_button_style(status)
    } else {
        transport_button_style(status)
    }
}

/// Mono/Stereo toggle button style.
pub fn mono_button_style(is_mono: bool, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.20, 0.20, 0.25),
        button::Status::Pressed => Color::from_rgb(0.15, 0.15, 0.20),
        _ => Color::from_rgb(0.18, 0.18, 0.22),
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: if is_mono { TEXT } else { ACCENT },
        border: iced::Border {
            color: if is_mono { SEPARATOR } else { ACCENT },
            width: 1.0,
            radius: 2.0.into(),
        },
        ..Default::default()
    }
}

pub fn small_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.22, 0.22, 0.22),
        button::Status::Pressed => Color::from_rgb(0.15, 0.15, 0.15),
        _ => Color::TRANSPARENT,
    };

    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: TEXT,
        border: iced::Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 2.0.into(),
        },
        ..Default::default()
    }
}

/// Bordered container style for the compound timing panel (BPM / time sig /
/// position / metronome).
pub fn timing_panel_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(PANEL_DARK)),
        border: iced::Border {
            color: SEPARATOR,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}

/// Borderless text input used inside the timing panel. Transparent
/// background, no border, accent text — blends into the surrounding panel.
pub fn borderless_text_input_style(
    _theme: &Theme,
    _status: text_input::Status,
) -> text_input::Style {
    text_input::Style {
        background: iced::Background::Color(Color::TRANSPARENT),
        border: iced::Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        icon: TEXT_DIM,
        placeholder: Color { a: 0.4, ..TEXT_DIM },
        value: ACCENT,
        selection: Color {
            r: ACCENT.r,
            g: ACCENT.g,
            b: ACCENT.b,
            a: 0.35,
        },
    }
}

/// Style for floating buttons that sit on top of the timeline canvas
/// (e.g. the zoom +/- overlay). Semi-opaque so it reads against clips.
pub fn floating_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgba(0.22, 0.22, 0.22, 0.92),
        button::Status::Pressed => Color::from_rgba(0.15, 0.15, 0.15, 0.92),
        _ => Color::from_rgba(0.12, 0.12, 0.12, 0.85),
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: TEXT,
        border: iced::Border {
            color: SEPARATOR,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

/// Red-tinted button for destructive actions (delete confirmations).
pub fn destructive_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.50, 0.14, 0.14),
        button::Status::Pressed => Color::from_rgb(0.38, 0.10, 0.10),
        _ => Color::from_rgb(0.45, 0.12, 0.12),
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: Color::WHITE,
        border: iced::Border {
            color: RECORD_RED,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

/// Accent-coloured button for primary/positive actions (e.g. "Save & Quit").
pub fn accent_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color::from_rgb(0.18, 0.50, 0.72),
        button::Status::Pressed => Color::from_rgb(0.12, 0.38, 0.58),
        _ => Color::from_rgb(ACCENT.r, ACCENT.g, ACCENT.b),
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: Color::WHITE,
        border: iced::Border {
            color: ACCENT,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

// ---- Container style helpers -------------------------------------------
// These wrap the "flat background container with no border" pattern that
// appears ~80 times across the view module.

/// Flat PANEL background, no border.
pub fn panel_bg(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(PANEL)),
        ..Default::default()
    }
}

/// Flat PANEL_DARK background, no border.
pub fn panel_dark_bg(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(PANEL_DARK)),
        ..Default::default()
    }
}

/// Flat BG (base) background, no border.
pub fn base_bg(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG)),
        ..Default::default()
    }
}

/// Flat SEPARATOR-color background (for 1px separator Spaces).
pub fn separator_bg(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(SEPARATOR)),
        ..Default::default()
    }
}

/// PANEL background with a subtle SEPARATOR outline. Used on track
/// header frames.
pub fn panel_outlined(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(PANEL)),
        border: iced::Border {
            color: SEPARATOR,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

/// PANEL_DARK background with a thin SEPARATOR outline. Used on mixer
/// strip frames.
pub fn panel_dark_outlined(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(PANEL_DARK)),
        border: iced::Border {
            color: SEPARATOR,
            width: 0.5,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}
