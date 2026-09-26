//! Panel container: `PanelState` classification and the top-level `view`
//! entry point that routes to the right body.

use iced::widget::{button, column, container, row, text, Space};
use iced::{alignment, Element, Length};

use crate::message::*;
use crate::reference::ReferenceState;
use crate::theme::{self};

use super::bodies;

/// Which body the panel container routes to, derived from
/// [`ReferenceState`]. The populated A/B controls for *loaded* references
/// are filled in by a later todo; missing/errored entries already render
/// inline within the populated body so the rest of the panel stays usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelState {
    /// No references listed (a load is listed as analysing from the
    /// moment it is sent).
    Empty,
    /// At least one reference is mid-analysis.
    Analyzing,
    /// One or more references exist (loaded, missing, or errored).
    Populated,
    /// A load failed with no entry to attach the notice to — surfaced from
    /// `last_error` while the slot is otherwise empty.
    Error,
}

fn classify(state: &ReferenceState) -> PanelState {
    // A load failure that created no entry takes the whole panel: there is
    // nothing else to show it against. A missing/errored *entry*, by
    // contrast, renders inline in `Populated` so any loaded references
    // alongside it stay usable (design doc #198).
    if state.entries.is_empty() {
        if state.last_error.is_some() {
            return PanelState::Error;
        }
        return PanelState::Empty;
    }
    if state
        .entries
        .iter()
        .any(|e| matches!(e.status, crate::reference::ReferenceStatus::Analyzing(_)))
    {
        return PanelState::Analyzing;
    }
    PanelState::Populated
}

pub(in crate::view::mixer) fn view(r: &crate::Resonance) -> Element<'_, Message> {
    let body: Element<'_, Message> = match classify(&r.reference) {
        PanelState::Empty => bodies::empty_body(),
        PanelState::Analyzing => bodies::analyzing_body(&r.reference),
        PanelState::Populated => bodies::populated_body(&r.reference),
        PanelState::Error => bodies::error_body(&r.reference),
    };

    let content = column![header(), Space::new().height(18), body].spacing(0);

    container(content)
        .width(Length::Fixed(theme::REFERENCE_PANEL_WIDTH))
        .height(Length::Fill)
        .padding(theme::RAIL_PADDING)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            ..Default::default()
        })
        .into()
}

/// Panel title row: label + a close (×) button that re-toggles the rail.
fn header() -> Element<'static, Message> {
    let close = button(text("\u{00d7}").size(15).color(theme::TEXT_3))
        .on_press(Message::Ui(UiMessage::ToggleReferencePanel))
        .padding([1, 7])
        .style(|_theme, status| theme::small_button_style(status));

    row![
        column![
            text("REFERENCE & A/B")
                .size(10)
                .font(theme::UI_FONT_SEMIBOLD)
                .color(theme::TEXT_3),
            Space::new().height(2),
            text("Monitor only — not in exports")
                .size(11)
                .color(theme::TEXT_2),
        ]
        .spacing(0),
        Space::new().width(Length::Fill),
        close,
    ]
    .align_y(alignment::Vertical::Center)
    .into()
}
