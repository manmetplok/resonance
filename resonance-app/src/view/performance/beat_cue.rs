//! Performance mode — beat ring + count-in countdown cue (epic #11,
//! todo #310, design doc #151 / arch doc #152).
//!
//! The third column of the centre stage. It renders, on a single `Canvas`
//! with a cached static layer:
//!
//! * **Beat ring** — N pips for the meter (6 for 6/8). The current beat is
//!   lit [`theme::TEXT_1`], past beats recede to [`theme::TEXT_3`], and
//!   upcoming beats stay a neutral dark fill. A **WARM shrinking arc** wraps
//!   the ring: it spans the fraction of the current chord still to play, so
//!   an upcoming chord change is anticipated rather than reacted to.
//! * **Count-in cue** — during the transport pre-count
//!   (`transport.precount_bars`) a big [`theme::GOOD`]-mint countdown is
//!   drawn over the column while the primed first chord shows (dimmed) on
//!   the stage behind it, so the first chord is on screen before audio
//!   starts. The primed position comes from the chord-derivation core
//!   ([`crate::engine_events::performance`]) — the view never special-cases
//!   the pre-count.
//!
//! Per the view-performance rules the geometry is cached and only re-drawn
//! when the cue's inputs change (a beat tick, a chord change, or the
//! count-in number) — never per frame — so a take never drops audio frames.
//!
//! [`BeatCueState`] is a pure value derived from the transport + the
//! [`ChordReadout`](crate::engine_events::performance::ChordReadout); it is
//! built by [`BeatCueState::derive`] and is unit-tested headlessly in
//! `tests/performance_beat_cue.rs`.

use std::cell::Cell;
use std::f32::consts::{FRAC_PI_2, TAU};

use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::widget::Canvas;
use iced::{mouse, Color, Element, Length, Point, Radians, Rectangle, Renderer, Theme};

use resonance_audio::types::TempoMap;

use crate::engine_events::performance::ChordReadout;
use crate::theme;

/// Side length of the cue canvas, in logical pixels.
pub const CUE_SIZE: f32 = 260.0;

/// Largest meter we draw individual pips for; beyond this the ring would be
/// too dense to read across a room, so the pip count is capped.
const MAX_PIPS: u32 = 12;

/// The pure, render-ready description of the beat ring + count-in cue for a
/// single transport position. Derived from the chord readout and transport
/// state by [`BeatCueState::derive`]; everything the `Canvas` needs to draw
/// (and to decide whether a redraw is required) lives here.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BeatCueState {
    /// Number of pips in the ring (the bar's beat count). 0 hides the ring.
    pub beats_per_bar: u32,
    /// 1-based current beat within the bar. 0 when the transport is parked
    /// (no beat lit).
    pub beat_in_bar: u32,
    /// Fraction `0.0..=1.0` of the current chord's span still remaining —
    /// drives the shrinking WARM arc. `None` hides the arc (no chord under
    /// the playhead, or nothing upcoming). Stepped to whole beats so the arc
    /// only changes on a beat tick, never per frame.
    pub arc_remaining: Option<f32>,
    /// During the pre-count, the remaining whole beats to show as the big
    /// mint countdown. `None` outside the count-in.
    pub count_in_beats: Option<u32>,
    /// Whether the transport is rolling — lights the current beat and shows
    /// the live ring. When false the ring rests in its neutral state.
    pub rolling: bool,
}

impl BeatCueState {
    /// Derive the cue state from a [`ChordReadout`] plus the live transport
    /// context.
    ///
    /// * `readout` — the chord-derivation result for this position. When the
    ///   readout was taken at a primed (pre-count) position
    ///   ([`ChordReadout::priming`]), the count-in countdown is shown.
    /// * `rolling` — whether the transport is playing/recording.
    /// * `tempo_map`, `sample_rate` — used only to measure the count-in
    ///   distance (playhead → primed sample) in whole beats.
    /// * `playhead`, `primed_position` — the live playhead and, during the
    ///   pre-count, the sample where the first chord will sound.
    pub fn derive(
        readout: &ChordReadout,
        rolling: bool,
        tempo_map: &TempoMap,
        sample_rate: u32,
        playhead: u64,
        primed_position: Option<u64>,
    ) -> Self {
        // Time-until-next as a fraction of the current chord's length,
        // stepped to whole beats so the arc only moves on a beat tick.
        let arc_remaining = match (readout.current.as_ref(), readout.beats_until_next) {
            (Some(cur), Some(beats_until)) if cur.duration_beats > 0 => {
                let remaining_beats = beats_until.ceil().max(0.0);
                let frac = (remaining_beats / cur.duration_beats as f64) as f32;
                Some(frac.clamp(0.0, 1.0))
            }
            _ => None,
        };

        // Count-in countdown: whole beats from the playhead up to the primed
        // first-chord sample. Only while the readout is priming.
        let count_in_beats = match primed_position {
            Some(primed) if readout.priming && primed > playhead => {
                let from = sample_to_global_beat(tempo_map, playhead, sample_rate);
                let to = sample_to_global_beat(tempo_map, primed, sample_rate);
                Some(((to - from).ceil() as u32).max(1))
            }
            _ => None,
        };

        Self {
            beats_per_bar: readout.beats_per_bar.min(MAX_PIPS),
            beat_in_bar: if rolling { readout.beat_in_bar } else { 0 },
            arc_remaining,
            count_in_beats,
            rolling,
        }
    }

    /// Stable fingerprint of everything that affects the drawn geometry.
    /// The canvas cache is cleared only when this changes — i.e. on a beat
    /// tick, a chord change, or a count-in step, never per frame.
    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.beats_per_bar.hash(&mut h);
        self.beat_in_bar.hash(&mut h);
        // Quantise the arc to whole percent so float noise can't churn it.
        self.arc_remaining
            .map(|f| (f * 100.0).round() as i32)
            .hash(&mut h);
        self.count_in_beats.hash(&mut h);
        self.rolling.hash(&mut h);
        h.finish()
    }
}

/// Absolute, fractional grid-beat position of a sample, from project bar 0.
/// Mirrors the chord-derivation core's conversion but over the public
/// [`TempoMap`] API, so the view stays in its lane (it does not reach into
/// the core's private helpers).
fn sample_to_global_beat(tempo_map: &TempoMap, sample: u64, sample_rate: u32) -> f64 {
    let (bar, frac) = tempo_map.sample_to_bar(sample, sample_rate);
    let beats_before_bar: u64 = (0..bar).map(|b| tempo_map.numerator_at_bar(b) as u64).sum();
    let num = tempo_map.numerator_at_bar(bar) as f64;
    beats_before_bar as f64 + frac * num
}

/// Build the beat-ring / count-in cue as a fixed-size `Canvas` element.
pub fn beat_cue<'a, Message: 'a>(state: BeatCueState) -> Element<'a, Message> {
    Canvas::new(BeatCue { state })
        .width(Length::Fixed(CUE_SIZE))
        .height(Length::Fixed(CUE_SIZE))
        .into()
}

/// The canvas program. Holds the immutable per-frame [`BeatCueState`]; the
/// cached geometry lives in [`CueCache`] (the `Program::State`).
struct BeatCue {
    state: BeatCueState,
}

/// Persistent canvas state: the cached geometry plus the fingerprint it was
/// drawn at, so a redraw that doesn't change the cue reuses the cache.
#[derive(Default)]
struct CueCache {
    cache: canvas::Cache,
    drawn_fp: Cell<u64>,
}

impl<Message> canvas::Program<Message> for BeatCue {
    type State = CueCache;

    fn draw(
        &self,
        state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let fp = self.state.fingerprint();
        if state.drawn_fp.get() != fp {
            state.cache.clear();
            state.drawn_fp.set(fp);
        }
        let cue = self.state;
        let geometry = state.cache.draw(renderer, bounds.size(), |frame: &mut Frame| {
            draw_cue(frame, bounds, &cue);
        });
        vec![geometry]
    }
}

/// Pull a colour down to a faint version of itself (alpha-scaled).
fn faded(color: Color, alpha: f32) -> Color {
    Color { a: alpha, ..color }
}

/// Draw the whole cue into `frame`. Pure function of `cue` + `bounds`, so it
/// is trivially reasoned about (and the only place colours land — all from
/// [`theme`]).
fn draw_cue(frame: &mut Frame, bounds: Rectangle, cue: &BeatCueState) {
    let center = Point::new(bounds.width * 0.5, bounds.height * 0.5);
    let dim = bounds.width.min(bounds.height);
    // Pip ring sits inside the arc track; leave room for both + the pips.
    let arc_radius = dim * 0.5 - 12.0;
    let ring_radius = arc_radius - 16.0;
    let pip_radius = (dim * 0.026).clamp(4.0, 8.0);

    let counting_in = cue.count_in_beats.is_some();
    // The ring recedes behind the mint countdown during the pre-count.
    let ring_alpha = if counting_in { 0.35 } else { 1.0 };

    // -- WARM time-until-next arc (skipped during count-in) --------------
    if !counting_in {
        if let Some(remaining) = cue.arc_remaining {
            // Faint full-circle track so the ring always reads as a dial.
            frame.stroke(
                &Path::circle(center, arc_radius),
                Stroke::default().with_width(3.0).with_color(theme::LINE),
            );
            let sweep = remaining.clamp(0.0, 1.0) * TAU;
            if sweep > 0.001 {
                let arc = Path::new(|b| {
                    b.arc(canvas::path::Arc {
                        center,
                        radius: arc_radius,
                        start_angle: Radians(-FRAC_PI_2),
                        end_angle: Radians(-FRAC_PI_2 + sweep),
                    });
                });
                frame.stroke(
                    &arc,
                    Stroke::default()
                        .with_width(3.5)
                        .with_color(theme::WARM)
                        .with_line_cap(canvas::LineCap::Round),
                );
            }
        }
    }

    // -- Beat-ring pips ---------------------------------------------------
    let n = cue.beats_per_bar;
    if n > 0 {
        for i in 0..n {
            let angle = -FRAC_PI_2 + TAU * (i as f32) / (n as f32);
            let p = Point::new(
                center.x + ring_radius * angle.cos(),
                center.y + ring_radius * angle.sin(),
            );
            let beat = i + 1;
            let is_current = cue.rolling && beat == cue.beat_in_bar;
            let is_past = cue.rolling && beat < cue.beat_in_bar;

            if is_current {
                // Lit current beat: bright dot with a soft halo.
                frame.fill(
                    &Path::circle(p, pip_radius * 1.9),
                    faded(theme::TEXT_1, 0.18 * ring_alpha),
                );
                frame.fill(
                    &Path::circle(p, pip_radius * 1.25),
                    faded(theme::TEXT_1, ring_alpha),
                );
            } else if is_past {
                frame.fill(&Path::circle(p, pip_radius), faded(theme::TEXT_3, ring_alpha));
            } else {
                // Upcoming / resting: neutral dark fill with a thin edge.
                frame.fill(&Path::circle(p, pip_radius), faded(theme::BG_3, ring_alpha));
                frame.stroke(
                    &Path::circle(p, pip_radius),
                    Stroke::default()
                        .with_width(1.0)
                        .with_color(faded(theme::LINE, ring_alpha)),
                );
            }
        }
    }

    // -- Count-in countdown ----------------------------------------------
    if let Some(remaining) = cue.count_in_beats {
        frame.fill_text(canvas::Text {
            content: remaining.to_string(),
            position: center,
            color: theme::GOOD,
            size: (dim * 0.42).into(),
            font: theme::SERIF_ITALIC_FONT,
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..canvas::Text::default()
        });
        frame.fill_text(canvas::Text {
            content: "COUNT-IN".to_string(),
            position: Point::new(center.x, center.y + dim * 0.30),
            color: faded(theme::GOOD, 0.8),
            size: (dim * 0.05).into(),
            font: theme::MONO_FONT,
            align_x: iced::alignment::Horizontal::Center.into(),
            align_y: iced::alignment::Vertical::Center,
            ..canvas::Text::default()
        });
    }
}
