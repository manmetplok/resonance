//! Segmented control — a strip of segments where exactly one is "on".
//!
//! The canonical version of the control the drums and wavetable editors
//! each carried privately (ba doc #275, LOW finding; promoted by ba todo
//! #1334). Those two copies differed only in dead code: wavetable's took
//! a `led_for_active` flag whose body was a `let _ = pos;` no-op, drums'
//! took the same flag under an underscore name and ignored it. Neither
//! ever drew an LED, so the flag is gone rather than promoted — the
//! shared control has no argument that does nothing.
//!
//! The granular delay's scheduler column is the same control laid out
//! vertically out of [`super::chip`] pills; [`SegmentedStyle::COMPACT`]
//! plus `vertical` covers it, so that third idiom folds in here too.

use super::chip::{chip_styled, Chip, ChipStyle};
use crate::theme::lavender as theme;

/// Frame, direction and segment look of a segmented control.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentedStyle {
    /// How each segment is drawn.
    pub segment: ChipStyle,
    /// Stack the segments in a column instead of a row.
    pub vertical: bool,
    /// Draw the enclosing pill (fill + outline) around the segments.
    pub framed: bool,
    /// Corner radius of the enclosing pill.
    pub frame_radius: f32,
    /// Padding between the enclosing pill and the segments.
    pub frame_margin: i8,
    /// Gap between neighbouring segments.
    pub gap: egui::Vec2,
}

impl SegmentedStyle {
    /// The framed 22 px pill the drums and wavetable editors use.
    pub const LAVENDER: Self = Self {
        segment: ChipStyle::SEGMENT,
        vertical: false,
        framed: true,
        frame_radius: 7.0,
        frame_margin: 3,
        gap: egui::vec2(2.0, 0.0),
    };

    /// Unframed compact chips, for dense strips (the granular delay's
    /// scheduler and route pickers).
    pub const COMPACT: Self = Self {
        segment: ChipStyle::COMPACT,
        vertical: false,
        framed: false,
        frame_radius: 7.0,
        frame_margin: 0,
        gap: egui::vec2(2.0, 2.0),
    };

    /// Lay the segments out in a column.
    pub fn vertical(mut self, vertical: bool) -> Self {
        self.vertical = vertical;
        self
    }
}

impl Default for SegmentedStyle {
    fn default() -> Self {
        Self::LAVENDER
    }
}

/// Draw a segmented selector in the default style. Returns the index
/// that was clicked, if any — including a click on the current
/// selection, exactly as the editors' private copies did.
pub fn segmented(ui: &mut egui::Ui, labels: &[&str], selected: usize) -> Option<usize> {
    segmented_styled(ui, labels, selected, &SegmentedStyle::LAVENDER)
}

/// Draw a segmented selector with an explicit style.
pub fn segmented_styled(
    ui: &mut egui::Ui,
    labels: &[&str],
    selected: usize,
    style: &SegmentedStyle,
) -> Option<usize> {
    let mut clicked = None;
    let mut segments = |ui: &mut egui::Ui| {
        ui.spacing_mut().item_spacing = style.gap;
        let draw = |ui: &mut egui::Ui| {
            for (i, label) in labels.iter().enumerate() {
                let chip = Chip::new(label, i == selected).style(style.segment);
                if chip_styled(ui, &chip) {
                    clicked = Some(i);
                }
            }
        };
        if style.vertical {
            ui.vertical(draw);
        } else {
            ui.horizontal(draw);
        }
    };

    if style.framed {
        let frame = egui::Frame::default()
            .fill(theme::BG_2)
            .stroke(egui::Stroke::new(1.0, theme::LINE_2))
            .corner_radius(style.frame_radius)
            .inner_margin(egui::Margin::same(style.frame_margin));
        frame.show(ui, &mut segments);
    } else {
        segments(ui);
    }

    clicked
}
