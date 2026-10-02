/// View rendering for the Resonance application. The top-level dispatch
/// lives here; concrete surfaces are in sibling modules (transport,
/// mixer, compose, track_header, menus, settings, editor_panel,
/// timeline_panel, timeline, piano_roll, midi_editor).
pub mod arrange_layout;
pub(crate) mod bounce_dialog;
pub(crate) mod browser;
pub(crate) mod clip_inspector;
pub(crate) mod export_dialog;
pub(crate) mod bounce_progress;
pub mod compose;
pub(crate) mod confirm_delete_track;
pub(crate) mod confirm_quit;
pub mod controls;
pub(crate) mod editor_panel;
pub mod freeze_banner;
pub(crate) mod import_dialog;
pub(crate) mod import_progress_dialog;
pub mod knob;
pub(crate) mod markers_overview;
pub(crate) mod menus;
pub mod midi_editor;
pub(crate) mod midi_quantize;
pub(crate) mod mixer;
pub(crate) mod missing_plugins_dialog;
pub(crate) mod palette;
pub(crate) mod plugin_window;
pub(crate) mod preset_browser;
pub(crate) mod recovery_prompt;
pub(crate) mod relink_dialog;
pub(crate) mod remote_indicator;
pub mod performance;
pub mod piano_roll;
pub(crate) mod selection_bar;
pub(crate) mod shortcut_hint;
pub(crate) mod settings;
pub(crate) mod settings_keyboard;
pub(crate) mod startup;
pub mod timeline;
pub(crate) mod timeline_panel;
pub(crate) mod track_header;
pub(crate) mod transport;
pub(crate) mod transport_labels;
pub(crate) mod ui_caches;

// Surgical re-export of the pure tiling-ribbon span builder so integration
// tests can drive it without widening the whole crate-private `compose`
// view tree. See `compose/drumroll/ribbon.rs`.
pub use compose::drumroll::ribbon::{build_ribbon_spans, RibbonSpan, RibbonSpanKind};

use crate::message::*;
use crate::state::*;
use crate::theme;
use iced::widget::{button, column, container, row, stack, text, Space};
use iced::{alignment, Element, Length};

impl crate::Resonance {
    pub fn view(&self) -> Element<'_, Message> {
        let base = self.view_base();
        // The generic plugin window floats over the base view and under
        // every root overlay: it is non-modal, and a modal opened from it
        // (the preset browser) belongs on top (mixer-cleanup.md §4). The
        // layer is always there so opening or closing the window never
        // changes the tree's shape (which would reset the strips' scroll
        // offsets under it).
        //
        // The Arrange track menu (and its "Save as preset…" prompt) is the
        // one overlay drawn inside the base view rather than in the root
        // stack, so the window would land on top of it; while it is up
        // the window is not drawn at all.
        let track_menu_up = self.ui.interaction.track_menu.is_some()
            || self.ui.interaction.preset_save.is_some();
        let window: Element<'_, Message> = if track_menu_up {
            None
        } else {
            self.view_plugin_window()
        }
        .unwrap_or_else(|| Space::new().into());
        let base: Element<'_, Message> = stack![base, window].into();
        let root: Element<'_, Message> = match self.view_root_overlay() {
            Some(overlay) => stack![base, overlay].into(),
            None => base,
        };
        // A preset drag from the media browser (slice P8) follows the
        // pointer and ends on any release. The wrapper is always there so
        // arming a drag never changes the tree's shape (which would reset
        // every scroll offset and focus); it only listens while armed.
        let area = iced::widget::mouse_area(root);
        if self.presets.dragging.is_some() {
            let end = || Message::Plugin(PluginMessage::PresetUi(PresetUiMessage::DragEnd));
            // Any release ends it (after a header's drop); so does a press
            // no row took (the release happened outside the window) and
            // the pointer leaving the window.
            area.on_move(|at| {
                Message::Plugin(PluginMessage::PresetUi(PresetUiMessage::DragMoved(at)))
            })
            .on_release(end())
            .on_press(end())
            .on_exit(end())
            .into()
        } else if self.ui.mixer.plugin_window.is_some_and(|w| w.drag.is_some()) {
            // A generic plugin window's title-bar drag follows the pointer
            // in window coordinates until the button comes up.
            let drag = |step| Message::Plugin(PluginMessage::PluginWindowDrag(step));
            area.on_move(move |at| drag(PluginWindowDrag::Moved(at)))
                .on_release(drag(PluginWindowDrag::End))
                .on_exit(drag(PluginWindowDrag::End))
                .into()
        } else {
            area.into()
        }
    }

    /// The window content under any root overlay.
    fn view_base(&self) -> Element<'_, Message> {
        // Performance mode is a full-bleed, distraction-free surface: it
        // owns its own status bar / footer and intentionally hides the
        // normal transport chrome below.
        if matches!(self.ui.view_mode, ViewMode::Performance) {
            return self.view_performance_shell();
        }

        let transport = transport::view_transport(self);
        let main_area = match self.ui.view_mode {
            ViewMode::Arrange => self.view_main_area(),
            ViewMode::Mixer => self.view_mixer(),
            ViewMode::Compose => self.view_compose(),
            // Unreachable: Performance returns early above via
            // `view_performance_shell`; kept so the match stays exhaustive.
            ViewMode::Performance => self.view_performance_shell(),
        };

        // The status area is always a child, empty or not: a slot that
        // came and went moved `main_area` between child indices, and iced
        // then rebuilt its whole widget tree — every scroll offset, canvas
        // key focus and in-progress drag lost to a failed preset star
        // (code review UX-05).
        let content: Element<'_, Message> =
            column![transport, self.view_status_area(), main_area].spacing(0).into();

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::base_bg)
            .into()
    }

    /// The status lines between the transport and the main area, worst
    /// first: the engine's health (UX-04), a run of autosave failures
    /// (UX-13), then the transient error banner. The first two are
    /// statuses — derived from state, cleared when the condition clears,
    /// never overwritten by an error — so only the banner has a dismiss
    /// button. Renders as a zero-height column when all is well.
    fn view_status_area(&self) -> Element<'_, Message> {
        let mut lines = column![].spacing(0).width(Length::Fill);
        if let Some(status) = self.banners.engine_health.message() {
            lines = lines.push(status_line("AUDIO", status, theme::BAD, theme::BAD_DIM));
        }
        if let Some(status) = self.banners.autosave_failing() {
            lines = lines.push(status_line("AUTOSAVE", status, theme::WARM, theme::WARM_DIM));
        }
        if let Some(ref err) = self.banners.error_message {
            lines = lines.push(error_bar(err));
        }
        lines.into()
    }

    /// The one window-root overlay, chosen by [`Resonance::root_overlay`] —
    /// the same priority order the keyboard gate reads, so what the user
    /// sees is what the shortcuts are gated on (command-palette.md §3.2).
    fn view_root_overlay(&self) -> Option<Element<'_, Message>> {
        let overlay: Element<'_, Message> = match self.root_overlay()? {
            Overlay::Recovery => {
                // The autosave-recovery prompt (FU-M12a) answers an open, so
                // it sits over the startup screen when nothing is open yet.
                let prompt = self
                    .io
                    .recovery_prompt
                    .as_ref()
                    .map(recovery_prompt::view_recovery_prompt_overlay)?;
                if self.io.has_active_project {
                    prompt
                } else {
                    stack![startup::view_startup_overlay(self), prompt].into()
                }
            }
            Overlay::Startup => startup::view_startup_overlay(self),
            // The bounce progress modal sits above any other overlay
            // because it gates user input until the engine finishes the
            // current bounce — letting the quit-confirm or delete-track
            // dialog appear over it would invite the user into a state
            // change the engine isn't ready for.
            Overlay::BounceProgress => bounce_progress::view_bounce_progress_overlay(self),
            // The WAV mixdown gates the same traffic as a bounce in place
            // (`gates_message`), so it gets the same blocking modal at the
            // same priority (code review FU-F1c).
            Overlay::MixdownProgress => bounce_progress::view_mixdown_progress_overlay(self),
            // The freeze progress modal (design doc #181, todo #582) is the
            // same blocking overlay — a freeze IS a bounce-in-place run.
            Overlay::FreezeProgress => bounce_progress::view_freeze_progress_overlay(self),
            Overlay::Palette => palette::view_palette_overlay(self),
            Overlay::ConfirmQuit => confirm_quit::view_confirm_quit_overlay(self),
            Overlay::ConfirmProjectSwitch => {
                let switch = self.modals.confirm_switch.as_ref()?;
                confirm_quit::view_confirm_switch_overlay(self, switch)
            }
            Overlay::ConfirmDeleteTrack => {
                let track_id = self.modals.confirm_delete_track?;
                confirm_delete_track::view_confirm_delete_track_overlay(self, track_id)
            }
            Overlay::BounceDialog => bounce_dialog::view_bounce_dialog_overlay(self),
            Overlay::ExportDialog => export_dialog::view_export_dialog_overlay(self),
            Overlay::ImportDialog => import_dialog::view_import_dialog_overlay(self),
            // Audio-import transcode-progress modal (doc #175, todo #606).
            Overlay::ImportProgress => import_progress_dialog::view_import_progress_overlay(self),
            // Missing-plugin load warning (ba doc #275 P5, todo #1309). Sits
            // ABOVE the relink modal only because one modal shows at a time;
            // the two are independent and a project can trip both.
            Overlay::MissingPlugins => missing_plugins_dialog::view_missing_plugins_overlay(self),
            // Missing-files relink modal (doc #175, todo #607).
            Overlay::Relink => relink_dialog::view_relink_dialog_overlay(self),
            Overlay::Settings => settings::view_settings_overlay(self),
            Overlay::PresetBrowser => preset_browser::view_preset_browser_overlay(self),
            Overlay::AddTrackMenu => menus::view_add_track_menu(self),
            Overlay::MarkersOverview => markers_overview::view_markers_overview_overlay(self),
            Overlay::DrumGroupsManager => compose::drum_groups_manager::view(self),
            // Arrangement-marker context menu / inline rename float above
            // the arrange timeline (todo #369).
            Overlay::MarkerMenu => menus::view_marker_overlay(self),
            // Drawn by `view_main_area` in arrange-area space.
            Overlay::TrackMenu => return None,
            // Floating "Group selected" bar — non-modal, so it layers over
            // the arrange view without blocking it (todo #684).
            Overlay::SelectionBar => selection_bar::view_selection_bar(self),
        };
        Some(overlay)
    }

    fn view_main_area(&self) -> Element<'_, Message> {
        let track_headers = track_header::view_track_headers(self);
        let timeline = self.view_timeline();

        // The docked media browser (design doc #175) sits flush against the
        // left edge as a peer of the track headers + timeline, so browsing
        // and auditioning never obscures the arrangement. Hidden by default;
        // toggled from the "Media" chrome button / the panel's collapse caret.
        let main = if self.media.browser.visible {
            row![
                browser::view_browser_panel(self),
                track_headers,
                timeline
            ]
        } else {
            row![track_headers, timeline]
        };

        let base: Element<'_, Message> = if let Some(editor) = self.view_midi_editor_panel() {
            column![
                container(main).width(Length::Fill).height(Length::Fill),
                editor,
            ]
            .spacing(0)
            .into()
        } else {
            container(main)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        };

        // The clip fade/gain inspector floats over the top-right of the
        // arrange area for the selected editable audio clip (epic #18).
        let base: Element<'_, Message> = if let Some(flyout) = self.view_clip_inspector_flyout() {
            let overlay = container(flyout)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(alignment::Horizontal::Right)
                .align_y(alignment::Vertical::Top)
                .padding(10);
            stack![base, overlay].into()
        } else {
            base
        };

        // The right-click track context menu (design doc #181, todo #581)
        // floats over the arrange main area. It stacks *here* — not in the
        // window-root overlay chain — because its anchor coordinates are
        // computed in arrange-area space from the track-header row layout
        // (a widget `mouse_area` press carries no cursor position), so the
        // overlay must share that coordinate origin. The "Save as
        // preset…" name prompt is drawn by the same overlay and outlives
        // the menu that opened it, so it keeps the stack alive on its own
        // (review VIEW-07).
        if self.ui.interaction.track_menu.is_some() || self.ui.interaction.preset_save.is_some() {
            stack![base, menus::view_track_menu_overlay(self)].into()
        } else {
            base
        }
    }
}

/// One persistent status line: a tag chip and the status text, on a
/// tinted band. No dismiss button — it clears when the condition does.
fn status_line<'a>(
    tag: &'static str,
    status: impl text::IntoFragment<'a>,
    tone: iced::Color,
    wash: iced::Color,
) -> Element<'a, Message> {
    let chip = container(text(tag).size(11).color(tone))
        .padding([1, 6])
        .style(move |_theme| container::Style {
            border: iced::Border {
                color: tone,
                width: 1.0,
                radius: 3.0.into(),
            },
            ..Default::default()
        });
    container(
        row![chip, text(status).size(13).color(theme::TEXT)]
            .spacing(8)
            .align_y(alignment::Vertical::Center)
            .padding([6, 8]),
    )
    .width(Length::Fill)
    .style(move |_theme| container::Style {
        background: Some(iced::Background::Color(wash)),
        ..Default::default()
    })
    .into()
}

/// The transient, dismissable error banner.
fn error_bar(err: &str) -> Element<'_, Message> {
    container(
        row![
            text(err).size(13).color(iced::Color::WHITE),
            Space::new().width(Length::Fill),
            button(text("\u{00d7}").size(14).color(iced::Color::WHITE))
                .on_press(Message::Ui(UiMessage::DismissError))
                .style(|_theme, _status| iced::widget::button::Style {
                    background: Some(iced::Background::Color(iced::Color::TRANSPARENT)),
                    text_color: iced::Color::WHITE,
                    ..Default::default()
                })
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center)
        .padding(8),
    )
    .width(Length::Fill)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::RECORD_RED)),
        ..Default::default()
    })
    .into()
}
