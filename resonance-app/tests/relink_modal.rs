//! Golden-image + behavioural coverage for the **missing-files relink
//! modal** (design doc #175, todo #607).
//!
//! The modal is surfaced on load when a project's media pool references
//! audio whose backing WAV is gone. It lists every missing file with a
//! per-file `Locate…` action and a one-shot `Search a folder…`, and
//! updates live as relinks land (a resolved row flips to a green
//! `Relinked` check, the footer tallies `N of M relinked`).
//!
//! The reducers behind `Locate` / `SearchFolder` open OS dialogs, so
//! those aren't driven here (they're covered in `tests/relink.rs`); these
//! tests exercise the modal's own wiring — open on request, list the
//! missing assets, reflect a completed relink, and dismiss — plus two
//! locked-in snapshots:
//!
//! 1. **two missing files** — the modal as it opens: two `Locate…` rows,
//!    the batch note, and `0 of 2 relinked` in the footer.
//! 2. **one relinked** — after one asset resolves through the real
//!    `RelinkMessage::Imported(Ok)` path: that row flips to `Relinked`
//!    and the footer reads `1 of 2 relinked`.

use std::path::{Path, PathBuf};

use iced::Size;
use iced_test::simulator::Simulator;
use resonance_app::message::{Message, RelinkMessage};
use resonance_app::state::{PoolAsset, ViewMode};
use resonance_app::{demo, theme, Resonance, STARTUP_TAB};
use resonance_audio::types::AssetId;

const RATE: u32 = 48_000;
const WINDOW: (f32, f32) = (1440.0, 900.0);

fn sim_settings() -> iced::Settings {
    let mut fonts: Vec<std::borrow::Cow<'static, [u8]>> = Vec::new();
    fonts.push(theme::ICON_FONT_BYTES.into());
    for face in theme::UI_FONT_FACES {
        fonts.push((*face).into());
    }
    iced::Settings {
        fonts,
        default_font: theme::UI_FONT,
        ..iced::Settings::default()
    }
}

/// A pool asset flagged `missing`, whose original source was at
/// `original_path` (used for the row's filename + last-known path).
fn missing_asset(id: AssetId, original_path: &str) -> PoolAsset {
    PoolAsset {
        id,
        project_relative_path: format!("audio/asset_{id}.wav"),
        original_path: original_path.to_string(),
        format: resonance_common::AudioFormat::Wav,
        channels: 2,
        source_sample_rate: RATE,
        duration_frames: 0,
        thumbnail_peaks: Vec::new(),
        missing: true,
    }
}

/// Demo app on the Arrange tab with two missing pool assets and the
/// relink modal opened through the real `ShowModal` reducer, so each
/// snapshot has representative content dimmed behind the overlay.
fn build_app_with_relink_open() -> Resonance {
    let _ = STARTUP_TAB.set(ViewMode::Arrange);
    let (mut app, _task) = Resonance::new();
    demo::seed_demo_content(&mut app);
    app.test_add_pool_asset(missing_asset(1, "/Users/max/Samples/Guitars/Old Guitar Loop.wav"));
    app.test_add_pool_asset(missing_asset(2, "/Users/max/Downloads/Crowd Ambience.mp3"));
    app.test_dispatch(Message::Relink(RelinkMessage::ShowModal));
    app
}

fn snapshot_to(app: &Resonance, path: &str) {
    let mut ui = Simulator::with_size(sim_settings(), Size::new(WINDOW.0, WINDOW.1), app.view());
    let snap = ui
        .snapshot(&theme::resonance_theme())
        .expect("snapshot should render");
    assert!(
        snap.matches_image(path).expect("matches_image i/o"),
        "snapshot diverged from golden: {path}"
    );
}

/// A unique temp dir for one test, plus its `audio/` subfolder.
fn temp_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("resonance-relink-modal-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("audio")).expect("create audio dir");
    dir
}

/// Write a real, decodable stereo f32 WAV of `frames` non-silent frames.
fn write_source_wav(path: &Path, frames: usize) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create source parent");
    }
    let mut samples = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let v = ((i % 64) as f32 / 64.0) - 0.5;
        samples.push(v);
        samples.push(v);
    }
    resonance_audio::transcode_to_wav(path, &samples, RATE).expect("write source wav");
}

/// Run the same import the relink worker would (copy/transcode the source
/// into the project folder) and hand the outcome back as the reducer
/// expects.
fn run_worker(asset_id: AssetId, src: &Path, project_dir: &Path) -> RelinkMessage {
    let outcome =
        resonance_audio::import_one_to_pool(asset_id, &src.to_string_lossy(), project_dir, RATE)
            .expect("import_one_to_pool");
    RelinkMessage::Imported(Ok(outcome))
}

/// `ShowModal` opens the modal and snapshots the currently-missing assets
/// into its tracked set.
#[test]
fn show_modal_lists_missing_assets() {
    let app = build_app_with_relink_open();
    let relink = app.test_relink();
    assert!(relink.modal_open, "ShowModal opens the modal");
    assert_eq!(
        relink.modal_targets,
        vec![1, 2],
        "modal tracks both missing assets, in import order",
    );
    assert!(app.test_pool().has_missing());
}

/// `DismissModal` tears the overlay down and forgets its tracked set.
#[test]
fn dismiss_modal_closes_and_clears_targets() {
    let mut app = build_app_with_relink_open();
    app.test_dispatch(Message::Relink(RelinkMessage::DismissModal));
    let relink = app.test_relink();
    assert!(!relink.modal_open, "DismissModal closes the modal");
    assert!(
        relink.modal_targets.is_empty(),
        "dismiss forgets which assets it was tracking",
    );
}

/// A completed relink flips its row to resolved while the modal stays
/// open on its original tracked set, so the footer can read `1 of 2`.
#[test]
fn completed_relink_marks_row_resolved_and_keeps_modal_open() {
    let dir = temp_dir("resolved");
    let src = dir.join("sources/Old Guitar Loop.wav");
    write_source_wav(&src, 4_800);

    let mut app = build_app_with_relink_open();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.clone());

    // Resolve asset 1 through the real import path.
    let applied = run_worker(1, &src, &dir);
    app.test_dispatch(Message::Relink(applied));

    let relink = app.test_relink();
    assert!(relink.modal_open, "modal stays open after one relink");
    assert_eq!(
        relink.modal_targets,
        vec![1, 2],
        "the tracked set is unchanged so the resolved row stays visible",
    );
    assert!(
        !app.test_pool_asset(1).unwrap().missing,
        "relinked asset is no longer missing",
    );
    assert!(
        app.test_pool_asset(2).unwrap().missing,
        "the other asset is still missing",
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Golden: freshly-opened modal with two missing files (both offering
/// `Locate…`), the batch note, and `0 of 2 relinked` in the footer.
#[test]
fn relink_modal_two_missing_files() {
    let app = build_app_with_relink_open();
    snapshot_to(&app, "tests/snapshots/relink_modal_two_missing_files.png");
}

/// Golden: after one asset resolves, its row flips to a green `Relinked`
/// check and the footer reads `1 of 2 relinked`, while the second row
/// still offers `Locate…`.
///
/// The resolved state is set by re-adding asset 1 as no-longer-missing
/// (the same in-place refresh `apply_relinked_asset` performs) rather
/// than driving a real filesystem import — that keeps the snapshot
/// deterministic (no temp path / PID leaking into the row or the title
/// bar). The real import → resolved path is covered by
/// `completed_relink_marks_row_resolved_and_keeps_modal_open` and
/// `tests/relink.rs`.
#[test]
fn relink_modal_one_resolved() {
    let mut app = build_app_with_relink_open();
    let mut resolved = missing_asset(1, "/Users/max/Samples/Guitars/Old Guitar Loop.wav");
    resolved.missing = false;
    resolved.duration_frames = 4_800;
    app.test_add_pool_asset(resolved);

    assert!(!app.test_pool_asset(1).unwrap().missing);
    assert!(app.test_pool_asset(2).unwrap().missing);
    assert!(app.test_relink().modal_open);

    snapshot_to(&app, "tests/snapshots/relink_modal_one_resolved.png");
}
