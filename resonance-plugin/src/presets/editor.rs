//! The editor-side state machine behind the preset bar.

use super::bank::PresetBank;
use super::session::PresetSession;
use super::{PresetRef, PresetSource};
use crate::param::Param;

/// What the user did in a preset bar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PresetEvent {
    #[default]
    None,
    /// A preset was applied — every parameter may have moved, so the
    /// editor should push the whole surface to the host.
    Loaded(PresetRef),
    Saved(PresetRef),
    Renamed(PresetRef),
    Deleted(PresetRef),
}

/// Which name is being typed, when one is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamingKind {
    SaveAs,
    Rename,
}

/// The transient state of a preset bar: an in-progress name entry and
/// the last error. Deliberately free of any GUI toolkit — the egui
/// rendering in [`crate::preset_ui`] is a thin skin over this, so the
/// save / rename / delete behaviour is testable without a window.
#[derive(Default)]
pub struct PresetEditor {
    naming: Option<Naming>,
    error: Option<String>,
}

#[derive(Clone)]
struct Naming {
    kind: NamingKind,
    /// The preset being renamed; `None` for a save.
    target: Option<PresetRef>,
    buf: String,
}

impl PresetEditor {
    /// The message to show under the bar, if the last action failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether a name is being typed (editors suppress their own
    /// keyboard shortcuts while it is).
    pub fn naming(&self) -> Option<NamingKind> {
        self.naming.as_ref().map(|n| n.kind)
    }

    /// The name being typed, for the text field to edit in place.
    pub fn name_buffer(&mut self) -> Option<&mut String> {
        self.naming.as_mut().map(|n| &mut n.buf)
    }

    /// Start a "save as", seeded from the loaded preset: a user preset
    /// offers to overwrite itself, a factory preset offers a copy (so
    /// "Save" over "Vocal — Noise Gate" never looks like it will
    /// overwrite the factory bank).
    pub fn begin_save(&mut self, session: &PresetSession) {
        let initial = match session.current() {
            Some(p) if p.source == PresetSource::User => p.name,
            Some(p) => format!("{} (edit)", p.name),
            None => "My Preset".to_string(),
        };
        self.error = None;
        self.naming = Some(Naming {
            kind: NamingKind::SaveAs,
            target: None,
            buf: initial,
        });
    }

    /// Start renaming a user preset.
    pub fn begin_rename(&mut self, preset: &PresetRef) {
        self.error = None;
        self.naming = Some(Naming {
            kind: NamingKind::Rename,
            target: Some(preset.clone()),
            buf: preset.name.clone(),
        });
    }

    /// Abandon the name entry.
    pub fn cancel(&mut self) {
        self.naming = None;
        self.error = None;
    }

    /// Commit the typed name. On failure the field stays open with the
    /// offending name still in it and [`error`](Self::error) set, so the
    /// user can correct it instead of losing what they typed.
    pub fn submit(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        params: &[&dyn Param],
    ) -> PresetEvent {
        let Some(naming) = self.naming.clone() else {
            return PresetEvent::None;
        };
        let result = match naming.kind {
            NamingKind::SaveAs => session
                .save_as(bank, &naming.buf, params)
                .map(PresetEvent::Saved),
            NamingKind::Rename => match &naming.target {
                Some(target) => session
                    .rename(bank, target, &naming.buf)
                    .map(PresetEvent::Renamed),
                None => Err("Nothing to rename".to_string()),
            },
        };
        match result {
            Ok(event) => {
                self.naming = None;
                self.error = None;
                event
            }
            Err(e) => {
                self.error = Some(e);
                PresetEvent::None
            }
        }
    }

    /// Load a preset picked from the list.
    pub fn pick(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        preset: &PresetRef,
        params: &[&dyn Param],
    ) -> PresetEvent {
        if session.load_preset(bank, preset, params) {
            self.error = None;
            PresetEvent::Loaded(session.current().unwrap_or_else(|| preset.clone()))
        } else {
            self.error = Some(format!("Preset '{}' could not be loaded", preset.name));
            PresetEvent::None
        }
    }

    /// Move `delta` places through the merged factory+user list and load
    /// what lands, for the bar's ◀ / ▶ buttons (ba todo #1280).
    ///
    /// Stepping is clamped, not wrapping: running off the end of a list
    /// and silently reappearing at the other end is disorienting when
    /// you are listening rather than looking. With nothing loaded, a
    /// step forwards starts at the first preset and a step backwards at
    /// the last, so either button gets a user into the list.
    pub fn step(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        delta: i32,
        params: &[&dyn Param],
    ) -> PresetEvent {
        let all = bank.list();
        if all.is_empty() {
            return PresetEvent::None;
        }
        let target = match session.current() {
            Some(current) => match all.iter().position(|p| p.matches(&current)) {
                Some(index) => {
                    let next = index as i32 + delta;
                    if next < 0 || next >= all.len() as i32 {
                        return PresetEvent::None;
                    }
                    next as usize
                }
                // Loaded preset is not in the list any more (deleted
                // outside the app): treat the step as an entry point.
                None => 0,
            },
            None if delta >= 0 => 0,
            None => all.len() - 1,
        };
        let preset = all[target].clone();
        self.pick(bank, session, &preset, params)
    }

    /// Delete the loaded user preset.
    pub fn delete(
        &mut self,
        bank: &PresetBank,
        session: &PresetSession,
        preset: &PresetRef,
    ) -> PresetEvent {
        match session.delete(bank, preset) {
            Ok(()) => {
                self.error = None;
                PresetEvent::Deleted(preset.clone())
            }
            Err(e) => {
                self.error = Some(e);
                PresetEvent::None
            }
        }
    }
}
