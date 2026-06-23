//! Resonance theme — soft dark, lavender-accent design system.
//!
//! Tokens follow the redesign spec: a layered backdrop (`BG_0..BG_4`),
//! two border weights (`LINE`, `LINE_2`), four text greys (`TEXT_1..TEXT_4`),
//! a lavender primary accent with derived soft / dim / line variants, and
//! three semantic colours (`WARM`, `GOOD`, `BAD`). Older constant names
//! (`PANEL`, `TEXT`, `ACCENT`, ...) are preserved as aliases so the rest of
//! the codebase keeps compiling while it migrates piece by piece.
use iced::font::{Family, Weight};
use iced::widget::text::{Shaping, Text};
use iced::widget::{button, container, text, text_input, Container, Row};
use iced::{Color, Font, Theme};

/// Raw bytes of the bundled Font Awesome Solid font, extended with a custom
/// metronome glyph and renamed to the unique family "Resonance Icons" so
/// that a system-installed Font Awesome cannot shadow our modified copy.
pub const ICON_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/fa-solid-900.otf");

/// Raw bytes for every UI font face we ship. Iced loads each independently
/// — registration is one entry per weight / style. Order matters only for
/// load priority; Iced picks the closest available weight for a given
/// `Font` request.
pub const UI_FONT_FACES: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Geist-Light.ttf"),
    include_bytes!("../assets/fonts/Geist-Regular.ttf"),
    include_bytes!("../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../assets/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Geist-Bold.ttf"),
    include_bytes!("../assets/fonts/GeistMono-Regular.ttf"),
    include_bytes!("../assets/fonts/GeistMono-Medium.ttf"),
    include_bytes!("../assets/fonts/GeistMono-SemiBold.ttf"),
    include_bytes!("../assets/fonts/InstrumentSerif-Regular.ttf"),
    include_bytes!("../assets/fonts/InstrumentSerif-Italic.ttf"),
];

/// Font handle for the bundled, extended icon font.
pub const ICON_FONT: Font = Font {
    family: Family::Name("Resonance Icons"),
    weight: Weight::Black,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

/// Primary UI sans (Geist). When the bundled font isn't available the
/// platform falls back to the system sans.
pub const UI_FONT: Font = Font {
    family: Family::Name("Geist"),
    weight: Weight::Normal,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

/// Medium-weight UI sans for emphasised labels and primary buttons.
pub const UI_FONT_MEDIUM: Font = Font {
    family: Family::Name("Geist"),
    weight: Weight::Medium,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

/// Semibold UI sans for tab labels and section headers.
pub const UI_FONT_SEMIBOLD: Font = Font {
    family: Family::Name("Geist"),
    weight: Weight::Semibold,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

/// Monospace UI font (Geist Mono). Used for numeric readouts — BPM, dB,
/// bar counts, seeds.
pub const MONO_FONT: Font = Font {
    family: Family::Name("Geist Mono"),
    weight: Weight::Normal,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Normal,
};

/// Italic display serif (Instrument Serif). Used sparingly for the project
/// title in the chrome and for chord symbols in the Compose view.
pub const SERIF_ITALIC_FONT: Font = Font {
    family: Family::Name("Instrument Serif"),
    weight: Weight::Normal,
    stretch: iced::font::Stretch::Normal,
    style: iced::font::Style::Italic,
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
    /// Caret pointing right — collapsed indicator.
    pub const CARET_RIGHT: char = '\u{f0da}';
    /// Caret pointing down — expanded indicator.
    pub const CARET_DOWN: char = '\u{f0d7}';
    /// Arrow pointing right — used for output routing labels.
    pub const ARROW_RIGHT: char = '\u{f061}';
    /// Filled circle with an "i" — hover-tooltip info marker.
    pub const CIRCLE_INFO: char = '\u{f05a}';
    /// Counter-clockwise rotating arrow — used for "regenerate / reroll"
    /// affordances next to a primary Generate button.
    pub const ARROW_ROTATE_LEFT: char = '\u{f0e2}';
}

// ---------------------------------------------------------------------------
// Hex helpers — used to express palette swatches as readable hex pairs.
// ---------------------------------------------------------------------------

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::from_rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
}

// ---------------------------------------------------------------------------
// Backdrop layers — five steps from window to raised control.
// ---------------------------------------------------------------------------

/// Page / window backdrop. The OS window sits on this.
pub const BG_0: Color = rgb(0x0f, 0x10, 0x13);
/// App body — the surface inside the window chrome.
pub const BG_1: Color = rgb(0x15, 0x16, 0x1b);
/// Panels, channel strips, cards.
pub const BG_2: Color = rgb(0x1b, 0x1d, 0x23);
/// Hover state, raised controls.
pub const BG_3: Color = rgb(0x23, 0x26, 0x2e);

// ---------------------------------------------------------------------------
// Borders / hairlines.
// ---------------------------------------------------------------------------

/// Standard borders.
pub const LINE: Color = rgb(0x27, 0x2a, 0x31);
/// Subtle dividers / inner hairlines.
pub const LINE_2: Color = rgb(0x1f, 0x22, 0x29);

// ---------------------------------------------------------------------------
// Text greys.
// ---------------------------------------------------------------------------

/// Primary text.
pub const TEXT_1: Color = rgb(0xe8, 0xe7, 0xe3);
/// Secondary text.
pub const TEXT_2: Color = rgb(0x9a, 0xa0, 0xac);
/// Tertiary / labels.
pub const TEXT_3: Color = rgb(0x5d, 0x62, 0x6d);
/// Disabled.
pub const TEXT_4: Color = rgb(0x3f, 0x43, 0x4c);

// ---------------------------------------------------------------------------
// Accent (lavender) + semantic colours.
// ---------------------------------------------------------------------------

/// Lavender primary accent — selection, brand, MIDI.
pub const ACCENT: Color = rgb(0x8b, 0x6d, 0xff);
/// Lighter lavender — text on dim backgrounds.
pub const ACCENT_SOFT: Color = rgb(0xa8, 0x92, 0xff);
/// Lavender wash — selection backgrounds.
pub const ACCENT_DIM: Color = rgba(0x8b, 0x6d, 0xff, 0.16);
/// Lavender border — selection outlines.
pub const ACCENT_LINE: Color = rgba(0x8b, 0x6d, 0xff, 0.34);

/// Warm amber — audio clips, busses, playhead.
pub const WARM: Color = rgb(0xe8, 0xc4, 0x7b);
/// Warm border — bus strip outlines.
pub const WARM_LINE: Color = rgba(0xe8, 0xc4, 0x7b, 0.34);

/// Mint green — meters, success.
pub const GOOD: Color = rgb(0x6d, 0xd6, 0xa3);
/// Soft pink — mute, peaking, errors.
pub const BAD: Color = rgb(0xe8, 0x7b, 0x8b);

// ---------------------------------------------------------------------------
// Legacy aliases — keep the rest of the codebase compiling while the views
// migrate. New code should use the tokens above directly.
// ---------------------------------------------------------------------------

/// Window backdrop. Aliased to BG_1 so existing `base_bg` containers paint
/// the app body color.
pub const BG: Color = BG_1;
/// Panel background (channel strips, track headers, cards).
pub const PANEL: Color = BG_2;
/// Recessed sub-area — VU meter background, fader track, mini buttons.
pub const PANEL_DARK: Color = BG_1;
/// Standard border.
pub const SEPARATOR: Color = LINE;
/// Primary text.
pub const TEXT: Color = TEXT_1;
/// Secondary text.
pub const TEXT_DIM: Color = TEXT_2;
/// Record-armed glow / record button accent.
pub const RECORD_RED: Color = BAD;
/// Track lane background while a recording pass is in progress.
pub const PANEL_ARMED: Color = rgb(0x24, 0x1a, 0x1f);
/// Sub-track strip background on the Mixer. One step darker than the
/// normal strip so a parent and its expanded sub-tracks read as a
/// cluster — the recessed shade groups them visually even before the
/// left-edge accent rail kicks in.
pub const MIXER_SUB_STRIP_BG: Color = BG_1;
/// Left-edge accent rail color on a sub-track strip. Subtle lavender
/// so the parent → child relationship reads at a glance without
/// competing with the selection outline.
pub const MIXER_SUB_STRIP_RAIL: Color = ACCENT_LINE;
/// Bar line in the timeline ruler / lane.
pub const BAR_LINE: Color = LINE;
/// Beat sub-line in the timeline lane.
pub const BEAT_LINE: Color = LINE_2;
/// Metronome enabled colour.
pub const METRONOME_ON: Color = GOOD;
/// Background for the global track rows (tempo / signature).
pub const GLOBAL_TRACK_BG: Color = BG_2;

// ---------------------------------------------------------------------------
// Layout constants. Values follow the redesign spec (96px row, 28px ruler,
// 280px track-list column, 140px mixer strip) after the 2026-06 whitespace
// pass loosened the spacing scale.
// ---------------------------------------------------------------------------

/// Arrange-view track row height — matches the design's "balanced" density.
pub const TRACK_HEIGHT: f32 = 96.0;
/// Timeline ruler height.
pub const RULER_HEIGHT: f32 = 28.0;
/// Section band sitting under the ruler — the section-pill strip on the
/// arrange canvas.
pub const SECTION_BAND_HEIGHT: f32 = 22.0;
/// Height of the always-visible "GLOBAL" shelf header strip — the
/// one-line summary bar (`6/8 · 90 BPM · B min · N chords`) that hosts
/// the caret-toggle and stays present even when the shelf is collapsed.
pub const GLOBAL_SHELF_HEADER_HEIGHT: f32 = 32.0;
/// Chord lane height inside the expanded global shelf — section tabs
/// stack above chord blocks so the row reads two lines tall.
pub const GLOBAL_TRACK_CHORD_HEIGHT: f32 = 56.0;
/// Tempo automation lane height inside the expanded global shelf.
pub const GLOBAL_TRACK_TEMPO_HEIGHT: f32 = 40.0;
/// Time-signature lane height inside the expanded global shelf — a
/// single-line strip of pills + downbeat ticks.
pub const GLOBAL_TRACK_SIG_HEIGHT: f32 = 28.0;
/// Width of the small glyph tile shown next to each global-track label
/// in the shelf's left column.
pub const GLOBAL_TRACK_GLYPH_SIZE: f32 = 22.0;
/// Track-list column width on the Arrange view.
pub const TRACK_HEADER_WIDTH: f32 = 280.0;
/// Vertical inset of a clip card inside its arrange track lane. The clip
/// body spans `TRACK_HEIGHT - 2 * CLIP_LANE_INSET`.
pub const CLIP_LANE_INSET: f32 = 10.0;
/// Standard channel strip width on the Mixer.
pub const MIXER_STRIP_WIDTH: f32 = 140.0;
/// Sub-track strip width on the Mixer. Sub-tracks are fed from one
/// non-main output of their parent's instrument plugin — they have no
/// FX chain, no input, and no record arm, so the strip is narrower than
/// a normal channel strip. The narrower width also creates a visual
/// rhythm that telegraphs "this is a child of the strip on its left".
pub const MIXER_SUB_STRIP_WIDTH: f32 = 92.0;
/// Width of the lavender-tinted left-edge accent rail on a sub-track
/// strip. Sits flush against the left edge of the strip card so the eye
/// reads a parent → child relationship even before reading the strip's
/// dimmed name.
pub const MIXER_SUB_STRIP_RAIL_WIDTH: f32 = 2.0;
/// Master strip width.
pub const MASTER_STRIP_WIDTH: f32 = 156.0;
/// Inspector column width on the Mixer.
pub const INSPECTOR_WIDTH: f32 = 320.0;
/// Reference & A/B right-rail width on the Mixer (design doc #184/#198).
pub const REFERENCE_PANEL_WIDTH: f32 = 360.0;
/// Inner padding shared by the Mixer's right rails (inspector, reference).
pub const RAIL_PADDING: f32 = 26.0;
/// Horizontal gap between unrelated strips in a mixer strip lane.
/// Parent + sub-track clusters stay flush (0 px) inside this gap.
pub const MIXER_STRIP_GAP: f32 = 16.0;
/// Horizontal lead-in/lead-out padding of the mixer strip lanes.
pub const MIXER_LANE_HPAD: f32 = 26.0;
/// Right-rail column width on the Compose view.
pub const COMPOSE_RAIL_WIDTH: u16 = 324;

/// Height of the vertical fader used in mixer strips and master strip.
pub const FADER_HEIGHT: f32 = 120.0;
/// Fixed total height for a track/master mixer strip. Pins the fader at
/// the bottom of the strip and lets the FX list scroll inside instead of
/// resizing the entire strip — keeps mixer resize cheap.
///
/// Sized together with `BUS_STRIP_HEIGHT` so both lanes plus the 1px
/// separator fit the 1440×900 minimum window under the 62px chrome +
/// 74px transport: 440 + 1 + 320 = 761 ≤ 900 − 136.
pub const MIXER_STRIP_HEIGHT: u16 = 440;
/// Fixed height for bus strips. Shorter than track strips since busses
/// have no instrument slot and no M/S/arm/monitor block — see the
/// `MIXER_STRIP_HEIGHT` budget note for how the two heights are derived.
pub const BUS_STRIP_HEIGHT: u16 = 320;
/// Pixel radius around a clip's left/right edge that starts a trim (not move).
pub const CLIP_EDGE_THRESHOLD: f32 = 6.0;

// ---------------------------------------------------------------------------
// Radius scale.
// ---------------------------------------------------------------------------

/// Cells, tiny buttons.
pub const RADIUS_XS: f32 = 4.0;
/// Segmented tabs.
pub const RADIUS_SM: f32 = 6.0;
/// Standard buttons + inputs.
pub const RADIUS_MD: f32 = 7.0;
/// Clip cards, chord cards, instrument slots.
pub const RADIUS_LG: f32 = 8.0;
/// Strip cards, drum grid panel.
pub const RADIUS_XL: f32 = 12.0;

pub fn resonance_theme() -> Theme {
    Theme::Dark
}

// ---------------------------------------------------------------------------
// Button styles.
// ---------------------------------------------------------------------------

pub fn transport_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => BG_3,
        button::Status::Pressed => LINE_2,
        _ => BG_2,
    };

    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: TEXT_1,
        border: iced::Border {
            color: LINE,
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        ..Default::default()
    }
}

pub fn record_armed_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => rgb(0x35, 0x18, 0x1f),
        button::Status::Pressed => rgb(0x2a, 0x12, 0x18),
        _ => rgb(0x2a, 0x14, 0x1a),
    };

    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: BAD,
        border: iced::Border {
            color: BAD,
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        ..Default::default()
    }
}

/// Tab-style button used for the chrome's segmented Arrange/Mixer/Compose
/// nav. Active tab fills with `BG_3` and shows primary text; inactive tabs
/// are transparent with `TEXT_2`.
pub fn tab_button_style(active: bool, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => {
            if active {
                BG_3
            } else {
                BG_2
            }
        }
        button::Status::Pressed => LINE_2,
        _ => {
            if active {
                BG_3
            } else {
                Color::TRANSPARENT
            }
        }
    };

    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: if active { TEXT_1 } else { TEXT_2 },
        border: iced::Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

/// Chip-style button used for prominent section placements on the Compose
/// strip. Active section gets a lavender wash + accent border to read as
/// "currently editing"; inactive sections are flat `BG_2` cards with a
/// hairline border. Unlike `section_button_style` the chip body does not
/// reflect the section's color — the color is shown as a small dot inside
/// the chip so the lavender selection state stays unambiguous.
pub fn section_chip_button_style(active: bool, status: button::Status) -> button::Style {
    let bg = match (active, status) {
        (true, button::Status::Hovered) => Color { a: 0.22, ..ACCENT },
        (true, _) => ACCENT_DIM,
        (false, button::Status::Hovered) => BG_3,
        (false, button::Status::Pressed) => LINE_2,
        (false, _) => BG_2,
    };
    let border_color = if active { ACCENT } else { LINE_2 };
    let border_width = if active { 1.5 } else { 1.0 };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: TEXT_1,
        border: iced::Border {
            color: border_color,
            width: border_width,
            radius: RADIUS_LG.into(),
        },
        ..Default::default()
    }
}

/// Container style for the small "EDITING" pill shown on the active section
/// chip and on the lane inspector header card. Lavender wash + accent
/// border + soft lavender text on a rounded full-pill shape.
pub fn editing_pill_style(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(ACCENT_SOFT),
        background: Some(iced::Background::Color(ACCENT_DIM)),
        border: iced::Border {
            color: ACCENT_LINE,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    }
}

/// Warm-tinted variant of `editing_pill_style` — used for the "PER-TRACK"
/// pill on the lane inspector header when a track lane is active.
pub fn editing_pill_warm_style(_theme: &Theme) -> container::Style {
    container::Style {
        text_color: Some(WARM),
        background: Some(iced::Background::Color(rgba(0xe8, 0xc4, 0x7b, 0.14))),
        border: iced::Border {
            color: WARM_LINE,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    }
}

/// Card-style container for the lane inspector "EDITING …" context header.
/// Lavender wash + accent border when a section lane is active.
pub fn editing_header_card_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(ACCENT_DIM)),
        border: iced::Border {
            color: ACCENT_LINE,
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        ..Default::default()
    }
}

/// Warm-tinted variant of `editing_header_card_style` — used when a track
/// lane is active in the lane inspector.
pub fn editing_header_card_warm_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(rgba(0xe8, 0xc4, 0x7b, 0.10))),
        border: iced::Border {
            color: WARM_LINE,
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        ..Default::default()
    }
}

/// Toggle button style for active/inactive states with a custom active color.
/// Used for monitor, metronome, and punch buttons. Active state uses the
/// `active_color` for border and text against a soft tinted bg.
pub fn toggle_button_style(
    active: bool,
    active_color: Color,
    small: bool,
    status: button::Status,
) -> button::Style {
    if active {
        let bg = match status {
            button::Status::Hovered => Color {
                a: 0.22,
                ..active_color
            },
            button::Status::Pressed => Color {
                a: 0.30,
                ..active_color
            },
            _ => Color {
                a: 0.16,
                ..active_color
            },
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: active_color,
            border: iced::Border {
                color: active_color,
                width: 1.0,
                radius: if small { RADIUS_XS } else { RADIUS_LG }.into(),
            },
            ..Default::default()
        }
    } else if small {
        small_button_style(status)
    } else {
        transport_button_style(status)
    }
}

/// Mono/Stereo toggle button style. Lavender outline when stereo, neutral
/// when forced mono.
pub fn mono_button_style(is_mono: bool, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => BG_3,
        button::Status::Pressed => LINE_2,
        _ => BG_2,
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: if is_mono { TEXT_2 } else { ACCENT_SOFT },
        border: iced::Border {
            color: if is_mono { LINE } else { ACCENT_LINE },
            width: 1.0,
            radius: RADIUS_XS.into(),
        },
        ..Default::default()
    }
}

pub fn small_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => BG_3,
        button::Status::Pressed => LINE_2,
        _ => Color::TRANSPARENT,
    };

    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: TEXT_1,
        border: iced::Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: RADIUS_XS.into(),
        },
        ..Default::default()
    }
}

/// "Ghost" button — transparent body, hairline border, secondary text.
/// Used for chrome controls (⌘K, Share) and toolbar actions ("Edit
/// section", "Export chords", "Snapshot", ...).
pub fn ghost_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => BG_2,
        button::Status::Pressed => LINE_2,
        _ => Color::TRANSPARENT,
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: TEXT_2,
        border: iced::Border {
            color: LINE,
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

/// Lavender primary action button — used for the Compose generator's
/// "Generate" and the play button when in the chrome.
pub fn primary_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => ACCENT_SOFT,
        button::Status::Pressed => Color {
            r: ACCENT.r * 0.82,
            g: ACCENT.g * 0.82,
            b: ACCENT.b * 0.95,
            a: 1.0,
        },
        _ => ACCENT,
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: rgb(0x0e, 0x0a, 0x1f),
        border: iced::Border {
            color: ACCENT,
            width: 0.0,
            radius: RADIUS_MD.into(),
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
        icon: TEXT_2,
        placeholder: Color { a: 0.4, ..TEXT_2 },
        value: ACCENT_SOFT,
        selection: Color {
            a: 0.35,
            ..ACCENT
        },
    }
}

/// Style for floating buttons that sit on top of the timeline canvas
/// (e.g. the zoom +/- overlay). Semi-opaque so it reads against clips.
pub fn floating_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Color {
            a: 0.92,
            ..BG_3
        },
        button::Status::Pressed => Color {
            a: 0.92,
            ..LINE_2
        },
        _ => Color {
            a: 0.85,
            ..BG_2
        },
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: TEXT_1,
        border: iced::Border {
            color: LINE,
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

/// Red-tinted button for destructive actions (delete confirmations).
pub fn destructive_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => rgb(0x4a, 0x18, 0x22),
        button::Status::Pressed => rgb(0x36, 0x10, 0x18),
        _ => rgb(0x3e, 0x14, 0x1d),
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: TEXT_1,
        border: iced::Border {
            color: BAD,
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

// ---- Container style helpers ------------------------------------------------
// These wrap the most common backdrop+border pairings used throughout the
// view layer.

/// Flat BG_2 (panel) background, no border.
pub fn panel_bg(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_2)),
        ..Default::default()
    }
}

/// Flat BG (app body) background, no border.
pub fn base_bg(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_1)),
        ..Default::default()
    }
}

/// Flat LINE-color background (used for 1px separator Spaces).
pub fn separator_bg(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(LINE_2)),
        ..Default::default()
    }
}

/// Panel background with a subtle hairline outline. Used on track header
/// frames and other "card" containers.
pub fn panel_outlined(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_2)),
        border: iced::Border {
            color: LINE_2,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

/// Recessed background with a thin hairline outline. Used on mixer
/// strip frames and other inner panels.
pub fn panel_dark_outlined(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_1)),
        border: iced::Border {
            color: LINE_2,
            width: 0.5,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

/// Card-style container with a lavender selection outline. Used for the
/// currently selected channel strip and chord card.
pub fn card_selected(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_2)),
        border: iced::Border {
            color: ACCENT_LINE,
            width: 1.0,
            radius: RADIUS_XL.into(),
        },
        ..Default::default()
    }
}

/// Card-style container with a warm/amber outline. Used for bus strips.
pub fn card_warm(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(BG_2)),
        border: iced::Border {
            color: WARM_LINE,
            width: 1.0,
            radius: RADIUS_XL.into(),
        },
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Keyboard-shortcut presentation — keycaps + nav / edited / conflict states.
//
// Foundations for the command palette and the Preferences › Keyboard panel
// (epic #58). Shortcuts read as physical keycaps rendered in `MONO_FONT`;
// modifiers show as glyphs (⌘ ⌥ ⇧ ↵) so a chord like ⌘⇧M reads as one
// compact row. Selection / edited / conflict states reuse the existing
// semantic tokens (`ACCENT_*`, `WARM*`, `BAD`) so nothing looks bolted on.
// ---------------------------------------------------------------------------

/// Modifier- and special-key glyphs used to render a key chord as keycaps.
/// Centralised here so call sites never hardcode the codepoints.
pub mod kbd {
    /// Command / Super (⌘).
    pub const CMD: char = '\u{2318}';
    /// Option / Alt (⌥).
    pub const OPTION: char = '\u{2325}';
    /// Shift (⇧).
    pub const SHIFT: char = '\u{21e7}';
    /// Control (⌃).
    pub const CONTROL: char = '\u{2303}';
    /// Return / Enter (↵).
    pub const ENTER: char = '\u{21b5}';
    /// Up arrow (↑) — palette navigation footer.
    pub const ARROW_UP: char = '\u{2191}';
    /// Down arrow (↓) — palette navigation footer.
    pub const ARROW_DOWN: char = '\u{2193}';
}

/// Visual tone of a keycap. `Neutral` is the resting state; `Active` lifts
/// the caps inside a selected palette row (accent-soft text + accent ring);
/// `Conflict` flags the offending chord on a binding clash (`BAD`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeycapTone {
    Neutral,
    Active,
    Conflict,
}

/// Keycap fill — one step below the panel so caps read as inset tiles.
pub const KEYCAP_BG: Color = BG_1;
/// Keycap label size (mono).
pub const KEYCAP_TEXT_SIZE: f32 = 11.0;
/// Inner padding of a keycap as `[vertical, horizontal]`.
pub const KEYCAP_PADDING: [f32; 2] = [2.0, 6.0];
/// Gap between adjacent keycaps in a chord row.
pub const KEYCAP_GAP: f32 = 4.0;

/// Container style for a single keycap: `BG_1` fill, hairline outline, and a
/// small radius — the `kbd` tile look from the design. `tone` recolours the
/// text and border to match the surrounding row state.
///
/// Note: the design calls for a bottom-weighted (2 px) lower border to mimic
/// a physical key. iced 0.14's `Border` carries a single uniform width, so
/// the cap uses a uniform 1 px outline; the mono font + inset fill still read
/// unmistakably as a keycap.
pub fn keycap_style(tone: KeycapTone) -> impl Fn(&Theme) -> container::Style {
    move |_theme| {
        let (text_color, border_color) = match tone {
            KeycapTone::Neutral => (TEXT_2, LINE),
            KeycapTone::Active => (ACCENT_SOFT, ACCENT_LINE),
            KeycapTone::Conflict => (BAD, BAD),
        };
        container::Style {
            text_color: Some(text_color),
            background: Some(iced::Background::Color(KEYCAP_BG)),
            border: iced::Border {
                color: border_color,
                width: 1.0,
                radius: RADIUS_XS.into(),
            },
            ..Default::default()
        }
    }
}

/// One keycap tile: a mono-font label inside an outlined cap. `label` is the
/// rendered glyph(s) — a single key (`"M"`), a modifier glyph (`kbd::CMD`),
/// or a short name (`"Esc"`).
pub fn keycap<'a, Message: 'a>(label: &str, tone: KeycapTone) -> Container<'a, Message> {
    container(
        text(label.to_string())
            .font(MONO_FONT)
            .size(KEYCAP_TEXT_SIZE)
            // Advanced shaping so the modifier glyphs (⌘ ⌥ ⇧ ↵) resolve
            // through font fallback rather than rendering as tofu.
            .shaping(Shaping::Advanced),
    )
    .padding(KEYCAP_PADDING)
    .style(keycap_style(tone))
}

/// A chord rendered as a row of keycaps — e.g. `&["⌘", "⇧", "M"]`. Caps lay
/// out left-to-right with `KEYCAP_GAP` spacing and share one `tone`.
pub fn keycap_row<'a, Message: 'a>(labels: &[&str], tone: KeycapTone) -> Row<'a, Message> {
    let mut row = Row::new()
        .spacing(KEYCAP_GAP)
        .align_y(iced::alignment::Vertical::Center);
    for label in labels {
        row = row.push(keycap(label, tone));
    }
    row
}

/// Active-row wash for the palette's selected result and the Preferences
/// nav-rail's current item: `ACCENT_DIM` fill with an `ACCENT_LINE` inset
/// ring (iced paints the 1 px border inside the bounds, matching the
/// prototype's `box-shadow: inset 0 0 0 1px`). Same selection language as
/// `card_selected`.
pub fn active_row_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(iced::Background::Color(ACCENT_DIM)),
        border: iced::Border {
            color: ACCENT_LINE,
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

/// Small "edited" pill shown on a binding row whose chord diverges from its
/// default — warm amber wash + border. Reuses the `WARM` tokens (and the
/// lane-inspector pill treatment) so "customised" reads the same everywhere.
pub fn edited_pill_style(theme: &Theme) -> container::Style {
    editing_pill_warm_style(theme)
}

/// Conflict outline for a binding row whose captured chord collides with an
/// existing binding: a `BAD` ring with no fill, pairing with the conflict
/// banner that names the current owner.
pub fn conflict_ring_style(_theme: &Theme) -> container::Style {
    container::Style {
        border: iced::Border {
            color: BAD,
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        ..Default::default()
    }
}
