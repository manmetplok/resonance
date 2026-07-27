//! Reference waveform overview Canvas: peaks + playhead + marker ticks +
//! click-to-scrub. A live visual per the view performance rules.

use std::cell::Cell;

use iced::widget::canvas;
use iced::{mouse, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_audio::types::ReferenceId;

use crate::message::*;
use crate::reference::{ReferenceEntry, ReferenceMarkerState, ReferenceMessage};
use crate::theme;

/// The waveform overview Canvas (peaks + playhead + marker ticks +
/// click-to-scrub) at a fixed height.
pub(super) fn waveform(entry: &ReferenceEntry) -> Element<'_, Message> {
    canvas(ReferenceWaveform {
        peaks: &entry.waveform_peaks,
        position_samples: entry.position_samples,
        length_samples: entry.length_samples,
        markers: &entry.markers,
        ref_id: entry.id,
    })
    .width(Length::Fill)
    .height(Length::Fixed(72.0))
    .into()
}

// ---------------------------------------------------------------------------
// Waveform Canvas — the reference's downsampled overview with a playhead, the
// comparison-marker ticks, and click-to-scrub. A live visual per the view
// performance rules: a `canvas::Cache` keeps the geometry across hover /
// resize redraws and only re-rasterises when the inputs actually change.
// ---------------------------------------------------------------------------

struct ReferenceWaveform<'a> {
    peaks: &'a [(f32, f32)],
    position_samples: u64,
    length_samples: u64,
    markers: &'a [ReferenceMarkerState],
    ref_id: ReferenceId,
}

#[derive(Default)]
struct WaveformState {
    cache: canvas::Cache,
    fingerprint: Cell<u64>,
}

impl ReferenceWaveform<'_> {
    /// Fraction `[0, 1]` of a sample position along the overview. Returns
    /// `0` when the length is unknown so nothing is drawn off-canvas.
    fn fraction(&self, sample: u64) -> f32 {
        if self.length_samples == 0 {
            0.0
        } else {
            (sample as f32 / self.length_samples as f32).clamp(0.0, 1.0)
        }
    }

    /// A cheap order-sensitive hash of everything that affects the drawn
    /// pixels, so the cache invalidates on a real change but survives a
    /// pure hover / resize repaint.
    fn fingerprint(&self) -> u64 {
        let mut h: u64 = 1469598103934665603; // FNV-1a offset basis
        let mut mix = |v: u64| {
            h ^= v;
            h = h.wrapping_mul(1099511628211);
        };
        mix(self.peaks.len() as u64);
        mix(self.position_samples);
        mix(self.length_samples);
        mix(self.ref_id.0 as u64);
        for mk in self.markers {
            mix(mk.position_samples);
        }
        h
    }
}

impl canvas::Program<Message> for ReferenceWaveform<'_> {
    type State = WaveformState;

    fn update(
        &self,
        _state: &mut Self::State,
        event: &iced::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        // Click anywhere on the overview scrubs the reference cursor to the
        // matching sample. Needs a known length to map x → samples.
        if self.length_samples == 0 {
            return None;
        }
        if let iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
            if let Some(pos) = cursor.position_in(bounds) {
                let fraction = (pos.x / bounds.width).clamp(0.0, 1.0);
                let position_samples = (fraction * self.length_samples as f32) as u64;
                return Some(
                    canvas::Action::publish(Message::Reference(ReferenceMessage::Scrub {
                        ref_id: self.ref_id,
                        position_samples,
                    }))
                    .and_capture(),
                );
            }
        }
        None
    }

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let fp = self.fingerprint();
        if state.fingerprint.get() != fp {
            state.cache.clear();
            state.fingerprint.set(fp);
        }
        let geometry = state.cache.draw(renderer, bounds.size(), |frame| {
            let w = bounds.width;
            let h = bounds.height;
            let mid = h / 2.0;

            // Backdrop.
            frame.fill_rectangle(Point::ORIGIN, Size::new(w, h), theme::BG_3);
            // Zero-amplitude centre line.
            frame.fill_rectangle(Point::new(0.0, mid - 0.5), Size::new(w, 1.0), theme::LINE_2);

            // Peak columns over the mono (min, max) overview.
            if !self.peaks.is_empty() {
                let col_w = w / self.peaks.len() as f32;
                let bar_w = col_w.max(1.0);
                for (i, (min, max)) in self.peaks.iter().enumerate() {
                    let x = i as f32 * col_w;
                    let top = mid - max.clamp(-1.0, 1.0) * mid;
                    let bottom = mid - min.clamp(-1.0, 1.0) * mid;
                    let bar_h = (bottom - top).max(1.0);
                    frame.fill_rectangle(
                        Point::new(x, top),
                        Size::new(bar_w, bar_h),
                        theme::TEXT_2,
                    );
                }
            }

            // Marker ticks — a thin amber line per comparison marker.
            for mk in self.markers {
                let x = self.fraction(mk.position_samples) * w;
                frame.fill_rectangle(Point::new(x, 0.0), Size::new(1.0, h), theme::WARM_LINE);
            }

            // Playhead — the reference's own cursor.
            let px = self.fraction(self.position_samples) * w;
            frame.fill_rectangle(Point::new(px - 0.75, 0.0), Size::new(1.5, h), theme::ACCENT);
        });
        vec![geometry]
    }
}
