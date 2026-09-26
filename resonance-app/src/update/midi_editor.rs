use iced::Task;
use resonance_audio::quantize::{self, Division, GrooveTemplate, QuantizeMode};
use resonance_audio::types::{AudioCommand, ClipId, MidiNote, TrackId};

use crate::message::Message;
use crate::state::{GridChoice, GrooveSelection};
use crate::update::clips;
use crate::Resonance;

#[derive(Debug, Clone)]
pub enum MidiEditorMessage {
    OpenMidiEditor(ClipId),
    /// Open the currently selected MIDI clip (if any) in the piano roll editor.
    OpenSelectedMidiClip,
    CloseMidiEditor,
    AddNote {
        clip_id: ClipId,
        note: u8,
        start_tick: u64,
        duration_ticks: u64,
        velocity: f32,
    },
    RemoveNote {
        clip_id: ClipId,
        note_index: usize,
    },
    /// Remove every currently-selected note from `clip_id` in one edit
    /// (the piano roll's Delete/Backspace on a multi-note selection).
    RemoveSelectedNotes {
        clip_id: ClipId,
    },
    MoveNote {
        clip_id: ClipId,
        note_index: usize,
        new_start_tick: u64,
        new_note: u8,
    },
    ResizeNote {
        clip_id: ClipId,
        note_index: usize,
        new_duration_ticks: u64,
    },
    /// Set one note's velocity in place (control endpoint `notes.edit`,
    /// doc #265, todo #1155). The piano roll has no velocity-drag yet, so
    /// this variant exists for the control surface; undoable + frozen-
    /// input-gated exactly like the other single-note edits.
    SetNoteVelocity {
        clip_id: ClipId,
        note_index: usize,
        velocity: f32,
    },
    /// Replace a clip's whole note array in one edit (control endpoints
    /// `notes.insert_many` / `notes.replace_all`, ba doc #269 FR-5).
    /// The caller passes the final, sorted array; the engine stores it
    /// and echoes one `MidiNotesEdited`. One undo entry and one engine
    /// round trip for the whole batch — writing a 400-note part
    /// note-by-note would cost 400 of each.
    SetClipNotes {
        clip_id: ClipId,
        notes: Vec<resonance_audio::types::MidiNote>,
    },
    /// Replace the selection with a single note, or clear it (`None`).
    /// Used by a plain click and by the vocal roll's single-select path.
    SelectNote {
        note_index: Option<usize>,
    },
    /// Toggle one note's membership in the selection (shift/ctrl-click).
    ToggleNoteSelection {
        note_index: usize,
    },
    /// Apply a rubber-band marquee result: the notes whose rectangles fall
    /// inside the drag rect. `additive` (shift held) unions with the
    /// current selection instead of replacing it.
    SelectNotesInRect {
        indices: Vec<usize>,
        additive: bool,
    },
    /// Select every note in the open clip (Ctrl/Cmd+A).
    SelectAllNotes,
    /// Drop the whole selection (click on empty space).
    ClearNoteSelection,
    PreviewNote(TrackId, u8),
    StopPreview(TrackId, u8),
    ScrollY(f32),
    /// Vocal-roll only: toggle the OpenUtau slur marker on the i-th
    /// note of `clip_id`. `+` continuation ↔ the auto-syllabified
    /// surface form. Lives on this enum so the vocal roll's key
    /// handlers can dispatch through the same router as the other
    /// note edits.
    ToggleSlur {
        clip_id: ClipId,
        note_index: usize,
    },

    // -- Bulk timing edits (quantize / humanize / groove), doc #163, epic #25 --
    // These operate on the *open* MIDI editor clip, so they carry no
    // `clip_id`: the handler reads the active editor and the current
    // multi-note selection (#389), falling back to the whole clip when the
    // selection is empty. Each dispatches one bulk `AudioCommand` (#388)
    // that the engine applies atomically and mirrors back as a single
    // `MidiNotesEdited`; the pre-dispatch undo snapshot captures the prior
    // notes so the whole op is one undo step.
    /// Quantize the current selection (or whole clip) toward `grid`.
    Quantize {
        grid: Division,
        /// Blend toward the grid, `0.0..=1.0` (`1.0` snaps exactly).
        strength: f32,
        /// Swing applied to odd grid steps, `0.0..=1.0`.
        swing: f32,
        mode: QuantizeMode,
        /// Snap note-offs to the grid as well as note-ons.
        quantize_ends: bool,
        /// Apply the strength blend repeatedly (soft/iterative quantize).
        iterative: bool,
    },
    /// Humanize the current selection (or whole clip) with bounded,
    /// seeded timing + velocity jitter. `seed` is `None` for ordinary
    /// invocations — the handler draws one fresh seed per invocation so a
    /// single edit is reproducible (and captured as one undo step); a new
    /// invocation re-rolls. Tests pass `Some(_)` for determinism.
    Humanize {
        /// Maximum absolute timing offset in ticks.
        timing: u32,
        /// Velocity jitter fraction, `0.0..=1.0`.
        vel: f32,
        seed: Option<u64>,
    },
    /// Apply the named groove template to the current selection (or whole
    /// clip). `template_id` names a stock groove today; user/extracted
    /// grooves land with the library-persistence slice (#395).
    ApplyGroove {
        template_id: String,
        /// Template blend, `0.0..=1.0`.
        strength: f32,
    },
    /// Extract a groove template from the open clip at `grid` resolution.
    /// Reads the whole clip (selection-independent); emits
    /// `AudioEvent::GrooveExtracted` and does not modify the notes.
    ExtractGroove {
        grid: Division,
    },

    // -- Quantize panel controls (todo #392) --
    // These write the Quantize panel's settings
    // (`Resonance::midi_quantize`); none of them touch the notes. The
    // panel's Apply button reads those settings to build the bulk
    // `Quantize` message above. Pure view-state edits, so undo skips them.
    /// Set the quantize grid division.
    SetQuantizeGrid(GridChoice),
    /// Set the quantize strength, `0.0..=1.0`.
    SetQuantizeStrength(f32),
    /// Set the swing amount, `0.0..=1.0`.
    SetQuantizeSwing(f32),
    /// Set the quantize mode (start-only vs start+length).
    SetQuantizeMode(QuantizeMode),
    /// Toggle snapping note-ends to the grid.
    SetQuantizeEnds(bool),
    /// Toggle iterative/soft quantize.
    SetQuantizeIterative(bool),
    /// Set the Humanize timing-jitter amount, in ticks (clamped to
    /// `0..=`[`crate::state::HUMANIZE_TIMING_MAX_TICKS`]).
    SetHumanizeTiming(u32),
    /// Set the Humanize velocity-jitter fraction, `0.0..=1.0`.
    SetHumanizeVelocity(f32),

    // -- Groove extract / apply panel controls (todo #394) --
    // Pure view-state edits to the Quantize panel's groove fields
    // (`Resonance::midi_quantize`); none touch the notes, so undo skips
    // them. The Extract / Apply buttons read these to build the bulk
    // `ExtractGroove` / `ApplyGroove` messages above.
    /// Set the name for the next "Extract groove" capture.
    SetGrooveName(String),
    /// Select a groove (stock or user-extracted) in the apply picker.
    SetGrooveSelection(GrooveSelection),
    /// Set the groove apply strength, `0.0..=1.0`.
    SetGrooveStrength(f32),
}

impl MidiEditorMessage {
    /// How this message interacts with the undo history (`undo::classify`
    /// delegates here). Exhaustive on purpose — no `_` arm — so a new
    /// variant does not compile until someone decides what undo does with
    /// it (ARCH-06 A6-4).
    pub(crate) fn undo_action(&self) -> crate::undo::UndoAction {
        use crate::undo::UndoAction;
        match self {
            Self::AddNote { .. }
            | Self::RemoveNote { .. }
            | Self::RemoveSelectedNotes { .. }
            | Self::MoveNote { .. }
            | Self::ResizeNote { .. }
            | Self::SetNoteVelocity { .. }
            | Self::ToggleSlur { .. }
            // Bulk control write (doc #269 FR-5): the pre-dispatch
            // snapshot of the prior notes makes the whole batch one
            // undo step — the entire point of the method.
            | Self::SetClipNotes { .. }
            // Bulk timing edits (doc #163): each rewrites the clip's note
            // array, so the pre-dispatch snapshot of the prior notes is
            // the single undo step. Humanize draws its seed in the handler,
            // so re-doing rolls a new feel — undo still restores the exact
            // prior notes via the snapshot, which is what matters.
            | Self::Quantize { .. }
            | Self::Humanize { .. }
            | Self::ApplyGroove { .. } => UndoAction::Record,
            // Groove *extraction* reads the clip and produces a template;
            // it never mutates the notes, so there's nothing to undo here.
            // Library persistence/undo is a separate slice (#395).
            Self::ExtractGroove { .. } => UndoAction::Skip,
            Self::OpenMidiEditor(_)
            | Self::OpenSelectedMidiClip
            | Self::CloseMidiEditor
            | Self::SelectNote { .. }
            | Self::ToggleNoteSelection { .. }
            | Self::SelectNotesInRect { .. }
            | Self::SelectAllNotes
            | Self::ClearNoteSelection
            | Self::PreviewNote(_, _)
            | Self::StopPreview(_, _)
            | Self::ScrollY(_)
            // Quantize-panel control edits (todo #392) just mutate view
            // state — the actual note edit is the `Quantize` message above.
            | Self::SetQuantizeGrid(_)
            | Self::SetQuantizeStrength(_)
            | Self::SetQuantizeSwing(_)
            | Self::SetQuantizeMode(_)
            | Self::SetQuantizeEnds(_)
            | Self::SetQuantizeIterative(_)
            // Humanize-panel control edits (todo #393) likewise just mutate
            // view state — the note edit is the `Humanize` message above.
            | Self::SetHumanizeTiming(_)
            | Self::SetHumanizeVelocity(_)
            // Groove-panel control edits (todo #394) just mutate view state —
            // the note edit is the `ApplyGroove` message; extract is read-only.
            | Self::SetGrooveName(_)
            | Self::SetGrooveSelection(_)
            | Self::SetGrooveStrength(_) => UndoAction::Skip,
        }
    }
}

pub fn handle(r: &mut Resonance, m: MidiEditorMessage) -> Task<Message> {
    match m {
        MidiEditorMessage::OpenMidiEditor(clip_id) => {
            clips::open_midi_editor(r, clip_id);
        }
        MidiEditorMessage::OpenSelectedMidiClip => {
            if let Some(clip_id) = r.interaction.selected_midi_clip {
                clips::open_midi_editor(r, clip_id);
            }
        }
        MidiEditorMessage::CloseMidiEditor => {
            r.interaction.editing_midi_clip = None;
        }
        MidiEditorMessage::AddNote {
            clip_id,
            note,
            start_tick,
            duration_ticks,
            velocity,
        } => {
            let _ = r.engine.send(AudioCommand::AddMidiNote {
                clip_id,
                note: MidiNote {
                    note,
                    velocity,
                    start_tick,
                    duration_ticks,
                },
            });
        }
        MidiEditorMessage::SetClipNotes { clip_id, notes } => {
            // The app already mirrored these notes (the control handler
            // needs read-your-own-writes); this hands the engine the
            // same authoritative array, and its single MidiNotesEdited
            // echo is drained by the pending-echo token.
            let _ = r.engine.send(AudioCommand::SetMidiClipNotes { clip_id, notes });
        }
        MidiEditorMessage::RemoveNote {
            clip_id,
            note_index,
        } => {
            let _ = r.engine.send(AudioCommand::RemoveMidiNote {
                clip_id,
                note_index,
            });
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.clear_selection();
            }
        }
        MidiEditorMessage::RemoveSelectedNotes { clip_id } => {
            remove_selected_notes(r, clip_id);
        }
        MidiEditorMessage::MoveNote {
            clip_id,
            note_index,
            new_start_tick,
            new_note,
        } => {
            let _ = r.engine.send(AudioCommand::MoveMidiNote {
                clip_id,
                note_index,
                new_start_tick,
                new_note,
            });
        }
        MidiEditorMessage::ResizeNote {
            clip_id,
            note_index,
            new_duration_ticks,
        } => {
            let _ = r.engine.send(AudioCommand::ResizeMidiNote {
                clip_id,
                note_index,
                new_duration_ticks,
            });
        }
        MidiEditorMessage::SetNoteVelocity {
            clip_id,
            note_index,
            velocity,
        } => {
            let _ = r.engine.send(AudioCommand::SetMidiNoteVelocity {
                clip_id,
                note_index,
                velocity,
            });
        }
        MidiEditorMessage::SelectNote { note_index } => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.select_single(note_index);
            }
        }
        MidiEditorMessage::ToggleNoteSelection { note_index } => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.toggle_note(note_index);
            }
        }
        MidiEditorMessage::SelectNotesInRect { indices, additive } => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.apply_marquee(indices, additive);
            }
        }
        MidiEditorMessage::SelectAllNotes => {
            if let Some(clip_id) = r.interaction.editing_midi_clip.as_ref().map(|e| e.clip_id) {
                let len = r
                    .midi_clips
                    .iter()
                    .find(|c| c.id == clip_id)
                    .map(|c| c.notes.len())
                    .unwrap_or(0);
                if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                    editor.select_all(len);
                }
            }
        }
        MidiEditorMessage::ClearNoteSelection => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.clear_selection();
            }
        }
        MidiEditorMessage::PreviewNote(track_id, note) => {
            let _ = r.engine.send(AudioCommand::SendNoteOn {
                track_id,
                note,
                velocity: 0.8,
            });
        }
        MidiEditorMessage::StopPreview(track_id, note) => {
            let _ = r.engine.send(AudioCommand::SendNoteOff { track_id, note });
        }
        MidiEditorMessage::ScrollY(delta) => {
            if let Some(ref mut editor) = r.interaction.editing_midi_clip {
                editor.scroll_y = (editor.scroll_y + delta).max(0.0);
            }
        }
        MidiEditorMessage::ToggleSlur { clip_id, note_index } => {
            toggle_slur(r, clip_id, note_index);
        }
        MidiEditorMessage::Quantize {
            grid,
            strength,
            swing,
            mode,
            quantize_ends,
            iterative,
        } => {
            if let Some((clip_id, indices)) = bulk_target(r) {
                let _ = r.engine.send(AudioCommand::QuantizeMidiNotes {
                    clip_id,
                    indices,
                    grid,
                    strength,
                    swing,
                    mode,
                    quantize_ends,
                    iterative,
                });
            }
        }
        MidiEditorMessage::Humanize { timing, vel, seed } => {
            if let Some((clip_id, indices)) = bulk_target(r) {
                // One seed per invocation: a fresh draw when the caller
                // didn't pin one, so the jitter is reproducible within
                // this single (undoable) edit and a new invocation rolls
                // again. Tests pin `seed` for determinism.
                let seed = seed.unwrap_or_else(humanize_seed);
                let _ = r.engine.send(AudioCommand::HumanizeMidiNotes {
                    clip_id,
                    indices,
                    timing_ticks: timing,
                    vel_amt: vel,
                    seed,
                });
            }
        }
        MidiEditorMessage::ApplyGroove {
            template_id,
            strength,
        } => {
            if let Some((clip_id, indices)) = bulk_target(r) {
                // Unknown template id → no-op (no command, no undo entry).
                if let Some(template) = lookup_groove(r, &template_id) {
                    let _ = r.engine.send(AudioCommand::ApplyGrooveToClip {
                        clip_id,
                        indices,
                        template,
                        strength,
                    });
                }
            }
        }
        MidiEditorMessage::ExtractGroove { grid } => {
            // Extraction reads the whole open clip regardless of selection.
            if let Some(clip_id) = r.interaction.editing_midi_clip.as_ref().map(|e| e.clip_id) {
                // Stash the user's chosen name so the GrooveExtracted event
                // mirror (#390) can file the captured template into the
                // project groove library under it (#394). A blank name
                // becomes an auto-numbered default at file time.
                let name = r.midi_quantize.groove_name.trim();
                r.midi_quantize.pending_groove_name = if name.is_empty() {
                    None
                } else {
                    Some(name.to_string())
                };
                let _ = r
                    .engine
                    .send(AudioCommand::ExtractGrooveFromClip { clip_id, grid });
            }
        }

        // -- Quantize panel controls (todo #392): pure view-state edits.
        MidiEditorMessage::SetQuantizeGrid(grid) => {
            r.midi_quantize.grid = grid;
        }
        MidiEditorMessage::SetQuantizeStrength(v) => {
            r.midi_quantize.strength = v.clamp(0.0, 1.0);
        }
        MidiEditorMessage::SetQuantizeSwing(v) => {
            r.midi_quantize.swing = v.clamp(0.0, 1.0);
        }
        MidiEditorMessage::SetQuantizeMode(mode) => {
            r.midi_quantize.mode = mode;
        }
        MidiEditorMessage::SetQuantizeEnds(on) => {
            r.midi_quantize.quantize_ends = on;
        }
        MidiEditorMessage::SetQuantizeIterative(on) => {
            r.midi_quantize.iterative = on;
        }
        MidiEditorMessage::SetHumanizeTiming(ticks) => {
            r.midi_quantize.humanize_timing =
                ticks.min(crate::state::HUMANIZE_TIMING_MAX_TICKS);
        }
        MidiEditorMessage::SetHumanizeVelocity(v) => {
            r.midi_quantize.humanize_velocity = v.clamp(0.0, 1.0);
        }

        // -- Groove extract / apply panel controls (todo #394) --
        MidiEditorMessage::SetGrooveName(name) => {
            r.midi_quantize.groove_name = name;
        }
        MidiEditorMessage::SetGrooveSelection(sel) => {
            r.midi_quantize.groove_selection = sel;
        }
        MidiEditorMessage::SetGrooveStrength(v) => {
            r.midi_quantize.groove_strength = v.clamp(0.0, 1.0);
        }
    }
    Task::none()
}

/// Resolve the target of a bulk timing edit: the open editor clip plus
/// the note indices to operate on. Uses the current multi-note selection
/// (#389), or every note in the clip when the selection is empty
/// ("operate on the whole clip if none"). Out-of-range selection indices
/// are dropped. Returns `None` — making the op a no-op — when no clip is
/// open, the clip is empty, or no in-range notes remain.
fn bulk_target(r: &Resonance) -> Option<(ClipId, Vec<usize>)> {
    let editor = r.interaction.editing_midi_clip.as_ref()?;
    let clip_id = editor.clip_id;
    let note_count = r
        .midi_clips
        .iter()
        .find(|c| c.id == clip_id)
        .map(|c| c.notes.len())
        .unwrap_or(0);
    if note_count == 0 {
        return None;
    }
    let indices: Vec<usize> = if editor.selected_notes.is_empty() {
        (0..note_count).collect()
    } else {
        editor
            .selected_notes
            .iter()
            .copied()
            .filter(|&i| i < note_count)
            .collect()
    };
    if indices.is_empty() {
        None
    } else {
        Some((clip_id, indices))
    }
}

/// Draw one fresh, well-mixed humanize seed. Called once per `Humanize`
/// invocation (see the handler) so a single bulk edit's jitter is fixed
/// and reproducible while distinct invocations decorrelate.
fn humanize_seed() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    crate::util::next_seed(nanos)
}

/// Resolve a groove `template_id` to its [`GrooveTemplate`]. A
/// `"user:<id>"` id addresses a user-extracted groove in the project
/// library (#394/#395); any other id matches an in-code stock groove by
/// name. Unknown ids return `None` (the apply is then a no-op).
fn lookup_groove(r: &Resonance, template_id: &str) -> Option<GrooveTemplate> {
    if let Some(rest) = template_id.strip_prefix("user:") {
        let id: u64 = rest.parse().ok()?;
        return r.quantize.user_groove(id).map(|g| g.template.clone());
    }
    quantize::stock_grooves()
        .into_iter()
        .find(|(name, _)| name == template_id)
        .map(|(_, template)| template)
}

/// Remove every selected note from `clip_id`. Indices are sent to the
/// engine in descending order so each removal can't invalidate the
/// indices of the not-yet-removed notes below it. Selection is cleared
/// afterwards since the indices no longer refer to anything.
fn remove_selected_notes(r: &mut crate::Resonance, clip_id: resonance_audio::types::ClipId) {
    let Some(editor) = r.interaction.editing_midi_clip.as_ref() else {
        return;
    };
    let note_count = r
        .midi_clips
        .iter()
        .find(|c| c.id == clip_id)
        .map(|c| c.notes.len())
        .unwrap_or(0);
    // Descending order: removing a higher index never shifts a lower one.
    let mut indices: Vec<usize> = editor
        .selected_notes
        .iter()
        .copied()
        .filter(|&i| i < note_count)
        .collect();
    indices.sort_unstable_by(|a, b| b.cmp(a));

    for note_index in indices {
        let _ = r.engine.send(AudioCommand::RemoveMidiNote {
            clip_id,
            note_index,
        });
    }

    if let Some(ref mut editor) = r.interaction.editing_midi_clip {
        editor.clear_selection();
    }
}

/// Toggle the OpenUtau slur marker on the i-th note of `clip_id`. The
/// lyric side-table treats `""` as "use the next syllable from the
/// draft", so flipping to `""` reinstates the cursor-driven label
/// flow — every subsequent non-slur note slides its syllable one slot
/// left, and the now-spare syllable at the tail returns to the draft.
/// Flipping to `"+"` does the reverse: the trailing syllables slide
/// right.
fn toggle_slur(
    r: &mut crate::Resonance,
    clip_id: resonance_audio::types::ClipId,
    note_index: usize,
) {
    use resonance_music_theory::g2p;

    let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) else {
        return;
    };
    if note_index >= clip.notes.len() {
        return;
    }
    let note_count = clip.notes.len();

    let entry = r
        .compose
        .vocal_audio
        .clip_lyrics
        .entry(clip_id)
        .or_default();
    if entry.len() < note_count {
        entry.resize(note_count, String::new());
    }
    if g2p::is_slur_lyric(&entry[note_index]) {
        entry[note_index] = String::new();
    } else {
        entry[note_index] = g2p::SLUR_MARKER.to_string();
    }
}
