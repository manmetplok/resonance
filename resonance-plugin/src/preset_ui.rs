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

use wayland_plugin_gui::egui;

use crate::param::Param;
use crate::presets::{
    NamingKind, PresetBank, PresetEditor, PresetEvent, PresetRef, PresetSession, PresetSource,
};

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

        let current = session.current();
        event = picker(ui, id_salt, editor, bank, session, params, placeholder);

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
fn picker(
    ui: &mut egui::Ui,
    id_salt: &str,
    editor: &mut PresetEditor,
    bank: &PresetBank,
    session: &PresetSession,
    params: &[&dyn Param],
    placeholder: &str,
) -> PresetEvent {
    let mut event = PresetEvent::None;
    let current = session.current();

    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(session.label(placeholder))
        .show_ui(ui, |ui| {
            if !bank.factory().is_empty() {
                ui.label(egui::RichText::new("Factory").weak().small());
            }
            for entry in bank.factory() {
                let preset = PresetRef::factory(entry.name);
                if selectable(ui, &current, &preset) {
                    event = editor.pick(bank, session, &preset, params);
                }
            }
            let user = bank.list_user();
            if !user.is_empty() {
                ui.separator();
                ui.label(egui::RichText::new("User").weak().small());
                for preset in user {
                    if selectable(ui, &current, &preset) {
                        event = editor.pick(bank, session, &preset, params);
                    }
                }
            }
        });

    event
}

fn selectable(ui: &mut egui::Ui, current: &Option<PresetRef>, preset: &PresetRef) -> bool {
    let selected = current.as_ref() == Some(preset);
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
