//! The delay's control strip: data-driven group cards that wrap to the
//! window (code review PUX-02/-03/-05).
//!
//! The strip used to be one `ui.horizontal` of 22 range-mapped knobs —
//! about 1710 px of row in a 1200 px window, so Gate and Duck were cut
//! off at the default size and half the editor was gone at the 900 px
//! minimum. Every control was a knob, including the three bools and the
//! two small choices, and a knob on a stepped param could not be
//! dragged at all (Sync, Freeze and Gate could not be switched). And the
//! knobs mapped linearly, ignoring the skew `params.rs` declares on
//! Time, Hi/Lo Cut, Mod Rate and Duck Release.
//!
//! Now [`GROUPS`] lists every parameter exactly once
//! (`tests/editor_layout.rs` holds it to that) as the control its type
//! calls for — a knob for a continuous value, a chip for a switch, a
//! segmented control for a two- or three-way choice, a combo for a note
//! division — each bound through `resonance_plugin::editor_widgets`,
//! which reads range, skew, default and readout off the param and
//! announces every edit to the host. [`pack_rows`] fills rows greedily
//! from each card's known width, so the strip wraps instead of running
//! off the window, and [`strip_height`] tells the app how tall the
//! bottom panel has to be at the current width.

use plugin_gui_core::egui;
use plugin_gui_core::widgets::{ChipStyle, KnobStyle, SegmentedStyle};
use resonance_plugin::editor_widgets::{self, ParamKnob};
use resonance_plugin::{BoolParam, FloatParam, IntParam};

use crate::params::DelayParams;

use super::theme;

/// A float param, read off the params.
pub type FloatAt = fn(&DelayParams) -> &FloatParam;
/// A bool param, read off the params.
pub type BoolAt = fn(&DelayParams) -> &BoolParam;
/// An int param, read off the params.
pub type IntAt = fn(&DelayParams) -> &IntParam;

/// One switch in a [`Item::Switches`] column.
pub enum Switch {
    /// A chip toggle.
    Toggle(BoolAt, &'static str),
    /// A vertical one-of-N segmented control, labelled from the param's
    /// own choice table (`IntParam::with_choices`) — FU-P2a: the labels
    /// used to be a second argument here, duplicating `params.rs`.
    Segments(IntAt),
    /// A note-division combo, labelled from the param's own choice table.
    Division(IntAt),
}

/// One column of a group card.
pub enum Item {
    /// A knob, captioned for this card.
    Knob(FloatAt, &'static str),
    /// A column of switches, stacked.
    Switches(&'static [Switch]),
}

/// One captioned card.
pub struct Group {
    pub caption: &'static str,
    pub items: &'static [Item],
}

/// Every card, in the order the signal meets them. Covers every
/// declared parameter exactly once (`tests/editor_layout.rs`).
pub const GROUPS: &[Group] = &[
    Group {
        caption: "TIME",
        items: &[
            Item::Switches(&[
                Switch::Toggle(|p| &p.sync, "Sync"),
                Switch::Division(|p| &p.division),
            ]),
            Item::Knob(|p| &p.time_ms, "Time"),
        ],
    },
    Group {
        caption: "ECHO",
        items: &[
            Item::Knob(|p| &p.feedback, "Feedback"),
            Item::Knob(|p| &p.mix, "Mix"),
            Item::Switches(&[Switch::Toggle(|p| &p.freeze, "Freeze")]),
        ],
    },
    Group {
        caption: "CHARACTER",
        items: &[
            Item::Switches(&[Switch::Segments(|p| &p.character)]),
            Item::Switches(&[Switch::Segments(|p| &p.routing)]),
            Item::Knob(|p| &p.stereo_offset, "Offset"),
        ],
    },
    Group {
        caption: "TONE",
        items: &[
            Item::Knob(|p| &p.hi_cut, "Hi Cut"),
            Item::Knob(|p| &p.lo_cut, "Lo Cut"),
            Item::Knob(|p| &p.drive, "Drive"),
        ],
    },
    Group {
        caption: "MOD",
        items: &[
            Item::Knob(|p| &p.mod_rate, "Rate"),
            Item::Knob(|p| &p.mod_depth, "Depth"),
        ],
    },
    Group {
        caption: "GATE",
        items: &[
            Item::Switches(&[
                Switch::Toggle(|p| &p.gate_on, "Gate"),
                Switch::Division(|p| &p.gate_rate),
            ]),
            Item::Knob(|p| &p.gate_width, "Width"),
            Item::Knob(|p| &p.gate_shape, "Shape"),
            Item::Knob(|p| &p.gate_depth, "Depth"),
        ],
    },
    Group {
        caption: "DUCK",
        items: &[
            Item::Knob(|p| &p.duck_amount, "Amount"),
            Item::Knob(|p| &p.duck_threshold, "Threshold"),
            Item::Knob(|p| &p.duck_release, "Release"),
        ],
    },
];

/// The knob cell every delay knob draws in.
const KNOB_STYLE: KnobStyle = KnobStyle::CAPTIONED;
/// Width of a switch column.
pub const SWITCH_W: f32 = 84.0;
/// Width of a division combo inside a switch column.
const COMBO_W: f32 = 64.0;
/// Gap between the columns of a card.
const ITEM_GAP: f32 = 6.0;
/// Card padding, each side.
const CARD_PAD_X: f32 = 10.0;
const CARD_PAD_Y: f32 = 8.0;
/// Card caption row.
const CAPTION_H: f32 = 14.0;
/// Gap between cards, both ways.
pub const CARD_GAP: f32 = 8.0;
/// Space around the strip, each side.
pub const STRIP_PAD: f32 = 8.0;

impl Item {
    fn width(&self) -> f32 {
        match self {
            Item::Knob(..) => KNOB_STYLE.cell().x,
            Item::Switches(_) => SWITCH_W,
        }
    }
}

impl Group {
    /// The card's outer width, frame included.
    pub fn width(&self) -> f32 {
        let content: f32 = self.items.iter().map(Item::width).sum::<f32>()
            + ITEM_GAP * (self.items.len().saturating_sub(1)) as f32;
        content + 2.0 * CARD_PAD_X
    }
}

/// The card's outer height, frame included (every card is one row of
/// knob cells tall).
pub fn card_height() -> f32 {
    CAPTION_H + 4.0 + KNOB_STYLE.cell().y + 2.0 * CARD_PAD_Y
}

/// The groups, packed greedily into rows no wider than `width`: indices
/// into [`GROUPS`]. A card wider than `width` gets a row of its own.
pub fn pack_rows(width: f32) -> Vec<Vec<usize>> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut used = 0.0;
    for (i, group) in GROUPS.iter().enumerate() {
        let w = group.width();
        match rows.last_mut() {
            Some(row) if used + CARD_GAP + w <= width => {
                row.push(i);
                used += CARD_GAP + w;
            }
            _ => {
                rows.push(vec![i]);
                used = w;
            }
        }
    }
    rows
}

/// Height of the bottom strip at window `width`.
pub fn strip_height(width: f32) -> f32 {
    let rows = pack_rows(width - 2.0 * STRIP_PAD).len().max(1) as f32;
    rows * card_height() + (rows - 1.0) * CARD_GAP + 2.0 * STRIP_PAD
}

/// Draw the strip into the bottom panel.
pub fn draw(ui: &mut egui::Ui, params: &DelayParams) {
    let rows = pack_rows(ui.available_width() - 2.0 * STRIP_PAD);
    ui.add_space(STRIP_PAD);
    ui.spacing_mut().item_spacing = egui::vec2(CARD_GAP, CARD_GAP);
    for row in rows {
        ui.horizontal(|ui| {
            ui.add_space(STRIP_PAD - CARD_GAP);
            for i in row {
                draw_group(ui, params, &GROUPS[i]);
            }
        });
    }
}

fn draw_group(ui: &mut egui::Ui, params: &DelayParams, group: &Group) {
    let frame = egui::Frame::default()
        .fill(theme::BG_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE_2))
        .corner_radius(theme::RADIUS_PANEL)
        .inner_margin(egui::Margin::symmetric(CARD_PAD_X as i8, CARD_PAD_Y as i8));
    let inner_w = group.width() - 2.0 * CARD_PAD_X;
    frame.show(ui, |ui| {
        ui.set_width(inner_w);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(ITEM_GAP, 4.0);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(inner_w, CAPTION_H), egui::Sense::hover());
            ui.painter().text(
                rect.left_center(),
                egui::Align2::LEFT_CENTER,
                group.caption,
                egui::FontId::proportional(10.5),
                theme::TEXT_3,
            );
            ui.horizontal_top(|ui| {
                for item in group.items {
                    draw_item(ui, params, item);
                }
            });
        });
    });
}

fn draw_item(ui: &mut egui::Ui, params: &DelayParams, item: &Item) {
    match item {
        Item::Knob(at, label) => {
            editor_widgets::param_knob(ui, ParamKnob::new(at(params), label).style(KNOB_STYLE));
        }
        Item::Switches(switches) => {
            let size = egui::vec2(SWITCH_W, KNOB_STYLE.cell().y);
            ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Center), |ui| {
                ui.set_width(SWITCH_W);
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 6.0);
                ui.add_space(4.0);
                for switch in *switches {
                    draw_switch(ui, params, switch);
                }
            });
        }
    }
}

fn draw_switch(ui: &mut egui::Ui, params: &DelayParams, switch: &Switch) {
    match switch {
        Switch::Toggle(at, label) => {
            editor_widgets::param_chip(ui, at(params), label, true, ChipStyle::COMPACT);
        }
        Switch::Segments(at) => {
            let style = SegmentedStyle::COMPACT.vertical(true);
            editor_widgets::choice_segmented(ui, at(params), &style);
        }
        Switch::Division(at) => {
            editor_widgets::int_choice(ui, at(params), COMBO_W);
        }
    }
}
