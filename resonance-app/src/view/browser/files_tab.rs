//! Files tab body for the media browser (todo #602).
//!
//! Renders the filesystem-browsing surface: a breadcrumb strip, a
//! favourites / recent shelf, a per-folder filter field, and the
//! scrollable folder / audio listing (or the empty-folder state).

use std::path::{Path, PathBuf};

use iced::widget::text::LineHeight;
use iced::widget::{button, column, container, row, scrollable, text, text_input, Canvas, Space};
use iced::{alignment, Color, Element, Length};

use resonance_common::audio_probe::AudioFileEntry;

use crate::message::*;
use crate::theme;
use crate::Resonance;

use super::style::{
    file_display_name, folder_display_name, format_chip, format_duration, pill_button_style,
    row_button_style, WaveThumbnail, THUMB_H, THUMB_W,
};

// ---------------------------------------------------------------------------
// Tab body entry point
// ---------------------------------------------------------------------------

/// The Files tab body: breadcrumb + favourites/recent shelf + per-folder
/// filter + the folder / audio listing (or the empty-folder state).
pub(super) fn files_body<'a>(r: &'a Resonance) -> Element<'a, Message> {
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

// ---------------------------------------------------------------------------
// Breadcrumb
// ---------------------------------------------------------------------------

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
    let color = if is_current { theme::TEXT_1 } else { theme::TEXT_2 };
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

// ---------------------------------------------------------------------------
// Favourites / recent shelf
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Filter field
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Folder listing
// ---------------------------------------------------------------------------

/// The scrollable folder listing: subfolder rows followed by audio rows,
/// or the empty-folder state when the open folder holds no audio. While an
/// off-thread scan is in flight a "Scanning…" hint stands in.
///
/// The rows sit in a `lazy` region keyed on [`listing_fingerprint`]: the
/// audition transport below the tab is live (its playhead moves every
/// tick), and without the cache every tick rebuilt one row per file —
/// display name, duration and lower-cased filter strings included
/// (ui-work.md §11, review VIEW-27). The scrollable stays outside the lazy
/// region so its scroll position is never tied to the cache.
fn folder_listing<'a>(r: &'a Resonance) -> Element<'a, Message> {
    if r.browser.scanning {
        return placeholder("Scanning\u{2026}");
    }

    let rows = iced::widget::lazy(
        listing_fingerprint(r),
        move |_: &u64| -> Element<'static, Message> { listing_rows(r) },
    );
    scrollable(rows).height(Length::Fill).into()
}

/// The listing's rows (see [`folder_listing`]). Built only when the lazy
/// key changes.
fn listing_rows(r: &Resonance) -> Element<'static, Message> {
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

    list.into()
}

/// Lazy key of the folder listing: everything [`listing_rows`] reads — the
/// scan (folders, files and their drawn metadata, thumbnails), the filter,
/// and the selected / playing rows that carry the highlight. The audition
/// playhead is deliberately absent: it renders in the transport, outside
/// the region. Hashing allocates nothing.
pub(crate) fn listing_fingerprint(r: &Resonance) -> u64 {
    use std::hash::{Hash, Hasher};
    let b = &r.browser;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.scan.folders.hash(&mut h);
    b.scan.files.len().hash(&mut h);
    for entry in &b.scan.files {
        entry.path.hash(&mut h);
        std::mem::discriminant(&entry.info.format).hash(&mut h);
        entry.info.duration_secs.to_bits().hash(&mut h);
        // Thumbnail peaks, by value: a rescan can refresh them.
        for &(lo, hi) in b.scan.thumbnail(&entry.path) {
            lo.to_bits().hash(&mut h);
            hi.to_bits().hash(&mut h);
        }
    }
    b.filter.hash(&mut h);
    b.audition.selected.hash(&mut h);
    b.audition.playing.hash(&mut h);
    h.finish()
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
fn audio_row(r: &Resonance, entry: &AudioFileEntry) -> Element<'static, Message> {
    let path = std::path::Path::new(&entry.path);
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
        peaks: std::borrow::Cow::Owned(r.browser.scan.thumbnail(&entry.path).to_vec()),
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

    let meta_row =
        row![name_label, Space::new().width(4), fmt_chip, Space::new().width(4), duration]
            .align_y(alignment::Vertical::Center)
            .spacing(0);

    let content =
        row![glyph, Space::new().width(6), thumbnail, Space::new().width(8), meta_row]
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

// ---------------------------------------------------------------------------
// Empty-folder state
// ---------------------------------------------------------------------------

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
