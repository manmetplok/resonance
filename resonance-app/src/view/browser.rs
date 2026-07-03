//! Docked media-browser panel for the Arrange view (design doc #175,
//! epic #35).
//!
//! The container scaffold landed in todo #601: a fixed-width left column
//! ([`theme::BROWSER_WIDTH`], `BG_2` fill, `LINE` right border), its header,
//! and the Files / Pool tab switcher. This file now also holds the **Pool tab
//! body** (todo #603): a scrollable list of the project's imported audio
//! assets, each annotated with a usage badge (`used ×N` / `unused`) and —
//! for assets whose backing file is missing — an inline `relink` chip.
//!
//! The **Files tab body** (todo #602) browses the filesystem:
//!
//! * a **breadcrumb** row of clickable path crumbs plus a WARM star toggle
//!   that favourites / unfavourites the current folder;
//! * a **favourites / recent shelf** of pill chips (favourite = WARM star,
//!   recent = clock) — the entry point before any folder is open;
//! * a **per-folder filter** field (case-insensitive file-name substring);
//! * the **subfolder rows** (folder glyph + name) and **audio rows** (type
//!   glyph, mini waveform thumbnail, lightly colour-coded format chip, and
//!   duration); and
//! * the **empty-folder state** — copy plus a "Choose files…" button that
//!   opens the import picker — shown when an open folder holds no audio.
//!
//! The mini waveform thumbnails come from the `resonance_common`
//! waveform-thumbnail peaks decoded off-thread during the folder scan
//! ([`crate::state::FolderScan::thumbnails`]) and cached per navigation, per
//! the view-performance rules.
//!
//! Pool tab row anatomy (left → right):
//!
//! * **Type glyph** — ♪ (WARM) or ⚠ (BAD for missing).
//! * **Mini waveform thumbnail** — a 48 × 26 px Canvas bar chart of the
//!   asset's downsampled `thumbnail_peaks`.
//! * **Name** — the original filename without its directory path.
//! * **Format chip** — `wav` / `flac` / `mp3` / `ogg` etc., WARM border pill.
//! * **Duration** — `M:SS` derived from `duration_frames / sample_rate`.
//! * **Usage badge** — `used ×N` (ACCENT_SOFT) or `unused` (TEXT_3).
//! * *For missing rows only*: an inline `relink` chip (BAD-outlined, fires
//!   [`RelinkMessage::ShowModal`](crate::message::RelinkMessage::ShowModal)).
//!
//! The breadcrumb strip is intentionally absent from the Pool tab — the
//! pool is a project-level list, not a filesystem path.

use std::path::{Path, PathBuf};

use iced::widget::canvas::{self, Frame, Geometry};
use iced::widget::text::LineHeight;
use iced::widget::{
    button, column, container, row, scrollable, text, text_input, Canvas, Space,
};
use iced::{alignment, mouse, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use resonance_common::audio_probe::AudioFileEntry;

use crate::message::*;
use crate::state::pool::PoolAsset;
use crate::state::BrowserTab;
use crate::theme;
use crate::view::controls::collapse_caret;
use crate::view::relink_dialog::relink_chip;
use crate::Resonance;

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

/// Build the docked media-browser panel: a `BROWSER_WIDTH` column with a
/// `LINE` right border. Returned only when `browser.visible`; the caller
/// (`view_main_area`) prepends it to the arrange row so it sits flush
/// against the left edge, a peer of the track headers + timeline.
pub(crate) fn view_browser_panel(r: &Resonance) -> Element<'_, Message> {
    let tab = r.browser.tab;

    let body: Element<'_, Message> = column![
        header(),
        Space::new().height(14),
        tab_switcher(tab),
        Space::new().height(14),
        tab_body(r, tab),
    ]
    .spacing(0)
    .height(Length::Fill)
    .into();

    let panel = container(body)
        .width(Length::Fixed(theme::BROWSER_WIDTH))
        .height(Length::Fill)
        .padding(18)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            ..Default::default()
        });

    // `LINE` right border as a 1px hairline — the same separator the mixer
    // uses between its columns. Keeping it a sibling (rather than a uniform
    // `Border` on the container) gives a right-edge-only rule.
    let right_border =
        container(Space::new().width(1).height(Length::Fill)).style(theme::separator_bg);

    row![panel, right_border].spacing(0).into()
}

// ---------------------------------------------------------------------------
// Panel chrome
// ---------------------------------------------------------------------------

/// Panel title row: a "MEDIA" section label plus the shared collapse caret,
/// which closes the panel (`ToggleVisible`) — the same affordance as the
/// "Media" chrome toggle.
fn header<'a>() -> Element<'a, Message> {
    let title = text("MEDIA")
        .size(10)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0));

    let caret = button(collapse_caret(true))
        .on_press(Message::Browser(BrowserMessage::ToggleVisible))
        .padding(0)
        .style(|_theme, status| theme::ghost_button_style(status));

    row![
        title,
        Space::new().width(Length::Fill),
        caret,
    ]
    .align_y(alignment::Vertical::Center)
    .into()
}

/// Files / Pool segmented switcher, styled like the chrome view tabs.
fn tab_switcher<'a>(current: BrowserTab) -> Element<'a, Message> {
    container(
        row![
            browser_tab_button("Files", BrowserTab::Files, current),
            browser_tab_button("Pool", BrowserTab::Pool, current),
        ]
        .spacing(3)
        .padding(4),
    )
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(theme::BG_1)),
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: theme::RADIUS_LG.into(),
        },
        ..Default::default()
    })
    .into()
}

fn browser_tab_button<'a>(
    label: &'a str,
    tab: BrowserTab,
    current: BrowserTab,
) -> iced::widget::Button<'a, Message> {
    let active = current == tab;
    button(
        text(label)
            .size(12)
            .font(theme::UI_FONT_MEDIUM)
            .line_height(LineHeight::Relative(1.0)),
    )
    .on_press(Message::Browser(BrowserMessage::SelectTab(tab)))
    .style(move |_theme, status| theme::tab_button_style(active, status))
    .padding([6, 16])
}

// ---------------------------------------------------------------------------
// Per-tab bodies
// ---------------------------------------------------------------------------

/// Per-tab body. The Files tab leads with the filesystem breadcrumb; the
/// Pool tab hides it (a project-level list has no path).
fn tab_body<'a>(r: &'a Resonance, tab: BrowserTab) -> Element<'a, Message> {
    match tab {
        BrowserTab::Files => files_body(r),
        BrowserTab::Pool => pool_body(r),
    }
}

// ---------------------------------------------------------------------------
// Files tab body (todo #602)
// ---------------------------------------------------------------------------

/// The Files tab body: breadcrumb + favourites/recent shelf + per-folder
/// filter + the folder / audio listing (or the empty-folder state).
fn files_body<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let mut col = column![breadcrumb_row(r)].spacing(0);

    if let Some(shelf) = folder_shelf(r) {
        col = col.push(Space::new().height(12)).push(shelf);
    }

    if r.browser.current_folder.is_some() {
        col = col
            .push(Space::new().height(12))
            .push(filter_field(r))
            .push(Space::new().height(10))
            .push(folder_listing(r));
    } else {
        col = col.push(Space::new().height(14)).push(placeholder(
            "Open a favourite or recent folder to browse audio \u{2014} or use Import audio\u{2026} to add files to the project.",
        ));
    }

    col.height(Length::Fill).into()
}

/// Breadcrumb strip: clickable path crumbs (each an `OpenFolder` target),
/// truncated to the last few segments so a deep path never overflows the
/// panel, plus a trailing WARM star that favourites / unfavourites the
/// current folder.
fn breadcrumb_row<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let crumbs = r.browser.breadcrumb();

    let mut trail = row![].spacing(2).align_y(alignment::Vertical::Center);
    if crumbs.is_empty() {
        trail = trail.push(
            text("No folder open")
                .size(11)
                .font(theme::MONO_FONT)
                .color(theme::TEXT_3)
                .line_height(LineHeight::Relative(1.0)),
        );
    } else {
        // Keep the breadcrumb compact: show at most the last three crumbs,
        // prefixed with a muted "…" when the chain is deeper.
        const MAX_CRUMBS: usize = 3;
        let start = crumbs.len().saturating_sub(MAX_CRUMBS);
        if start > 0 {
            trail = trail.push(sep_slash()).push(
                text("\u{2026}")
                    .size(11)
                    .color(theme::TEXT_3)
                    .line_height(LineHeight::Relative(1.0)),
            );
        }
        let last = crumbs.len() - 1;
        for (i, path) in crumbs.iter().enumerate().skip(start) {
            if i > 0 {
                trail = trail.push(sep_slash());
            }
            trail = trail.push(crumb_button(path, i == last));
        }
    }

    let mut header = row![trail, Space::new().width(Length::Fill)]
        .align_y(alignment::Vertical::Center)
        .spacing(4);

    if let Some(folder) = r.browser.current_folder.as_ref() {
        header = header.push(favourite_star(folder, r.pool.is_favourite(folder)));
    }

    container(header).width(Length::Fill).into()
}

/// A single breadcrumb crumb: a ghost button showing the folder name that
/// navigates into that ancestor. The final crumb (the current folder) is
/// brighter than its parents.
fn crumb_button<'a>(path: &Path, is_current: bool) -> Element<'a, Message> {
    let color = if is_current {
        theme::TEXT_1
    } else {
        theme::TEXT_2
    };
    button(
        text(folder_display_name(path))
            .size(11)
            .font(theme::MONO_FONT)
            .color(color)
            .line_height(LineHeight::Relative(1.0)),
    )
    .on_press(Message::Browser(BrowserMessage::OpenFolder(
        path.to_path_buf(),
    )))
    .padding([1, 3])
    .style(|_theme, status| theme::ghost_button_style(status))
    .into()
}

/// The `/` separator between breadcrumb crumbs.
fn sep_slash<'a>() -> Element<'a, Message> {
    text("/")
        .size(11)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0))
        .into()
}

/// The WARM star toggle that favourites / unfavourites `folder`. Filled
/// WARM when pinned, muted otherwise.
fn favourite_star<'a>(folder: &Path, favourite: bool) -> Element<'a, Message> {
    let star = text(String::from(theme::fa::STAR))
        .font(theme::ICON_FONT)
        .size(12)
        .color(if favourite { theme::WARM } else { theme::TEXT_3 })
        .line_height(LineHeight::Relative(1.0));
    button(star)
        .on_press(Message::Browser(BrowserMessage::ToggleFavourite(
            folder.to_path_buf(),
        )))
        .padding([2, 4])
        .style(|_theme, status| theme::ghost_button_style(status))
        .into()
}

/// The favourites / recent shelf: pill chips for pinned favourites (WARM
/// star) then recently-visited folders (clock), each an `OpenFolder`
/// target. Returns `None` when both lists are empty. Recents that are also
/// favourites are shown once (as a favourite). Chips are laid out two per
/// row so they never overflow the narrow panel.
fn folder_shelf<'a>(r: &'a Resonance) -> Option<Element<'a, Message>> {
    let favourites = &r.pool.favourites;
    let recents: Vec<&PathBuf> = r
        .pool
        .recent_folders
        .iter()
        .filter(|p| !favourites.contains(p))
        .collect();
    if favourites.is_empty() && recents.is_empty() {
        return None;
    }

    // (path, glyph, glyph-colour) for every chip, favourites first.
    let chips: Vec<(&PathBuf, char, Color)> = favourites
        .iter()
        .map(|p| (p, theme::fa::STAR, theme::WARM))
        .chain(recents.into_iter().map(|p| (p, theme::fa::CLOCK, theme::TEXT_2)))
        .collect();

    let mut grid = column![].spacing(6);
    for pair in chips.chunks(2) {
        let mut line = row![].spacing(6);
        for (path, glyph, glyph_color) in pair {
            line = line.push(shelf_pill(path, *glyph, *glyph_color));
        }
        grid = grid.push(line);
    }

    Some(
        column![shelf_caption("FAVOURITES & RECENT"), Space::new().height(6), grid]
            .spacing(0)
            .into(),
    )
}

/// One shelf pill chip: an icon + the folder name, navigating into it.
fn shelf_pill<'a>(path: &Path, glyph: char, glyph_color: Color) -> Element<'a, Message> {
    let content = row![
        text(String::from(glyph))
            .font(theme::ICON_FONT)
            .size(9)
            .color(glyph_color)
            .line_height(LineHeight::Relative(1.0)),
        Space::new().width(5),
        text(folder_display_name(path))
            .size(11)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_1)
            .line_height(LineHeight::Relative(1.0)),
    ]
    .align_y(alignment::Vertical::Center)
    .spacing(0);

    button(content)
        .on_press(Message::Browser(BrowserMessage::OpenFolder(
            path.to_path_buf(),
        )))
        .padding([4, 9])
        .style(|_theme, status| pill_button_style(status))
        .into()
}

/// A muted uppercase caption above a shelf / section.
fn shelf_caption<'a>(label: &'a str) -> Element<'a, Message> {
    text(label)
        .size(9)
        .font(theme::UI_FONT_SEMIBOLD)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0))
        .into()
}

/// The per-folder filter field: a magnifier glyph + a borderless text input
/// bound to [`BrowserState::filter`](crate::state::BrowserState::filter).
fn filter_field<'a>(r: &'a Resonance) -> Element<'a, Message> {
    let field = row![
        text(String::from(theme::fa::MAGNIFYING_GLASS))
            .font(theme::ICON_FONT)
            .size(11)
            .color(theme::TEXT_3)
            .line_height(LineHeight::Relative(1.0)),
        text_input("Filter this folder\u{2026}", &r.browser.filter)
            .on_input(|s| Message::Browser(BrowserMessage::SetFilter(s)))
            .size(12)
            .padding([4, 6])
            .style(theme::borderless_text_input_style),
    ]
    .spacing(6)
    .align_y(alignment::Vertical::Center);

    container(field)
        .padding([6, 10])
        .width(Length::Fill)
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_2)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_MD.into(),
            },
            ..Default::default()
        })
        .into()
}

/// The scrollable folder listing: subfolder rows followed by audio rows,
/// or the empty-folder state when the open folder holds no audio. While an
/// off-thread scan is in flight a "Scanning…" hint stands in.
fn folder_listing<'a>(r: &'a Resonance) -> Element<'a, Message> {
    if r.browser.scanning {
        return placeholder("Scanning\u{2026}");
    }

    let mut list = column![].spacing(4);

    for folder in &r.browser.scan.folders {
        list = list.push(folder_row(folder));
    }

    if r.browser.scan.files.is_empty() {
        // The folder holds no audio at all → empty-folder state. Any
        // subfolders above still show so the user can keep navigating.
        if !r.browser.scan.folders.is_empty() {
            list = list.push(Space::new().height(8));
        }
        list = list.push(empty_folder_state());
    } else {
        let matched: Vec<&AudioFileEntry> = r.browser.filtered_files().collect();
        if matched.is_empty() {
            // Folder has audio, but the filter hides every row.
            list = list
                .push(Space::new().height(6))
                .push(placeholder("No files match the filter."));
        } else {
            for entry in matched {
                list = list.push(audio_row(r, entry));
            }
        }
    }

    scrollable(list).height(Length::Fill).into()
}

/// A subfolder row: a folder glyph, the folder name, and a trailing caret,
/// navigating into the folder on press.
fn folder_row<'a>(path: &Path) -> Element<'a, Message> {
    let content = row![
        text(String::from(theme::fa::FOLDER))
            .font(theme::ICON_FONT)
            .size(12)
            .color(theme::TEXT_2)
            .line_height(LineHeight::Relative(1.0)),
        Space::new().width(8),
        text(folder_display_name(path))
            .size(12)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_1)
            .line_height(LineHeight::Relative(1.0)),
        Space::new().width(Length::Fill),
        text(String::from(theme::fa::CARET_RIGHT))
            .font(theme::ICON_FONT)
            .size(11)
            .color(theme::TEXT_3)
            .line_height(LineHeight::Relative(1.0)),
    ]
    .align_y(alignment::Vertical::Center)
    .spacing(0);

    button(content)
        .on_press(Message::Browser(BrowserMessage::OpenFolder(
            path.to_path_buf(),
        )))
        .width(Length::Fill)
        .padding([6, 6])
        .style(|_theme, status| row_button_style(false, status))
        .into()
}

/// An audio file row: a type glyph, a mini waveform thumbnail, the file
/// name, a lightly colour-coded format chip, and the duration. The whole
/// row is a select-to-audition button; the currently-playing / selected
/// row carries a WARM highlight.
fn audio_row<'a>(r: &'a Resonance, entry: &'a AudioFileEntry) -> Element<'a, Message> {
    let path = Path::new(&entry.path);
    let playing = r.browser.audition.is_playing(path);
    let selected = r.browser.audition.is_selected(path);

    // Type glyph — WARM music note (audio domain).
    let glyph = text(String::from(theme::fa::MUSIC))
        .font(theme::ICON_FONT)
        .size(11)
        .color(theme::WARM)
        .line_height(LineHeight::Relative(1.0));

    // Mini waveform thumbnail from the cached scan peaks.
    let thumbnail = Canvas::new(WaveThumbnail {
        peaks: r.browser.scan.thumbnail(&entry.path),
        muted: false,
    })
    .width(Length::Fixed(THUMB_W))
    .height(Length::Fixed(THUMB_H));

    // File name (final path component).
    let name_label = text(file_display_name(&entry.path))
        .size(11)
        .font(theme::UI_FONT_MEDIUM)
        .color(theme::TEXT_1)
        .line_height(LineHeight::Relative(1.0));

    // Format chip (WARM-outlined pill).
    let fmt_chip = format_chip(entry.info.format);

    // Duration `M:SS`.
    let duration = text(format_duration(entry.info.duration_secs))
        .size(10)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0));

    let meta_row = row![name_label, Space::new().width(4), fmt_chip, Space::new().width(4), duration]
        .align_y(alignment::Vertical::Center)
        .spacing(0);

    let content = row![glyph, Space::new().width(6), thumbnail, Space::new().width(8), meta_row]
        .align_y(alignment::Vertical::Center)
        .spacing(0);

    button(content)
        .on_press(Message::Browser(BrowserMessage::Select(Some(
            path.to_path_buf(),
        ))))
        .width(Length::Fill)
        .padding([5, 6])
        .style(move |_theme, status| row_button_style(playing || selected, status))
        .into()
}

/// The empty-folder state: an icon, an explanatory line, and a
/// "Choose files…" button that opens the import picker.
fn empty_folder_state<'a>() -> Element<'a, Message> {
    let icon = text(String::from(theme::fa::FOLDER_OPEN))
        .font(theme::ICON_FONT)
        .size(22)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0));

    let heading = text("No audio in this folder")
        .size(13)
        .font(theme::UI_FONT_MEDIUM)
        .color(theme::TEXT_2)
        .line_height(LineHeight::Relative(1.0));

    let body = text("Drop audio here, or choose files to import into the project.")
        .size(11)
        .color(theme::TEXT_3)
        .align_x(alignment::Horizontal::Center);

    let choose = button(
        text("Choose files\u{2026}")
            .size(12)
            .font(theme::UI_FONT_MEDIUM)
            .line_height(LineHeight::Relative(1.0)),
    )
    .on_press(Message::Pool(PoolMessage::PickFiles))
    .padding([7, 14])
    .style(|_theme, status| theme::primary_button_style(status));

    let block = column![
        icon,
        Space::new().height(8),
        heading,
        Space::new().height(4),
        body,
        Space::new().height(12),
        choose,
    ]
    .align_x(alignment::Horizontal::Center)
    .spacing(0);

    container(block)
        .width(Length::Fill)
        .padding([28, 12])
        .center_x(Length::Fill)
        .into()
}

fn placeholder<'a>(body: &'a str) -> Element<'a, Message> {
    text(body).size(12).color(theme::TEXT_3).into()
}

// ---------------------------------------------------------------------------
// Pool tab body (todo #603)
// ---------------------------------------------------------------------------

/// The Pool tab body: a scrollable list of the project's imported audio
/// assets. Each row shows a type glyph, a mini waveform thumbnail, the
/// filename, a format chip, the duration, and a usage badge. Missing rows
/// carry an inline `relink` chip instead of (or alongside) the usage badge.
fn pool_body<'a>(r: &'a Resonance) -> Element<'a, Message> {
    if r.pool.assets.is_empty() {
        return placeholder("No assets imported yet. Drag audio onto the timeline or use Import audio\u{2026}");
    }

    let sample_rate = r.sample_rate.max(1);
    let mut list = column![].spacing(4);
    for asset in &r.pool.assets {
        let usage = r.pool.usage_count(asset.id);
        list = list.push(pool_asset_row(asset, usage, sample_rate));
    }

    scrollable(list)
        .height(Length::Fill)
        .into()
}

/// One pool-asset row. Layout (left → right):
///
/// ```text
/// [♪] [~~waveform~~] [filename     ] [wav] [0:04] [used ×2]
///                                                  [relink ]  ← only if missing
/// ```
fn pool_asset_row<'a>(asset: &'a PoolAsset, usage: u32, sample_rate: u32) -> Element<'a, Message> {
    // --- Type glyph -------------------------------------------------------
    let (glyph_char, glyph_color) = if asset.missing {
        (theme::fa::TRIANGLE_EXCLAMATION, theme::BAD)
    } else {
        (theme::fa::MUSIC, theme::WARM)
    };
    let glyph = text(String::from(glyph_char))
        .font(theme::ICON_FONT)
        .size(11)
        .color(glyph_color)
        .line_height(LineHeight::Relative(1.0));

    // --- Mini waveform thumbnail ------------------------------------------
    // Fixed 48 × 26 px Canvas bar-chart of thumbnail_peaks (WARM amber).
    let thumbnail = Canvas::new(WaveThumbnail {
        peaks: &asset.thumbnail_peaks,
        muted: asset.missing,
    })
    .width(Length::Fixed(THUMB_W))
    .height(Length::Fixed(THUMB_H));

    // --- Filename ---------------------------------------------------------
    let fname = asset_filename(asset).into_owned();
    let name_label = text(fname)
        .size(11)
        .font(theme::UI_FONT_MEDIUM)
        .color(if asset.missing { theme::TEXT_3 } else { theme::TEXT_1 })
        .line_height(LineHeight::Relative(1.0));

    // --- Format chip ------------------------------------------------------
    let fmt_str = format_label(asset.format);
    let fmt_chip = container(
        text(fmt_str)
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(if asset.missing { theme::TEXT_3 } else { theme::WARM })
            .line_height(LineHeight::Relative(1.0)),
    )
    .padding([2, 5])
    .style(move |_theme| {
        let border_color = if asset.missing {
            theme::LINE
        } else {
            theme::WARM_LINE
        };
        container::Style {
            border: iced::Border {
                color: border_color,
                width: 1.0,
                radius: 999.0.into(),
            },
            ..Default::default()
        }
    });

    // --- Duration ---------------------------------------------------------
    let dur_secs = asset.duration_frames as f64 / sample_rate as f64;
    let dur_label = format_duration(dur_secs);
    let duration = text(dur_label)
        .size(10)
        .font(theme::MONO_FONT)
        .color(theme::TEXT_3)
        .line_height(LineHeight::Relative(1.0));

    // --- Name + chips row -------------------------------------------------
    let meta_row = row![name_label, Space::new().width(4), fmt_chip, Space::new().width(4), duration]
        .align_y(alignment::Vertical::Center)
        .spacing(0);

    // --- Usage badge / relink chip ----------------------------------------
    let badge: Element<'_, Message> = if asset.missing {
        relink_chip()
    } else if usage > 0 {
        text(format!("used \u{d7}{usage}"))
            .size(10)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::ACCENT_SOFT)
            .line_height(LineHeight::Relative(1.0))
            .into()
    } else {
        text("unused")
            .size(10)
            .font(theme::UI_FONT_MEDIUM)
            .color(theme::TEXT_3)
            .line_height(LineHeight::Relative(1.0))
            .into()
    };

    // --- Compose the full row ---------------------------------------------
    let inner = column![meta_row, Space::new().height(3), badge].spacing(0);

    let content = row![
        glyph,
        Space::new().width(6),
        thumbnail,
        Space::new().width(8),
        inner,
    ]
    .align_y(alignment::Vertical::Center)
    .spacing(0);

    container(content)
        .width(Length::Fill)
        .padding([5, 6])
        .style(|_theme| container::Style {
            background: Some(iced::Background::Color(theme::BG_3)),
            border: iced::Border {
                color: theme::LINE_2,
                width: 1.0,
                radius: theme::RADIUS_SM.into(),
            },
            ..Default::default()
        })
        .into()
}

// ---------------------------------------------------------------------------
// Pool thumbnail Canvas
// ---------------------------------------------------------------------------

/// Fixed size of the mini waveform thumbnail in pool rows.
const THUMB_W: f32 = 48.0;
const THUMB_H: f32 = 26.0;

/// A simple stateless [`canvas::Program`] that draws a waveform bar-chart
/// from `(min, max)` peak pairs. Shared by the Pool asset thumbnail and the
/// Files-tab audio rows; not a live visual so no cache is needed. When
/// `muted` (a missing pool asset) it draws in a faint BAD tint; otherwise
/// in WARM amber. An empty `peaks` slice draws just the centre baseline —
/// the idle state for a file whose waveform could not be decoded.
struct WaveThumbnail<'a> {
    peaks: &'a [(f32, f32)],
    muted: bool,
}

impl canvas::Program<Message> for WaveThumbnail<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let w = bounds.width;
        let h = bounds.height;
        let mid = h / 2.0;

        // Backdrop
        frame.fill_rectangle(Point::ORIGIN, Size::new(w, h), theme::BG_3);
        // Centre zero line
        frame.fill_rectangle(Point::new(0.0, mid - 0.5), Size::new(w, 1.0), theme::LINE_2);

        if !self.peaks.is_empty() {
            let bar_color: Color = if self.muted {
                Color { a: 0.45, ..theme::BAD }
            } else {
                Color { a: 0.75, ..theme::WARM }
            };
            let col_w = w / self.peaks.len() as f32;
            let bar_w = col_w.max(1.0);
            for (i, &(min_v, max_v)) in self.peaks.iter().enumerate() {
                let x = i as f32 * col_w;
                let top = mid - max_v.clamp(-1.0, 1.0) * mid;
                let bot = mid - min_v.clamp(-1.0, 1.0) * mid;
                let bar_h = (bot - top).max(1.0);
                frame.fill_rectangle(Point::new(x, top), Size::new(bar_w, bar_h), bar_color);
            }
        }

        vec![frame.into_geometry()]
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The filename (final path component) of an asset's original source path,
/// falling back to the whole path string when there is no component.
fn asset_filename(asset: &PoolAsset) -> std::borrow::Cow<'_, str> {
    Path::new(&asset.original_path)
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or(std::borrow::Cow::Borrowed(&asset.original_path))
}

/// The display name of a folder: its final path component, or the whole
/// path (e.g. the filesystem root `/`) when there is no component.
fn folder_display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// The display name of an audio file: the final component of its path.
fn file_display_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string())
}

/// A lightly colour-coded format chip (outlined pill) for an audio row.
/// The audio domain reads WARM; each container/codec family gets a subtle
/// hue shift off that base so wav / flac / mp3 / ogg are distinguishable at
/// a glance without leaving the palette.
fn format_chip<'a>(fmt: resonance_common::AudioFormat) -> Element<'a, Message> {
    let tint = format_color(fmt);
    container(
        text(format_label(fmt))
            .size(9)
            .font(theme::UI_FONT_SEMIBOLD)
            .color(tint)
            .line_height(LineHeight::Relative(1.0)),
    )
    .padding([2, 5])
    .style(move |_theme| container::Style {
        border: iced::Border {
            color: Color { a: 0.34, ..tint },
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    })
    .into()
}

/// The subtle per-format tint for the format chip. All stay muted and
/// close to WARM so the audio domain still reads amber; the small hue
/// shifts just help wav / flac / mp3 / ogg tell each other apart.
fn format_color(fmt: resonance_common::AudioFormat) -> Color {
    use resonance_common::AudioFormat;
    match fmt {
        // WARM amber — the uncompressed baseline.
        AudioFormat::Wav => theme::WARM,
        // Soft sage — lossless-compressed.
        AudioFormat::Flac => Color::from_rgb(0.60, 0.82, 0.69),
        // Soft lavender — lossy MPEG.
        AudioFormat::Mp3 => theme::ACCENT_SOFT,
        // Soft coral — lossy Vorbis.
        AudioFormat::Ogg => Color::from_rgb(0.90, 0.66, 0.56),
        // Everything else falls back to muted neutral.
        AudioFormat::Aac | AudioFormat::Mp4 | AudioFormat::Other => theme::TEXT_2,
    }
}

/// Button style for a folder / audio listing row. Neutral raised card by
/// default; a WARM wash when `highlighted` (the playing / selected audio
/// row); a lighter fill on hover.
fn row_button_style(highlighted: bool, status: button::Status) -> button::Style {
    let (bg, border_color) = if highlighted {
        (theme::WARM_DIM, theme::WARM_LINE)
    } else {
        match status {
            button::Status::Hovered | button::Status::Pressed => (theme::BG_3, theme::LINE),
            _ => (theme::BG_3, theme::LINE_2),
        }
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: theme::TEXT_1,
        border: iced::Border {
            color: border_color,
            width: 1.0,
            radius: theme::RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

/// Button style for a favourites / recent shelf pill: a full-round chip
/// that lifts on hover.
fn pill_button_style(status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => theme::BG_3,
        _ => theme::BG_2,
    };
    button::Style {
        background: Some(iced::Background::Color(bg)),
        text_color: theme::TEXT_1,
        border: iced::Border {
            color: theme::LINE_2,
            width: 1.0,
            radius: 999.0.into(),
        },
        ..Default::default()
    }
}

/// Short label for the container/codec family shown in the format chip.
fn format_label(fmt: resonance_common::AudioFormat) -> &'static str {
    use resonance_common::AudioFormat;
    match fmt {
        AudioFormat::Wav => "wav",
        AudioFormat::Flac => "flac",
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Ogg => "ogg",
        AudioFormat::Aac => "aac",
        AudioFormat::Mp4 => "m4a",
        AudioFormat::Other => "audio",
    }
}

/// Format a duration in seconds as `M:SS` (e.g. `"2:07"`).
fn format_duration(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    let m = total / 60;
    let s = total % 60;
    format!("{m}:{s:02}")
}
