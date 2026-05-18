//! Small "MOTIF · N notes" preview card with scattered dashes — read-only,
//! purely for at-a-glance density feedback. Includes the canvas program.

use iced::widget::{canvas, column, row, text, Space};
use iced::{alignment, Element, Length};

use resonance_music_theory::MotifSource;

use crate::compose::SectionDefinitionState;
use crate::message::*;
use crate::theme;

/// Small "MOTIF · N notes" preview card with scattered dashes — read-only,
/// purely for at-a-glance density feedback.
pub(super) fn motif_preview_card<'a>(
    definition: &'a SectionDefinitionState,
) -> Element<'a, Message> {
    let note_count = match &definition.motif_source {
        MotifSource::Manual { notes, .. } => notes.len(),
        MotifSource::Generated(_) => {
            let n = definition.motif_source.params().motif_len;
            if n == 0 {
                4
            } else {
                n as usize
            }
        }
    };

    let header = row![
        text("MOTIF")
            .size(10)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_3),
        Space::with_width(Length::Fill),
        text(format!("{note_count} notes"))
            .size(10)
            .font(theme::MONO_FONT)
            .color(theme::TEXT_3),
    ]
    .align_y(alignment::Vertical::Center);

    // Scattered note dashes — scaled rectangles arranged in a grid via
    // a tiny canvas; deterministic on `seed` so it doesn't churn.
    let preview_canvas = canvas(MotifPreviewCanvas {
        seed: definition.motif_source.params().seed,
        note_count,
    })
    .width(Length::Fill)
    .height(Length::Fixed(56.0));

    iced::widget::container(column![header, Space::with_height(6), preview_canvas].spacing(0))
        .padding([10, 12])
        .width(Length::Fill)
        .style(|_theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_LG.into(),
            },
            ..Default::default()
        })
        .into()
}

struct MotifPreviewCanvas {
    seed: u64,
    note_count: usize,
}

impl<Message> iced::widget::canvas::Program<Message> for MotifPreviewCanvas {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &iced::Renderer,
        _theme: &iced::Theme,
        bounds: iced::Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        use iced::widget::canvas::{Frame, Path};
        use iced::{Point, Size};

        let mut frame = Frame::new(renderer, bounds.size());
        if self.note_count == 0 {
            return vec![frame.into_geometry()];
        }
        let n = self.note_count.clamp(1, 24);
        let cell_w = bounds.width / n as f32;
        let row_count = 5;
        let row_h = bounds.height / row_count as f32;
        let mut acc: u64 = self.seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        for i in 0..n {
            // simple deterministic shuffle so preview varies with seed
            acc = acc
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let row = (acc >> 13) as usize % row_count;
            let dash_w = (cell_w - 4.0).max(2.0);
            let dash_h = 4.0;
            let x = i as f32 * cell_w + 2.0;
            let y = row as f32 * row_h + (row_h - dash_h) / 2.0;
            let path = Path::rounded_rectangle(
                Point::new(x, y),
                Size::new(dash_w, dash_h),
                2.0.into(),
            );
            frame.fill(&path, theme::ACCENT_SOFT);
        }
        vec![frame.into_geometry()]
    }
}
