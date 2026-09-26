//! Input handling, hit-testing, and quantize-grid math for the piano-roll
//! canvas. Separated from the rendering pipeline (see `draw.rs`) because
//! these change for different reasons: edit UX vs visual layout.

use iced::{mouse, Rectangle};
use iced::widget::canvas;

use crate::message::*;
use crate::view::piano_roll::{self, hit_test_note, NoteEdge};

use resonance_audio::quantize::quantize_notes;
use resonance_audio::types::{MidiNote, TempoMap};

use super::{PianoRollCanvas, PianoRollState, DEFAULT_VELOCITY, QuantizePreview};

/// Swing delay (ticks) applied to odd grid steps for step size `g`,
/// mirroring `resonance_audio::quantize`'s private `swing_delay` so the
/// drawn grid lines land exactly where the ghost preview snaps notes.
fn swing_delay(g: u64, swing: f32) -> u64 {
    let s = swing.clamp(0.0, 1.0) as f64;
    (s * g as f64 / 2.0).round() as u64
}

/// Local tick offsets (within a bar of `bar_len` ticks) of every quantize
/// grid line for step size `step_ticks`, swung on odd steps by `swing`.
///
/// The returned offsets start at the downbeat (`0`) and never exceed
/// `bar_len`; odd-indexed steps are delayed by [`swing_delay`] so a triplet
/// / swing feel reads as the off-beats sliding later. This is the single
/// source of grid-line geometry — the renderer walks it and its unit tests
/// assert it — so the drawn lines and the ghost snap can't drift apart.
pub fn quantize_grid_steps(step_ticks: u64, bar_len: u64, swing: f32) -> Vec<u64> {
    if step_ticks == 0 || bar_len == 0 {
        return Vec::new();
    }
    let delay = swing_delay(step_ticks, swing);
    let mut out = Vec::new();
    let mut k = 0u64;
    loop {
        let base = k * step_ticks;
        if base >= bar_len {
            break;
        }
        let local = if k % 2 == 1 {
            (base + delay).min(bar_len)
        } else {
            base
        };
        out.push(local);
        k += 1;
    }
    out
}

/// Quantized target positions for `selection`, anchored at clip tick 0 so
/// the ghost preview lands exactly on the grid drawn by
/// [`quantize_grid_steps`]. A thin wrapper over the pure
/// [`quantize_notes`] used by the engine's Apply, so the preview and the
/// committed result agree note-for-note (modulo the clip-start anchoring
/// the visible grid already assumes).
pub fn ghost_targets(
    notes: &[MidiNote],
    selection: &[usize],
    q: &QuantizePreview,
    tempo: &TempoMap,
) -> Vec<MidiNote> {
    quantize_notes(
        notes,
        selection,
        q.division,
        q.strength,
        q.swing,
        q.mode,
        q.quantize_ends,
        q.iterative,
        tempo,
        0,
    )
}

/// Dispatch a canvas `event` for the piano roll, returning any resulting
/// `Action` (captured event + optional published `Message`).
///
/// This is the sole entry point for input; the `canvas::Program::update`
/// impl in `mod.rs` delegates here so that interaction logic is kept
/// separate from the rendering pipeline in `draw.rs`.
pub(super) fn handle_event(
    canvas: &PianoRollCanvas<'_>,
    state: &mut PianoRollState,
    event: &iced::Event,
    bounds: Rectangle,
    cursor: mouse::Cursor,
) -> Option<canvas::Action<Message>> {
    let layout = canvas.layout(bounds);
    let viewport = canvas.viewport();
    let grid_x = layout.grid_x();
    let grid_h = layout.grid_h;

    state.key_focus.track(event, bounds, cursor);
    if matches!(event, iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { .. }))
        && !state.key_focus.owns_keys()
    {
        return None;
    }

    match event {
        // --- Scroll ---
        iced::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
            // Only handle wheel events when the cursor is actually over the
            // piano roll — otherwise scrolling the arrangement would also
            // scroll this editor.
            cursor.position_in(bounds)?;
            // Horizontal scroll is handled by the outer `Scrollable`
            // that wraps this canvas now (see `view_midi_editor_panel`).
            // Returning `Ignored` lets the event bubble up. Vertical
            // pitch scroll stays inside the canvas because the
            // keyboard column needs to scroll in lockstep with the
            // note rows.
            match delta {
                mouse::ScrollDelta::Lines { x, y } => {
                    if x.abs() > f32::EPSILON {
                        return None;
                    }
                    return Some(
                        canvas::Action::publish(Message::MidiEditor(
                            MidiEditorMessage::ScrollY(-y * 30.0),
                        ))
                        .and_capture(),
                    );
                }
                mouse::ScrollDelta::Pixels { x, y } => {
                    if x.abs() > f32::EPSILON {
                        return None;
                    }
                    return Some(
                        canvas::Action::publish(Message::MidiEditor(
                            MidiEditorMessage::ScrollY(-y),
                        ))
                        .and_capture(),
                    );
                }
            }
        }

        // --- Mouse press ---
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
            if let Some(pos) = cursor.position_in(bounds) {
                // Piano keyboard area: preview note
                if pos.x < grid_x && pos.y < grid_h {
                    let note = viewport.y_local_to_note(pos.y);
                    state.previewing_note = Some(note);
                    return Some(
                        canvas::Action::publish(Message::MidiEditor(
                            MidiEditorMessage::PreviewNote(canvas.track_id, note),
                        ))
                        .and_capture(),
                    );
                }

                // Velocity lane: not interactive for now (future: drag velocity bars)
                if pos.y >= grid_h {
                    return None;
                }

                // Note grid area
                if pos.x >= grid_x {
                    let click_tick = viewport.x_local_to_tick(pos.x - grid_x);
                    let click_note = viewport.y_local_to_note(pos.y);
                    // Shift/Ctrl held → additive selection edit, not a drag.
                    let additive = state.modifiers.shift() || state.modifiers.command();

                    // Check if clicking on an existing note
                    for (i, n) in canvas.clip.notes.iter().enumerate() {
                        let rect = canvas.note_rect(&layout, &viewport, n);
                        if let Some(edge) = hit_test_note(rect, pos) {
                            // Shift/Ctrl-click toggles the note's membership
                            // and never starts a drag — it's purely a
                            // selection edit.
                            if additive {
                                return Some(
                                    canvas::Action::publish(Message::MidiEditor(
                                        MidiEditorMessage::ToggleNoteSelection { note_index: i },
                                    ))
                                    .and_capture(),
                                );
                            }
                            state.drag = Some(match edge {
                                NoteEdge::ResizeRight => super::DragMode::ResizeNote {
                                    note_index: i,
                                    anchor_tick: n.start_tick,
                                },
                                NoteEdge::Body => {
                                    let tick_offset = n.start_tick as i64 - click_tick as i64;
                                    super::DragMode::MoveNote {
                                        note_index: i,
                                        start_tick_offset: tick_offset,
                                        notes: canvas.clip.notes.clone(),
                                    }
                                }
                            });
                            return Some(
                                canvas::Action::publish(Message::MidiEditor(
                                    MidiEditorMessage::SelectNote {
                                        note_index: Some(i),
                                    },
                                ))
                                .and_capture(),
                            );
                        }
                    }

                    // Empty space: defer the decision to release. A
                    // negligible drag is a click that creates a note; a
                    // longer drag is a rubber-band marquee selection.
                    let snapped = canvas.snap(click_tick);
                    state.empty_drag = Some(super::EmptyDrag {
                        origin: pos,
                        current: pos,
                        create_note: click_note,
                        create_tick: snapped,
                    });
                    return Some(canvas::Action::capture());
                }
            }
        }

        // --- Right-click: remove selected note ---
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)) => {
            if let Some(pos) = cursor.position_in(bounds) {
                if pos.x >= grid_x && pos.y < grid_h {
                    for (i, n) in canvas.clip.notes.iter().enumerate() {
                        let rect = canvas.note_rect(&layout, &viewport, n);
                        if hit_test_note(rect, pos).is_some() {
                            return Some(
                                canvas::Action::publish(Message::MidiEditor(
                                    MidiEditorMessage::RemoveNote {
                                        clip_id: canvas.clip.id,
                                        note_index: i,
                                    },
                                ))
                                .and_capture(),
                            );
                        }
                    }
                }
            }
        }

        // --- Mouse move (drag) ---
        iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
            if let Some(pos) = cursor.position_in(bounds) {
                // Extend an in-progress empty-grid press (marquee). The
                // overlay repaints on the next periodic UI tick.
                if let Some(ref mut ed) = state.empty_drag {
                    ed.current = pos;
                    return Some(canvas::Action::capture());
                }
                match &mut state.drag {
                    Some(super::DragMode::MoveNote {
                        note_index,
                        start_tick_offset,
                        notes,
                    }) if pos.x >= grid_x && pos.y < grid_h => {
                        let tick = viewport.x_local_to_tick(pos.x - grid_x);
                        let raw_tick = (tick as i64 + *start_tick_offset).max(0) as u64;
                        let snapped_tick = canvas.snap(raw_tick);
                        let note = viewport.y_local_to_note(pos.y);
                        let msg = MidiEditorMessage::MoveNote {
                            clip_id: canvas.clip.id,
                            note_index: *note_index,
                            new_start_tick: snapped_tick,
                            new_note: note,
                        };
                        *note_index = resonance_audio::types::move_note_resorted(
                            notes,
                            *note_index,
                            snapped_tick,
                            note,
                        );
                        return Some(
                            canvas::Action::publish(Message::MidiEditor(msg)).and_capture(),
                        );
                    }
                    Some(super::DragMode::ResizeNote {
                        note_index,
                        anchor_tick,
                    }) if pos.x >= grid_x => {
                        let tick = viewport.x_local_to_tick(pos.x - grid_x);
                        let snapped = canvas.snap(tick);
                        let new_dur =
                            snapped.saturating_sub(*anchor_tick).max(canvas.snap_ticks);
                        return Some(
                            canvas::Action::publish(Message::MidiEditor(
                                MidiEditorMessage::ResizeNote {
                                    clip_id: canvas.clip.id,
                                    note_index: *note_index,
                                    new_duration_ticks: new_dur,
                                },
                            ))
                            .and_capture(),
                        );
                    }
                    Some(_) | None => {}
                }
            }
        }

        // --- Mouse release ---
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            state.drag = None;
            // Resolve an empty-grid press.
            if let Some(ed) = state.empty_drag.take() {
                if ed.is_marquee() {
                    let indices = piano_roll::notes_in_marquee(
                        &canvas.clip.notes,
                        &layout,
                        &viewport,
                        ed.rect(),
                    );
                    let additive = state.modifiers.shift();
                    return Some(
                        canvas::Action::publish(Message::MidiEditor(
                            MidiEditorMessage::SelectNotesInRect { indices, additive },
                        ))
                        .and_capture(),
                    );
                } else if !canvas.selected_notes.is_empty() {
                    // Plain click on empty space with an active selection
                    // clears it rather than creating a note.
                    return Some(
                        canvas::Action::publish(Message::MidiEditor(
                            MidiEditorMessage::ClearNoteSelection,
                        ))
                        .and_capture(),
                    );
                } else {
                    // Nothing selected: a plain empty click creates a note.
                    return Some(
                        canvas::Action::publish(Message::MidiEditor(
                            MidiEditorMessage::AddNote {
                                clip_id: canvas.clip.id,
                                note: ed.create_note,
                                start_tick: ed.create_tick,
                                duration_ticks: canvas.snap_ticks,
                                velocity: DEFAULT_VELOCITY,
                            },
                        ))
                        .and_capture(),
                    );
                }
            }
            if let Some(note) = state.previewing_note.take() {
                return Some(
                    canvas::Action::publish(Message::MidiEditor(
                        MidiEditorMessage::StopPreview(canvas.track_id, note),
                    ))
                    .and_capture(),
                );
            }
        }

        // --- Delete key: remove selected note ---
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Delete),
            ..
        })
        | iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Backspace),
            ..
        }) => {
            if !canvas.selected_notes.is_empty() {
                return Some(
                    canvas::Action::publish(Message::MidiEditor(
                        MidiEditorMessage::RemoveSelectedNotes {
                            clip_id: canvas.clip.id,
                        },
                    ))
                    .and_capture(),
                );
            }
        }

        // --- Ctrl/Cmd+Shift+A: select the notes currently in view ---
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character(ref c),
            modifiers,
            ..
        }) if modifiers.command()
            && modifiers.shift()
            && c.as_str().eq_ignore_ascii_case("a") =>
        {
            let view_rect = Rectangle {
                x: grid_x,
                y: 0.0,
                width: (bounds.width - grid_x).max(0.0),
                height: grid_h,
            };
            let indices = piano_roll::notes_in_marquee(
                &canvas.clip.notes,
                &layout,
                &viewport,
                view_rect,
            );
            return Some(
                canvas::Action::publish(Message::MidiEditor(
                    MidiEditorMessage::SelectNotesInRect {
                        indices,
                        additive: false,
                    },
                ))
                .and_capture(),
            );
        }

        // --- Ctrl/Cmd+A: select every note in the clip ---
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Character(ref c),
            modifiers,
            ..
        }) if modifiers.command() && c.as_str().eq_ignore_ascii_case("a") => {
            return Some(
                canvas::Action::publish(Message::MidiEditor(
                    MidiEditorMessage::SelectAllNotes,
                ))
                .and_capture(),
            );
        }

        // --- Track modifier state for shift/ctrl-aware mouse clicks ---
        iced::Event::Keyboard(iced::keyboard::Event::ModifiersChanged(mods)) => {
            state.modifiers = *mods;
        }

        _ => {}
    }
    None
}
