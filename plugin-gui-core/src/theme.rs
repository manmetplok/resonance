//! The shared editor palette for Resonance plugin UIs.
//!
//! [`lavender`] is the canonical Resonance palette (see
//! `ux-guidelines.md`): quiet blue-grey surfaces with the lavender brand
//! accent. Token names (`BG_0`–`BG_3`, `LINE`, `TEXT_1`–`TEXT_4`,
//! `ACCENT`, `WARM`, `GOOD`, `BAD`) and hex values match the main app's
//! `theme.rs`. **All eleven plugin editors are on it** — ba todo #1338
//! moved the last seven (amp, compressor, delay, EQ, IR, mastering,
//! reverb) off the older blue-accent `classic` palette, which is gone.
//! There is no second palette to choose between any more, and
//! `resonance-plugin/tests/fleet_palette.rs` fails the build if one
//! reappears.
//!
//! The module exposes its colour constants plus an [`lavender::apply`]
//! that installs matching `egui::Visuals`. Genuinely plugin-specific
//! colours (scope traces, tuner zones, …) stay in each plugin's own
//! `editor/theme.rs`, which re-exports this module and adds its extras —
//! but they are shaped from these tokens, not picked freehand.

/// Canonical Resonance palette (lavender accent) — the whole fleet.
/// Mirrors the main app's `theme.rs` tokens.
pub mod lavender {
    // ---------- Surfaces ----------
    pub const BG_0: egui::Color32 = egui::Color32::from_rgb(0x0a, 0x0b, 0x0e);
    pub const BG_1: egui::Color32 = egui::Color32::from_rgb(0x15, 0x16, 0x1b);
    pub const BG_2: egui::Color32 = egui::Color32::from_rgb(0x1b, 0x1d, 0x23);
    pub const BG_3: egui::Color32 = egui::Color32::from_rgb(0x23, 0x26, 0x2e);

    pub const LINE: egui::Color32 = egui::Color32::from_rgb(0x27, 0x2a, 0x31);
    pub const LINE_2: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x22, 0x29);

    // ---------- Text ----------
    pub const TEXT_1: egui::Color32 = egui::Color32::from_rgb(0xe8, 0xe7, 0xe3);
    pub const TEXT_2: egui::Color32 = egui::Color32::from_rgb(0x9a, 0xa0, 0xac);
    pub const TEXT_3: egui::Color32 = egui::Color32::from_rgb(0x5d, 0x62, 0x6d);
    pub const TEXT_4: egui::Color32 = egui::Color32::from_rgb(0x3f, 0x43, 0x4c);

    // ---------- Accents ----------
    pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x8b, 0x6d, 0xff);
    pub const ACCENT_SOFT: egui::Color32 = egui::Color32::from_rgb(0xa8, 0x92, 0xff);
    // Hand-premultiplied: scale RGB by (alpha / 255).
    // ACCENT_DIM: alpha 0x28 = 40 → 40/255 ≈ 0.157 → (22, 17, 40, 40).
    pub const ACCENT_DIM: egui::Color32 = egui::Color32::from_rgba_premultiplied(22, 17, 40, 40);
    // ACCENT_GLOW: alpha 0x40 = 64 → 64/255 ≈ 0.251 → (35, 27, 64, 64).
    // The heavier of the two accent tints — a glow under a curve or a
    // selection fill, where ACCENT_DIM would disappear.
    pub const ACCENT_GLOW: egui::Color32 = egui::Color32::from_rgba_premultiplied(35, 27, 64, 64);

    pub const WARM: egui::Color32 = egui::Color32::from_rgb(0xe8, 0xc4, 0x7b);

    pub const GOOD: egui::Color32 = egui::Color32::from_rgb(0x6d, 0xd6, 0xa3);
    pub const BAD: egui::Color32 = egui::Color32::from_rgb(0xe8, 0x7b, 0x8b);

    // ---------- Legacy token names ----------
    // The names the retired `classic` palette used, aliased onto the
    // token that carries the same *meaning* here. They exist so ba todo
    // #1338 could repoint seven editors at this palette without rewriting
    // ~250 call sites in crates that have work in review; new code should
    // use the canonical name on the right.
    pub const BG: egui::Color32 = BG_0;
    pub const PANEL: egui::Color32 = BG_2;
    pub const PANEL_LIGHT: egui::Color32 = BG_3;
    pub const BORDER: egui::Color32 = LINE;
    pub const TEXT: egui::Color32 = TEXT_1;
    pub const TEXT_DIM: egui::Color32 = TEXT_3;
    pub const WARN: egui::Color32 = WARM;
    pub const DANGER: egui::Color32 = BAD;

    // ---------- Shape tokens ----------
    pub const RADIUS_PANEL: f32 = 9.0;
    pub const RADIUS_CHIP: f32 = 5.0;

    pub fn apply(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = BG_2;
        visuals.panel_fill = BG_0;
        visuals.override_text_color = Some(TEXT_1);
        visuals.faint_bg_color = BG_2;
        visuals.extreme_bg_color = BG_1;
        visuals.widgets.noninteractive.bg_fill = BG_2;
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_3);
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, LINE_2);
        visuals.widgets.inactive.bg_fill = BG_3;
        visuals.widgets.inactive.weak_bg_fill = BG_3;
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_1);
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, LINE);
        visuals.widgets.hovered.bg_fill = BG_3;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT);
        visuals.widgets.active.bg_fill = BG_3;
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5, ACCENT);
        visuals.widgets.open.bg_fill = BG_3;
        visuals.selection.bg_fill = ACCENT_DIM;
        visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
        ctx.set_visuals(visuals);
    }

    /// [`apply`], but only the first time it runs for this `Context` —
    /// every editor used to call `apply` on every frame (PUX-04).
    /// `set_visuals` is a full style rebuild, not a cheap no-op, and the
    /// palette never changes while an editor is open, so repeating it
    /// per frame was pure waste next to the audio work. Call this from
    /// `EditorApp::ui` instead of `apply` directly; it has no effect
    /// after the first call for a given `Context` (i.e. for the life of
    /// one editor window).
    pub fn apply_once(ctx: &egui::Context) {
        let id = egui::Id::new("resonance_lavender_theme_applied");
        let already = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
        if !already {
            apply(ctx);
            ctx.data_mut(|d| d.insert_temp(id, true));
        }
    }
}
