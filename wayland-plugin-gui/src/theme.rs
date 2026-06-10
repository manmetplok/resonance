//! Shared editor palettes for Resonance plugin UIs.
//!
//! Two palettes are in use across the plugin fleet:
//!
//! - [`lavender`] — the canonical Resonance palette (see
//!   `ux-guidelines.md`): quiet blue-grey surfaces with the lavender
//!   brand accent. Token names (`BG_0`–`BG_3`, `LINE`, `TEXT_1`–`TEXT_4`,
//!   `ACCENT`, `WARM`, `GOOD`, `BAD`) and hex values match the main
//!   app's `theme.rs`. New editors should use this palette.
//! - [`classic`] — the older blue-accent (`#5ac8fa`) palette the effect
//!   editors shipped with. Kept byte-identical so existing editors don't
//!   shift visually; it will migrate to `lavender` over time.
//!
//! Each module exposes its colour constants plus an `apply()` that
//! installs matching `egui::Visuals`. Genuinely plugin-specific colours
//! (scope traces, tuner zones, …) stay in each plugin's own `theme.rs`,
//! which re-exports one of these modules and adds its extras.

/// Older blue-accent palette used by the effect editors (amp, compressor,
/// delay, EQ, IR, mastering, reverb).
pub mod classic {
    pub const BG: egui::Color32 = egui::Color32::from_rgb(0x14, 0x14, 0x18);
    pub const PANEL: egui::Color32 = egui::Color32::from_rgb(0x1b, 0x1b, 0x22);
    pub const PANEL_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x25, 0x25, 0x2e);
    pub const BORDER: egui::Color32 = egui::Color32::from_rgb(0x33, 0x33, 0x3e);
    pub const TEXT: egui::Color32 = egui::Color32::from_rgb(0xe0, 0xe0, 0xe0);
    pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0x80, 0x80, 0x88);
    pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x5a, 0xc8, 0xfa);
    pub const ACCENT_DIM: egui::Color32 =
        egui::Color32::from_rgba_premultiplied(0x5a, 0xc8, 0xfa, 0x60);
    pub const ACCENT_GLOW: egui::Color32 =
        egui::Color32::from_rgba_premultiplied(0x5a, 0xc8, 0xfa, 0x40);
    pub const WARN: egui::Color32 = egui::Color32::from_rgb(0xff, 0xb6, 0x4a);
    pub const DANGER: egui::Color32 = egui::Color32::from_rgb(0xff, 0x6a, 0x6a);

    /// Install the standard dark visuals with `ACCENT_GLOW` selection.
    pub fn apply(ctx: &egui::Context) {
        apply_with_selection(ctx, ACCENT_GLOW);
    }

    /// Install the standard dark visuals; `selection_fill` is the
    /// selection background (some editors use `ACCENT_DIM` instead of
    /// `ACCENT_GLOW`).
    pub fn apply_with_selection(ctx: &egui::Context, selection_fill: egui::Color32) {
        let mut visuals = egui::Visuals::dark();
        visuals.window_fill = PANEL;
        visuals.panel_fill = BG;
        visuals.override_text_color = Some(TEXT);
        visuals.faint_bg_color = PANEL;
        visuals.extreme_bg_color = PANEL;
        visuals.widgets.noninteractive.bg_fill = PANEL;
        visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_DIM);
        visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
        visuals.widgets.inactive.bg_fill = PANEL_LIGHT;
        visuals.widgets.inactive.weak_bg_fill = PANEL_LIGHT;
        visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT);
        visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
        visuals.widgets.hovered.bg_fill = PANEL_LIGHT;
        visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.5, ACCENT);
        visuals.widgets.active.bg_fill = PANEL_LIGHT;
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.5, ACCENT);
        visuals.widgets.open.bg_fill = PANEL_LIGHT;
        visuals.selection.bg_fill = selection_fill;
        visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
        ctx.set_visuals(visuals);
    }
}

/// Canonical Resonance palette (lavender accent) used by the instrument
/// editors (drums, wavetable). Mirrors the main app's `theme.rs` tokens.
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

    pub const WARM: egui::Color32 = egui::Color32::from_rgb(0xe8, 0xc4, 0x7b);

    pub const GOOD: egui::Color32 = egui::Color32::from_rgb(0x6d, 0xd6, 0xa3);
    pub const BAD: egui::Color32 = egui::Color32::from_rgb(0xe8, 0x7b, 0x8b);

    // ---------- Backwards-compatible aliases ----------
    // A few older modules still reference these names.
    pub const PANEL: egui::Color32 = BG_2;
    pub const BORDER: egui::Color32 = LINE;
    pub const TEXT_DIM: egui::Color32 = TEXT_3;

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
}
