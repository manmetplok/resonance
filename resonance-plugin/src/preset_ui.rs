//! The shared preset bar for plugin editors: one picker listing factory
//! and user presets together, plus Save / Rename / Delete.
//!
//! Every plugin editor draws its own preset combo today, four of them
//! hardcoding "— select —" and none of them able to save (audit findings
//! X1 and X2). This is the one widget they move onto. It is deliberately
//! a thin skin: all the behaviour lives in
//! [`PresetEditor`](crate::presets::PresetEditor) and
//! [`PresetSession`](crate::presets::PresetSession), which are
//! GUI-agnostic and unit-tested, so this file only turns clicks into
//! their method calls.
//!
//! ```ignore
//! let event = preset_ui::preset_bar(
//!     ui, "gate_presets", &mut self.preset_editor, &self.bank,
//!     &self.presets, &self.params.refs(), "— preset —",
//! );
//! if matches!(event, PresetEvent::Loaded(_)) {
//!     self.push_all_params_to_host();
//! }
//! ```
//!
//! Feature-gated behind `editor-widgets` with the rest of the egui
//! helpers, so DSP-only consumers don't pull in the GUI stack.

use plugin_gui_core::egui;

use crate::param::Param;
use crate::presets::{
    NamingKind, PresetBank, PresetEditor, PresetEvent, PresetRecord, PresetRef, PresetSession,
    PresetSource,
};

/// How often the bar re-checks the user preset directory; see
/// [`crate::presets::BAR_REFRESH`].
pub use crate::presets::BAR_REFRESH;

/// Draw the preset bar. Returns what the user did, if anything.
pub fn preset_bar(
    ui: &mut egui::Ui,
    id_salt: &str,
    editor: &mut PresetEditor,
    bank: &PresetBank,
    session: &PresetSession,
    params: &[&dyn Param],
    placeholder: &str,
) -> PresetEvent {
    let mut event = PresetEvent::None;

    ui.horizontal(|ui| {
        if editor.naming().is_some() {
            event = name_entry_row(ui, editor, bank, session, params);
            return;
        }

        // A project saved before preset ids carries a name-only identity;
        // give it its id now that a bank is at hand (free once resolved).
        session.resolve(bank);
        let current = session.current();
        let records = bank.records_cached(BAR_REFRESH);

        // Step through the list without opening the combo — the fastest
        // way to audition a bank. Wavetable had these as a private fork
        // around its own combo; ba todo #1280 folds them in here so the
        // whole fleet gets them. Disabled at the ends, since stepping
        // clamps rather than wraps.
        let all = &records;
        let at = current
            .as_ref()
            .and_then(|c| all.iter().position(|r| r.preset.matches(c)));
        let can_prev = !all.is_empty() && at.map(|i| i > 0).unwrap_or(true);
        let can_next = !all.is_empty() && at.map(|i| i + 1 < all.len()).unwrap_or(true);
        if ui
            .add_enabled(can_prev, egui::Button::new("◀").small().frame(false))
            .on_hover_text("Previous preset")
            .clicked()
        {
            event = editor.step(bank, session, -1, params);
        }

        let picked = picker(ui, id_salt, editor, bank, session, params, placeholder, all);
        if !matches!(picked, PresetEvent::None) {
            event = picked;
        }

        if ui
            .add_enabled(can_next, egui::Button::new("▶").small().frame(false))
            .on_hover_text("Next preset")
            .clicked()
        {
            event = editor.step(bank, session, 1, params);
        }

        // Save is always available: it is how a user preset comes into
        // existence. Rename and Delete apply to user presets only —
        // the factory bank is read-only.
        let is_user = current
            .as_ref()
            .map(|p| p.source == PresetSource::User)
            .unwrap_or(false);

        if ui.small_button("Save").clicked() {
            editor.begin_save(session);
        }
        if ui
            .add_enabled(is_user, egui::Button::new("Rename").small())
            .clicked()
        {
            if let Some(p) = &current {
                editor.begin_rename(p);
            }
        }
        if ui
            .add_enabled(is_user, egui::Button::new("Delete").small())
            .clicked()
        {
            if let Some(p) = &current {
                event = editor.delete(bank, session, p);
            }
        }

        if session.is_modified() {
            ui.label(egui::RichText::new("•").weak())
                .on_hover_text("Edited since the preset was loaded");
        }
    });

    if let Some(err) = editor.error() {
        ui.label(egui::RichText::new(err).color(egui::Color32::from_rgb(0xd0, 0x60, 0x60)));
    }

    event
}

/// The combo itself: factory bank first, then the user's own presets
/// under their own heading, so the two sets are never confused.
#[allow(clippy::too_many_arguments)]
fn picker(
    ui: &mut egui::Ui,
    id_salt: &str,
    editor: &mut PresetEditor,
    bank: &PresetBank,
    session: &PresetSession,
    params: &[&dyn Param],
    placeholder: &str,
    records: &[PresetRecord],
) -> PresetEvent {
    let mut event = PresetEvent::None;
    let current = session.current();
    let factory = || records.iter().filter(|r| r.preset.source == PresetSource::Factory);
    let user = || records.iter().filter(|r| r.preset.source == PresetSource::User);

    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(session.label(placeholder))
        .show_ui(ui, |ui| {
            if factory().next().is_some() {
                ui.label(egui::RichText::new("Factory").weak().small());
            }
            for record in factory() {
                if selectable(ui, &current, &record.preset) {
                    event = editor.pick(bank, session, &record.preset, params);
                }
            }
            if user().next().is_some() {
                ui.separator();
                ui.label(egui::RichText::new("User").weak().small());
                for record in user() {
                    if selectable(ui, &current, &record.preset) {
                        event = editor.pick(bank, session, &record.preset, params);
                    }
                }
            }
        });

    event
}

fn selectable(ui: &mut egui::Ui, current: &Option<PresetRef>, preset: &PresetRef) -> bool {
    let selected = current.as_ref().is_some_and(|c| c.matches(preset));
    ui.selectable_label(selected, preset.name.as_str()).clicked()
}

/// The inline name field shown while saving or renaming, in place of the
/// buttons.
fn name_entry_row(
    ui: &mut egui::Ui,
    editor: &mut PresetEditor,
    bank: &PresetBank,
    session: &PresetSession,
    params: &[&dyn Param],
) -> PresetEvent {
    ui.label(match editor.naming() {
        Some(NamingKind::Rename) => "Rename",
        _ => "Save as",
    });

    let mut submitted = false;
    if let Some(buf) = editor.name_buffer() {
        let response = ui.add(
            egui::TextEdit::singleline(buf)
                .desired_width(140.0)
                .hint_text("Preset name"),
        );
        response.request_focus();
        submitted = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    }
    submitted |= ui.small_button("OK").clicked();
    let cancelled =
        ui.small_button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));

    if submitted {
        editor.submit(bank, session, params)
    } else {
        if cancelled {
            editor.cancel();
        }
        PresetEvent::None
    }
}
