//! Pill-shaped chip button — the shared kit's discrete toggle.
//!
//! This is the canonical chip: articulation pickers, filter-type
//! pickers, boolean pills (SYNC / PER-BEAT / SHIMMER) and the segments
//! of [`super::segmented`] are all drawn by [`chip_styled`].
//!
//! It is the superset of the three chips the plugin fleet grew
//! independently (ba doc #275, LOW finding; promoted by ba todo #1334):
//!
//! - the drums and wavetable editors' `chip_button` — an `egui::Button`
//!   pill, 22 px tall, 11 pt label, accent-tinted when active. The two
//!   copies differed only in a doc comment. Reproduced by
//!   [`ChipStyle::LAVENDER`], which is what the argument-for-argument
//!   drop-in [`chip_button`] uses;
//! - the granular delay's `param_chip` / `segment_chip` — a
//!   painter-drawn pill, 16 px tall, 8.5 pt upper-case label, with a
//!   hover state and a greyed disabled state. Reproduced by
//!   [`ChipStyle::COMPACT`].
//!
//! Two differences had to be reconciled, both resolved in favour of the
//! richer behaviour so a migrating editor gains rather than loses:
//!
//! - **hover feedback.** The button-based chips had none (an
//!   `egui::Button` with an explicit `fill` and `stroke` overrides
//!   egui's hover visuals), the painter-drawn ones did. Every chip now
//!   answers hover, through [`ChipPalette::hover`]; an editor that
//!   genuinely wants a dead chip sets `hover` equal to `idle`.
//! - **disabled state.** Only the granular chips had one. It is part of
//!   every style now; `enabled: true` chips never reach it.
//!
//! Geometry is painter-driven rather than `egui::Button`-driven so a
//! chip measures the same in every editor. [`ChipStyle::LAVENDER`]
//! therefore pads by egui's own default `button_padding.x` (4 px), which
//! keeps the pill the exact width the button-based forks laid out.

use egui::Color32;

use crate::theme::lavender as theme;

/// The three colours one chip state paints with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChipColors {
    /// Pill background.
    pub fill: Color32,
    /// Label colour.
    pub text: Color32,
    /// Outline, or `None` for a border-less segment.
    pub stroke: Option<Color32>,
}

impl ChipColors {
    /// A filled chip with an outline.
    pub const fn bordered(fill: Color32, text: Color32, stroke: Color32) -> Self {
        Self {
            fill,
            text,
            stroke: Some(stroke),
        }
    }

    /// A filled chip with no outline (segmented-control segments).
    pub const fn plain(fill: Color32, text: Color32) -> Self {
        Self {
            fill,
            text,
            stroke: None,
        }
    }
}

/// Colours of every chip state. `active` wins over `hover`, and
/// `disabled` wins over both.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChipPalette {
    /// Idle: not active, not hovered.
    pub idle: ChipColors,
    /// Pointer is over the chip and it is neither active nor disabled.
    pub hover: ChipColors,
    /// The chip is on / selected.
    pub active: ChipColors,
    /// The chip is greyed and ignores input.
    pub disabled: ChipColors,
}

/// Geometry, type scale and palette of one chip.
///
/// A plugin that needs a different chip size picks (or derives) a style
/// — it does not fork the widget.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChipStyle {
    /// Minimum pill height, px. A taller label grows it.
    pub height: f32,
    /// Padding either side of the label, px.
    pub pad_x: f32,
    /// Label font size (proportional).
    pub font_size: f32,
    /// Corner radius, px.
    pub radius: f32,
    /// Upper-case the label before measuring and drawing it.
    pub uppercase: bool,
    /// Colours per state.
    pub palette: ChipPalette,
}

/// Accent tint behind an active [`ChipStyle::LAVENDER`] chip: the
/// lavender accent at 9% alpha, exactly as the drums and wavetable forks
/// spelled it.
const LAVENDER_ACTIVE_FILL: Color32 = Color32::from_rgba_unmultiplied_const(0x8b, 0x6d, 0xff, 0x18);

impl ChipStyle {
    /// The 22 px pill the drums and wavetable editors use.
    pub const LAVENDER: Self = Self {
        height: 22.0,
        // egui's default `button_padding.x`, so the painter-drawn pill
        // is the width the `egui::Button` forks laid out.
        pad_x: 4.0,
        font_size: 11.0,
        radius: theme::RADIUS_CHIP,
        uppercase: false,
        palette: ChipPalette {
            idle: ChipColors::bordered(theme::BG_1, theme::TEXT_2, theme::LINE_2),
            hover: ChipColors::bordered(theme::BG_3, theme::TEXT_1, theme::LINE),
            active: ChipColors::bordered(LAVENDER_ACTIVE_FILL, theme::TEXT_1, theme::ACCENT),
            disabled: ChipColors::bordered(theme::BG_1, theme::TEXT_4, theme::LINE_2),
        },
    };

    /// The 16 px upper-case pill the granular delay uses, for dense
    /// control strips.
    pub const COMPACT: Self = Self {
        height: 16.0,
        pad_x: 7.0,
        font_size: 8.5,
        radius: theme::RADIUS_CHIP,
        uppercase: true,
        palette: ChipPalette {
            idle: ChipColors::bordered(theme::BG_1, theme::TEXT_3, theme::LINE_2),
            hover: ChipColors::bordered(theme::BG_3, theme::TEXT_2, theme::LINE),
            active: ChipColors::bordered(theme::ACCENT_DIM, theme::ACCENT_SOFT, theme::ACCENT),
            disabled: ChipColors::bordered(theme::BG_1, theme::TEXT_4, theme::LINE_2),
        },
    };

    /// The border-less segment of a [`super::segmented`] control: same
    /// height as [`Self::LAVENDER`], no outline, surface fills.
    pub const SEGMENT: Self = Self {
        height: 22.0,
        // Same reasoning as `LAVENDER`: egui's default button padding,
        // so a promoted segmented control measures exactly as wide as
        // the button-based forks it replaces.
        pad_x: 4.0,
        font_size: 11.5,
        radius: 5.0,
        uppercase: false,
        palette: ChipPalette {
            idle: ChipColors::plain(theme::BG_2, theme::TEXT_2),
            hover: ChipColors::plain(theme::BG_2, theme::TEXT_1),
            active: ChipColors::plain(theme::BG_3, theme::TEXT_1),
            disabled: ChipColors::plain(theme::BG_2, theme::TEXT_4),
        },
    };

    /// The colours this style paints with, given the chip's state.
    ///
    /// Precedence — disabled, then active, then hover — is pinned here
    /// so every chip resolves its state the same way.
    pub fn colors(&self, active: bool, enabled: bool, hovered: bool) -> ChipColors {
        if !enabled {
            self.palette.disabled
        } else if active {
            self.palette.active
        } else if hovered {
            self.palette.hover
        } else {
            self.palette.idle
        }
    }

    /// The pill size for a label of `label_size` (the measured galley).
    ///
    /// The label never overflows: the pill is at least [`Self::height`]
    /// tall and always [`Self::pad_x`] wider than the text on each side.
    pub fn size_for(&self, label_size: egui::Vec2) -> egui::Vec2 {
        egui::vec2(
            label_size.x + self.pad_x * 2.0,
            self.height.max(label_size.y + 2.0),
        )
    }
}

impl Default for ChipStyle {
    fn default() -> Self {
        Self::LAVENDER
    }
}

/// One chip: a label, an on/off state, and how to draw it.
#[derive(Debug, Clone, Copy)]
pub struct Chip<'a> {
    /// Text inside the pill.
    pub label: &'a str,
    /// Whether the chip reads as "on".
    pub active: bool,
    /// A disabled chip renders greyed and ignores clicks.
    pub enabled: bool,
    /// Geometry, type scale and palette.
    pub style: ChipStyle,
}

impl<'a> Chip<'a> {
    /// An enabled chip in the default [`ChipStyle::LAVENDER`] style.
    pub fn new(label: &'a str, active: bool) -> Self {
        Self {
            label,
            active,
            enabled: true,
            style: ChipStyle::LAVENDER,
        }
    }

    /// Grey the chip out and stop it answering clicks.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Draw with a non-default style.
    pub fn style(mut self, style: ChipStyle) -> Self {
        self.style = style;
        self
    }
}

/// Draw a chip in the default style; returns `true` on click.
///
/// Argument-for-argument the `chip_button` the drums and wavetable
/// editors carry privately, so migrating is an import change.
pub fn chip_button(ui: &mut egui::Ui, label: &str, active: bool) -> bool {
    chip_styled(ui, &Chip::new(label, active))
}

/// Draw a configured chip; returns `true` when it was clicked (never
/// when disabled).
pub fn chip_styled(ui: &mut egui::Ui, chip: &Chip<'_>) -> bool {
    let style = chip.style;
    let text = if style.uppercase {
        chip.label.to_uppercase()
    } else {
        chip.label.to_owned()
    };
    let font = egui::FontId::proportional(style.font_size);
    // Laid out with the placeholder colour so the state colour resolved
    // below is what actually paints (`Painter::galley` substitutes it).
    let galley = ui
        .painter()
        .layout_no_wrap(text, font, Color32::PLACEHOLDER);

    let sense = if chip.enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(style.size_for(galley.size()), sense);

    if ui.is_rect_visible(rect) {
        let colors = style.colors(chip.active, chip.enabled, response.hovered());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, style.radius, colors.fill);
        if let Some(stroke) = colors.stroke {
            painter.rect_stroke(
                rect,
                style.radius,
                egui::Stroke::new(1.0, stroke),
                egui::StrokeKind::Inside,
            );
        }
        painter.galley(
            rect.center() - galley.size() * 0.5,
            galley,
            colors.text,
        );
    }

    chip.enabled && response.clicked()
}
