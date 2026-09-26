//! `.clap` path → dlopen-target resolution: plain files load as-is, a
//! macOS-style bundle directory descends to `Contents/MacOS/<name>`
//! with the name taken from Info.plist's `CFBundleExecutable` when
//! present and the bundle's file stem otherwise.

use std::path::{Path, PathBuf};

use resonance_audio::test_support::bundle_binary_path;

/// A scratch dir unique to this test process, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "resonance-clap-bundle-path-{}-{tag}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn plain_file_is_returned_unchanged() {
    let scratch = Scratch::new("plain");
    let so = scratch.path().join("gate.clap");
    std::fs::write(&so, b"not really an so").unwrap();
    assert_eq!(bundle_binary_path(&so), so);
}

#[test]
fn missing_path_is_returned_unchanged() {
    // Not a dir, not a file — dlopen gets to produce the error.
    let missing = Path::new("/nonexistent/gate.clap");
    assert_eq!(bundle_binary_path(missing), missing);
}

#[test]
fn bundle_dir_descends_to_stem_named_binary() {
    let scratch = Scratch::new("stem");
    let bundle = scratch.path().join("Surge XT.clap");
    std::fs::create_dir_all(bundle.join("Contents").join("MacOS")).unwrap();
    assert_eq!(
        bundle_binary_path(&bundle),
        bundle.join("Contents").join("MacOS").join("Surge XT")
    );
}

#[test]
fn info_plist_executable_wins_over_stem() {
    let scratch = Scratch::new("plist");
    let bundle = scratch.path().join("Surge XT.clap");
    let contents = bundle.join("Contents");
    std::fs::create_dir_all(contents.join("MacOS")).unwrap();
    std::fs::write(
        contents.join("Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Surge XT</string>
    <key>CFBundleExecutable</key>
    <string>SurgeXT-binary</string>
</dict>
</plist>
"#,
    )
    .unwrap();
    assert_eq!(
        bundle_binary_path(&bundle),
        contents.join("MacOS").join("SurgeXT-binary")
    );
}

#[test]
fn malformed_plist_falls_back_to_stem() {
    let scratch = Scratch::new("badplist");
    let bundle = scratch.path().join("gate.clap");
    let contents = bundle.join("Contents");
    std::fs::create_dir_all(contents.join("MacOS")).unwrap();
    std::fs::write(contents.join("Info.plist"), "<key>CFBundleExecutable</key>").unwrap();
    assert_eq!(
        bundle_binary_path(&bundle),
        contents.join("MacOS").join("gate")
    );
}
