/// Resonance dark industrial theme.
use iced::widget::button;
use iced::{Color, Theme};

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

pub const PUNCH_MARKER: Color = Color::from_rgb(
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
