//! MIDI Import modal shell + Review stage body. Shares the export/bounce
//! modal scaffold — dimmed `mouse_area` backdrop, centered container on
//! `BG_2` with a `LINE` border and `RADIUS_XL` corners, serif-italic title
//! — so it reads as one family with `view::bounce_dialog`.
//!
//! The Review stage (the populated default of doc #158) is built here: a
//! summary band that surfaces what the parser found (SMF format, track
//! count, PPQ, length, tempo range) plus the "Tracks to import" list of
//! per-track checkbox rows with an inline rename field, a kind swatch, a
//! channel chip, a note-count + pitch-range readout, and a slot reserved
//! for the mini piano-roll preview Canvas (todo #511). All / None quick
//! toggles and a live selected-count sit above the list. Selected rows use
//! the `ACCENT_DIM` wash + filled-checkbox treatment shared with the
//! export modal's track rows.
//!
//! Review ends in an Import button, enabled exactly when Confirm would be
//! accepted; TempoConflict offers the three ways to reconcile a tempo
//! difference and a Continue; Imported reports what landed (code review
//! FU-V2a). Drop / Parsing / Error stay single labelled lines. Review also
//! carries minimal placement controls (#507, code review FU-V4a): start at
//! bar 1 or the playhead, and new tracks or a merge into one existing
//! instrument/vocal track.

use iced::widget::{
    button, column, container, mouse_area, opaque, row, scrollable, stack, text, text_input, Space,
};
use iced::{alignment, Element, Length};

use crate::message::{ImportMessage, Message};
use crate::state::{
    ImportDialogState, ImportStage, ImportSummary, ImportTrackKind, PlacementMode, PlacementStart,
    TempoAlignment, TempoChoice, TrackImportRow,
};
use crate::theme;
use crate::Resonance;

pub(crate) fn view_import_dialog_overlay<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let Some(dialog) = r.import_dialog.as_ref() else {
        return Space::new()
            .width(Length::Fixed(0.0))
            .height(Length::Fixed(0.0))
            .into();
    };

    let backdrop = mouse_area(
        container(Space::new().width(Length::Fill).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_theme| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.6,
                ))),
                ..Default::default()
            }),
    )
    .on_press(Message::Import(ImportMessage::Cancel));

    let title = text("Import MIDI")
        .size(20)
        .font(theme::SERIF_ITALIC_FONT)
        .color(theme::TEXT_1);

    // The Review stage gets its own populated body; the remaining stages
    // fall back to a single labelled line until their todos land. The
    // dialog widens for Review so the track table breathes.
    let (body, width): (Element<'a, Message>, f32) = match dialog.stage {
        ImportStage::Review => (review_body(r, dialog), 600.0),
        ImportStage::TempoConflict => (tempo_conflict_body(r, dialog), 460.0),
        ImportStage::Imported => (imported_body(dialog), 460.0),
        _ => (stage_placeholder(dialog), 460.0),
    };

    // Once the import has landed the only way on is out.
    let cancel_label = if dialog.stage == ImportStage::Imported {
        "Done"
    } else {
        "Cancel"
    };
    let cancel_btn = button(text(cancel_label).size(13).color(theme::TEXT_1))
        .on_press(Message::Import(ImportMessage::Cancel))
        .padding([8, 18])
        .style(|_theme, status| theme::ghost_button_style(status));

    // The stage's primary action: Import on Review (enabled exactly when
    // Confirm would be accepted — `confirm_blocker` is also the gate),
    // Continue on TempoConflict (code review FU-V2a).
    let primary: Option<(String, Option<ImportMessage>)> = match dialog.stage {
        ImportStage::Review => {
            let selected = dialog.selected_count();
            let label = format!("Import {}", plural(selected, "track", "tracks"));
            let enabled = crate::update::import::confirm_blocker(r).is_none();
            Some((label, enabled.then_some(ImportMessage::Confirm)))
        }
        ImportStage::TempoConflict => {
            Some(("Continue".to_owned(), Some(ImportMessage::ResolveTempo)))
        }
        _ => None,
    };

    let mut button_row = row![Space::new().width(Length::Fill), cancel_btn]
        .spacing(8)
        .align_y(alignment::Vertical::Center);
    if let Some((label, action)) = primary {
        button_row = button_row.push(primary_button(label, action));
    }

    let dialog_content = column![
        title,
        Space::new().height(14),
        body,
        Space::new().height(20),
        button_row,
    ]
    .spacing(0)
    .padding(24)
    .width(width);

    let dialog_box = container(dialog_content).style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_2)),
        border: iced::Border {
            color: theme::LINE,
            width: 1.0,
            radius: theme::RADIUS_XL.into(),
        },
        ..Default::default()
    });

    let centered = container(opaque(dialog_box))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill);

    stack![backdrop, centered].into()
}

/// Placeholder body for the not-yet-built stages: name the current step so
/// the shell is legible. An error message, when set, takes precedence.
fn stage_placeholder<'a>(dialog: &'a ImportDialogState) -> Element<'a, Message> {
    let stage_label = match dialog.stage {
        ImportStage::Drop => "Drop a MIDI file to import, or choose one.",
        ImportStage::Parsing => "Parsing\u{2026}",
        ImportStage::Review => "Review the tracks to import.",
        ImportStage::TempoConflict => "Resolve the tempo difference.",
        ImportStage::Error => "Import failed.",
        ImportStage::Imported => "Import complete.",
    };
    let label = text(dialog.error.as_deref().unwrap_or(stage_label))
        .size(13)
        .color(theme::TEXT_2);
    // The Drop prompt says "or choose one" — and a failed parse needs a
    // way to pick another file — so both carry the chooser (VIEW-25).
    if matches!(dialog.stage, ImportStage::Drop | ImportStage::Error) {
        let choose = button(text("Choose file\u{2026}").size(13).color(theme::TEXT_1))
            .on_press(Message::Import(ImportMessage::Choose))
            .padding([8, 18])
            .style(|_theme, status| theme::ghost_button_style(status));
        column![label, Space::new().height(12), choose].into()
    } else {
        label.into()
    }
}

/// The dialog's primary action button; `None` renders it disabled.
fn primary_button<'a>(label: String, action: Option<ImportMessage>) -> Element<'a, Message> {
    let active = action.is_some();
    let mut b = button(
        text(label)
            .size(13)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(if active { theme::BG_0 } else { theme::TEXT_3 }),
    )
    .padding([8, 18])
    .style(move |_theme, status| {
        if active {
            theme::primary_button_style(status)
        } else {
            theme::ghost_button_style(status)
        }
    });
    if let Some(message) = action {
        b = b.on_press(Message::Import(message));
    }
    b.into()
}

// ---------------------------------------------------------------------------
// Tempo-conflict + imported stage bodies
// ---------------------------------------------------------------------------

/// The file's tempo differs from the project's: offer the three ways to
/// reconcile them (doc #158). The choice is applied by Confirm.
fn tempo_conflict_body<'a>(r: &'a Resonance, dialog: &'a ImportDialogState) -> Element<'a, Message> {
    let project_bpm = r.tempo_events.first().map_or(r.transport.bpm, |e| e.bpm);
    let file_tempo = dialog
        .summary
        .as_ref()
        .and_then(tempo_range_label)
        .unwrap_or_else(|| "a different tempo".to_owned());
    let intro = text(format!(
        "This file plays at {file_tempo}; the project is at {} BPM.",
        round_bpm(project_bpm)
    ))
    .size(13)
    .color(theme::TEXT_2);

    let keep = dialog.tempo_choice == TempoChoice::KeepProject;
    let options = column![
        tempo_option(
            "Keep project tempo \u{2014} match bars",
            "Notes keep their bar and beat positions.",
            keep && dialog.tempo_alignment == TempoAlignment::MatchBars,
            ImportMessage::ChooseTempo(TempoChoice::KeepProject, TempoAlignment::MatchBars),
        ),
        tempo_option(
            "Keep project tempo \u{2014} match time",
            "Notes keep their timing in seconds.",
            keep && dialog.tempo_alignment == TempoAlignment::MatchTime,
            ImportMessage::ChooseTempo(TempoChoice::KeepProject, TempoAlignment::MatchTime),
        ),
        tempo_option(
            "Use the file's tempo",
            "The project adopts the file's tempo map.",
            !keep,
            ImportMessage::ChooseTempo(TempoChoice::AdoptFile, dialog.tempo_alignment),
        ),
    ]
    .spacing(6);

    column![intro, Space::new().height(12), options].into()
}

/// One selectable tempo option: a title + one-line explanation, washed
/// with the selected-row treatment when it is the current choice.
fn tempo_option<'a>(
    title: &'a str,
    detail: &'a str,
    selected: bool,
    on_press: ImportMessage,
) -> Element<'a, Message> {
    let content = column![
        text(title)
            .size(13)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_1),
        text(detail).size(12).color(theme::TEXT_2),
    ]
    .spacing(2);
    button(content)
        .width(Length::Fill)
        .padding([8, 12])
        .on_press(Message::Import(on_press))
        .style(move |_theme, _status| {
            let s = row_style(selected, false);
            button::Style {
                background: s.background,
                border: s.border,
                text_color: theme::TEXT_1,
                ..Default::default()
            }
        })
        .into()
}

/// What the import added.
fn imported_body<'a>(dialog: &'a ImportDialogState) -> Element<'a, Message> {
    let line = match dialog.result {
        Some(res) => format!(
            "Imported {} into {}, on {}.",
            plural(res.notes_imported, "note", "notes"),
            plural(res.clips_added, "clip", "clips"),
            if res.tracks_created > 0 {
                plural(res.tracks_created, "new track", "new tracks")
            } else {
                "the selected track".to_owned()
            },
        ),
        None => "Import complete.".to_owned(),
    };
    text(line).size(13).color(theme::TEXT_2).into()
}

// ---------------------------------------------------------------------------
// Review stage body
// ---------------------------------------------------------------------------

/// The populated Review screen: summary band, the All/None + count header,
/// and the scrollable tracks-to-import list.
fn review_body<'a>(r: &'a Resonance, dialog: &'a ImportDialogState) -> Element<'a, Message> {
    let summary: Element<'a, Message> = match dialog.summary.as_ref() {
        Some(s) => summary_band(s),
        None => Space::new().height(Length::Fixed(0.0)).into(),
    };

    // Header row: "Tracks to import" label, All/None quick toggles, and a
    // live "N selected" readout on the right.
    let selected = dialog.selected_count();
    let importable = dialog.importable_count();
    let all_disabled = importable == 0 || selected == importable;
    let none_disabled = selected == 0;

    let header = row![
        text("Tracks to import")
            .size(13)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_1),
        Space::new().width(Length::Fill),
        quick_toggle("All", !all_disabled, ImportMessage::SetAllTracks(true)),
        quick_toggle("None", !none_disabled, ImportMessage::SetAllTracks(false)),
        Space::new().width(10),
        text(format!("{selected} selected")).size(12).color(theme::TEXT_2),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);

    let mut list = column![].spacing(6);
    for (i, row) in dialog.rows.iter().enumerate() {
        list = list.push(track_row(i, row));
    }

    // Lazy-wrap the non-live track list inside a fixed-height scroll
    // region (view-performance rules: only the rename inputs are live, and
    // they repaint their own subtree).
    let list_scroll = scrollable(container(list).padding(iced::Padding {
        right: 6.0,
        ..iced::Padding::ZERO
    }))
    .height(Length::Fixed(300.0));

    column![
        summary,
        Space::new().height(16),
        header,
        Space::new().height(8),
        list_scroll,
        Space::new().height(12),
        placement_controls(r, dialog),
    ]
    .spacing(0)
    .into()
}

/// Where the import lands: a Start row (bar 1 / playhead) and an Into row
/// (new tracks / merge into one existing instrument or vocal track, whose
/// candidates appear once Merge is chosen). Plain toggles on the dialog's
/// existing messages; `confirm_blocker` still has the last word.
fn placement_controls<'a>(r: &'a Resonance, dialog: &'a ImportDialogState) -> Element<'a, Message> {
    let p = dialog.placement;
    let label = |s: &'a str| text(s).size(12).color(theme::TEXT_2).width(Length::Fixed(44.0));
    let start = row![
        label("Start"),
        choice_chip(
            "Bar 1",
            p.start == PlacementStart::Bar1,
            ImportMessage::SetPlacementStart(PlacementStart::Bar1)
        ),
        choice_chip(
            "Playhead",
            p.start == PlacementStart::Playhead,
            ImportMessage::SetPlacementStart(PlacementStart::Playhead)
        ),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);
    let into = row![
        label("Into"),
        choice_chip(
            "New tracks",
            p.mode == PlacementMode::NewTracks,
            ImportMessage::SetPlacementMode(PlacementMode::NewTracks)
        ),
        choice_chip(
            "Merge into…",
            p.mode == PlacementMode::MergeIntoSelected,
            ImportMessage::SetPlacementMode(PlacementMode::MergeIntoSelected)
        ),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);

    let mut controls = column![start, into].spacing(4);
    if p.mode == PlacementMode::MergeIntoSelected {
        let mut targets = row![label("")].spacing(6);
        for t in r.registry.tracks.iter().filter(|t| {
            matches!(
                t.track_type,
                resonance_audio::types::TrackType::Instrument
                    | resonance_audio::types::TrackType::Vocal
            )
        }) {
            targets = targets.push(choice_chip(
                &t.name,
                p.merge_target == Some(t.id),
                ImportMessage::SetMergeTarget(Some(t.id)),
            ));
        }
        controls = controls.push(scrollable(targets).direction(
            scrollable::Direction::Horizontal(scrollable::Scrollbar::new()),
        ));
    }
    controls.into()
}

/// One option of a small radio group: the selected one wears the row
/// selection wash, the others are pressable.
fn choice_chip<'a>(label: &'a str, selected: bool, msg: ImportMessage) -> Element<'a, Message> {
    let color = if selected { theme::ACCENT_SOFT } else { theme::TEXT_2 };
    button(text(label).size(12).color(color))
        .padding([4, 10])
        .on_press(Message::Import(msg))
        .style(move |_theme, status| {
            if selected {
                let s = row_style(true, false);
                button::Style {
                    background: s.background,
                    border: s.border,
                    text_color: color,
                    ..Default::default()
                }
            } else {
                theme::ghost_button_style(status)
            }
        })
        .into()
}

/// Summary band: file name + an SMF-format chip on top, then a row of stat
/// chips (tracks · notes · PPQ · length · tempo range) so the user trusts
/// the parse before committing.
fn summary_band<'a>(s: &'a ImportSummary) -> Element<'a, Message> {
    let smf_chip: Element<'a, Message> = match s.smf_format {
        Some(fmt) => accent_chip(format!("SMF {fmt}")),
        None => Space::new().width(Length::Fixed(0.0)).into(),
    };

    let top = row![
        text(&s.file_name)
            .size(13)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::TEXT_1),
        Space::new().width(Length::Fill),
        smf_chip,
    ]
    .align_y(alignment::Vertical::Center);

    let mut chips = row![].spacing(6).align_y(alignment::Vertical::Center);
    chips = chips.push(stat_chip(plural(s.track_count, "track", "tracks")));
    chips = chips.push(stat_chip(plural(s.total_notes, "note", "notes")));
    if let Some(ppq) = s.ppq {
        chips = chips.push(stat_chip(format!("{ppq} PPQ")));
    }
    if let Some(bars) = s.length_bars {
        chips = chips.push(stat_chip(plural(bars as usize, "bar", "bars")));
    }
    if let Some(tempo) = tempo_range_label(s) {
        chips = chips.push(stat_chip(tempo));
    }

    container(column![top, Space::new().height(10), chips].spacing(0))
        .width(Length::Fill)
        .padding(14)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::LINE_2)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}

/// One track-to-import row: checkbox · kind swatch · inline rename (+ note
/// readout below) · channel chip · preview slot. A notes-less
/// conductor/tempo row is rendered disabled with a "tempo & meter only"
/// note. Selected rows get the `ACCENT_DIM` wash + filled checkbox.
fn track_row<'a>(index: usize, r: &'a TrackImportRow) -> Element<'a, Message> {
    let conductor = r.is_conductor;
    let selected = r.selected && !conductor;

    let check = checkbox(index, selected, conductor);
    let swatch = kind_swatch(r.kind, conductor);

    // Name: an inline rename field for real tracks; a static, dimmed label
    // for the conductor (it carries no importable notes to rename).
    let name: Element<'a, Message> = if conductor {
        text(&r.name).size(13).color(theme::TEXT_3).into()
    } else {
        text_input("Track name", &r.name)
            .on_input(move |s| Message::Import(ImportMessage::RenameTrack(index, s)))
            .size(13)
            .padding([4, 8])
            .style(rename_input_style)
            .into()
    };

    let readout: Element<'a, Message> = if conductor {
        text("tempo & meter only").size(11).color(theme::TEXT_3).into()
    } else {
        text(note_readout(r)).size(11).color(theme::TEXT_2).into()
    };

    let name_col = column![name, Space::new().height(3), readout].spacing(0);

    // Channel chip — GM channels are 0-based internally; show the 1-based
    // value musicians expect. Suppressed for the conductor row.
    let channel: Element<'a, Message> = if conductor {
        Space::new().width(Length::Fixed(0.0)).into()
    } else {
        stat_chip(format!("Ch {}", r.channel as u16 + 1))
    };

    // Slot reserved for the mini piano-roll preview Canvas (todo #511);
    // for now a static placeholder so the row layout is final.
    let preview = preview_slot(conductor);

    let content = row![
        check,
        Space::new().width(10),
        swatch,
        Space::new().width(10),
        name_col,
        Space::new().width(Length::Fill),
        channel,
        Space::new().width(8),
        preview,
    ]
    .align_y(alignment::Vertical::Center);

    container(content)
        .width(Length::Fill)
        .padding([8, 10])
        .style(move |_theme| row_style(selected, conductor))
        .into()
}

// ---------------------------------------------------------------------------
// Small building blocks
// ---------------------------------------------------------------------------

/// Per-row selection checkbox: a filled accent box with a check when
/// selected, a hollow outline otherwise. Disabled (no press, muted) for a
/// conductor row, which can never be imported.
fn checkbox<'a>(index: usize, selected: bool, conductor: bool) -> Element<'a, Message> {
    let glyph: Element<'a, Message> = if selected {
        theme::icon(theme::fa::CHECK).size(11).color(theme::BG_0).into()
    } else {
        Space::new().width(Length::Fixed(0.0)).into()
    };

    let mut b = button(
        container(glyph)
            .width(Length::Fixed(18.0))
            .height(Length::Fixed(18.0))
            .center_x(Length::Fixed(18.0))
            .center_y(Length::Fixed(18.0)),
    )
    .padding(0)
    .style(move |_theme, _status| {
        let (bg, border) = if conductor {
            (iced::Color::TRANSPARENT, theme::TEXT_4)
        } else if selected {
            (theme::ACCENT, theme::ACCENT)
        } else {
            (iced::Color::TRANSPARENT, theme::TEXT_3)
        };
        button::Style {
            background: Some(iced::Background::Color(bg)),
            text_color: theme::BG_0,
            border: iced::Border {
                color: border,
                width: 1.5,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        }
    });
    if !conductor {
        b = b.on_press(Message::Import(ImportMessage::ToggleTrack(index)));
    }
    b.into()
}

/// Colored kind swatch: a small rounded square tinted + iconed by track
/// kind (instrument / drum / vocal), or a muted metronome for the
/// conductor/tempo track.
fn kind_swatch<'a>(kind: ImportTrackKind, conductor: bool) -> Element<'a, Message> {
    let (glyph, tint) = if conductor {
        (theme::fa::METRONOME, theme::TEXT_3)
    } else {
        match kind {
            ImportTrackKind::Instrument => (theme::fa::MUSIC, theme::ACCENT),
            ImportTrackKind::Drum => (theme::fa::DRUM, theme::WARM),
            ImportTrackKind::Vocal => (theme::fa::MICROPHONE, theme::GOOD),
        }
    };

    container(theme::icon(glyph).size(12).color(tint))
        .width(Length::Fixed(26.0))
        .height(Length::Fixed(26.0))
        .center_x(Length::Fixed(26.0))
        .center_y(Length::Fixed(26.0))
        .style(move |_theme| container::Style {
            background: Some(iced::Background::Color(alpha(tint, 0.14))),
            border: iced::Border {
                color: alpha(tint, 0.34),
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Reserved space for the per-row mini piano-roll preview Canvas (#511).
fn preview_slot<'a>(conductor: bool) -> Element<'a, Message> {
    let inner: Element<'a, Message> = if conductor {
        text("—").size(11).color(theme::TEXT_4).into()
    } else {
        Space::new().width(Length::Fill).height(Length::Fill).into()
    };
    container(inner)
        .width(Length::Fixed(108.0))
        .height(Length::Fixed(30.0))
        .center_x(Length::Fixed(108.0))
        .center_y(Length::Fixed(30.0))
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_1)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

/// A small ghost "All"/"None" toggle button. Greyed (no press) when it
/// would be a no-op (e.g. "All" when everything is already selected).
fn quick_toggle<'a>(label: &'a str, active: bool, msg: ImportMessage) -> Element<'a, Message> {
    let color = if active { theme::ACCENT_SOFT } else { theme::TEXT_3 };
    let mut b = button(text(label).size(12).color(color))
        .padding([4, 10])
        .style(|_theme, status| theme::ghost_button_style(status));
    if active {
        b = b.on_press(Message::Import(msg));
    }
    b.into()
}

/// Neutral stat chip (LINE_2 fill, TEXT_2 label) used in the summary band
/// and the per-row channel chip.
fn stat_chip<'a>(label: String) -> Element<'a, Message> {
    container(text(label).size(11).color(theme::TEXT_2))
        .padding([2, 8])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::LINE)),
            border: iced::Border {
                color: theme::LINE,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

/// Accent-tinted chip — the SMF-format badge in the summary band.
fn accent_chip<'a>(label: String) -> Element<'a, Message> {
    container(
        text(label)
            .size(11)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(theme::ACCENT_SOFT),
    )
    .padding([2, 8])
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::ACCENT_DIM)),
        border: iced::Border {
            color: theme::ACCENT_LINE,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    })
    .into()
}

/// Row background: an `ACCENT_DIM` wash + accent border when selected, a
/// quiet `LINE_2` fill for unselected/conductor rows.
fn row_style(selected: bool, conductor: bool) -> container::Style {
    let (bg, border) = if selected {
        (theme::ACCENT_DIM, theme::ACCENT_LINE)
    } else if conductor {
        (alpha(theme::LINE_2, 0.6), theme::LINE)
    } else {
        (theme::LINE_2, theme::LINE)
    };
    container::Style {
        background: Some(iced::Background::Color(bg)),
        border: iced::Border {
            color: border,
            width: 1.0,
            radius: theme::RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

/// Inline rename input style: blends into the row, accent caret/selection.
fn rename_input_style(_theme: &iced::Theme, status: text_input::Status) -> text_input::Style {
    let border_color = match status {
        text_input::Status::Focused { .. } => theme::ACCENT_LINE,
        _ => iced::Color::TRANSPARENT,
    };
    text_input::Style {
        background: iced::Background::Color(iced::Color::TRANSPARENT),
        border: iced::Border {
            color: border_color,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        icon: theme::TEXT_2,
        placeholder: theme::TEXT_3,
        value: theme::TEXT_1,
        selection: alpha(theme::ACCENT, 0.35),
    }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

/// "12 notes · C3–C5" — note count plus the pitch range. The range is
/// omitted when the track carries no pitched notes.
fn note_readout(r: &TrackImportRow) -> String {
    let count = plural(r.note_count, "note", "notes");
    match (r.pitch_min, r.pitch_max) {
        (Some(lo), Some(hi)) => format!(
            "{count} \u{00b7} {}\u{2013}{}",
            resonance_music_theory::midi_note_name(lo),
            resonance_music_theory::midi_note_name(hi),
        ),
        _ => count,
    }
}

/// Tempo summary label: a single "140 BPM", a "120–140 BPM" range when the
/// file has tempo changes, or `None` when no tempo is known.
fn tempo_range_label(s: &ImportSummary) -> Option<String> {
    match (s.tempo_bpm_min, s.tempo_bpm_max) {
        (Some(lo), Some(hi)) if (hi - lo).abs() >= 0.5 => {
            Some(format!("{}\u{2013}{} BPM", round_bpm(lo), round_bpm(hi)))
        }
        (Some(bpm), _) | (_, Some(bpm)) => Some(format!("{} BPM", round_bpm(bpm))),
        _ => s.file_tempo_bpm.map(|bpm| format!("{} BPM", round_bpm(bpm))),
    }
}

fn round_bpm(bpm: f32) -> i32 {
    bpm.round() as i32
}

/// `color` with its alpha replaced — for the tinted swatch fills/borders
/// and the row washes.
fn alpha(color: iced::Color, a: f32) -> iced::Color {
    iced::Color { a, ..color }
}

/// `"1 track"` / `"5 tracks"` — count plus a singular/plural noun.
fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}
