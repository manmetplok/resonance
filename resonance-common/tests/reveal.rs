//! The shared file-manager launcher picks the right invocations; nothing is
//! spawned here.

use std::path::Path;

use resonance_common::reveal::{file_uri, reveal_commands};

#[test]
fn file_uris_percent_encode_everything_but_the_path_characters() {
    assert_eq!(
        file_uri(Path::new("/home/me/My Models/a_b-1.nam")).as_deref(),
        Some("file:///home/me/My%20Models/a_b-1.nam")
    );
    assert_eq!(file_uri(Path::new("relative.nam")), None);
}

#[test]
fn a_folder_opens_as_itself_and_a_file_is_selected_where_possible() {
    let dir = std::env::temp_dir();
    let folder = reveal_commands(&dir);
    assert_eq!(folder.len(), 1, "a folder needs no selection step: {folder:?}");
    assert_eq!(folder[0].args.last().map(|a| a.as_os_str()), Some(dir.as_os_str()));

    let file = dir.join("resonance-reveal-probe.nam");
    let cmds = reveal_commands(&file);
    assert!(!cmds.is_empty());
    #[cfg(target_os = "linux")]
    {
        assert_eq!(cmds[0].program, "dbus-send", "FileManager1.ShowItems first");
        let last = cmds.last().unwrap();
        assert_eq!(last.program, "xdg-open");
        assert_eq!(last.args, vec![dir.as_os_str().to_owned()], "fallback opens the folder");
    }
    #[cfg(target_os = "macos")]
    assert_eq!(cmds[0].args[0], "-R");
}
