/// Scan a directory for files with a given extension, returning sorted absolute paths.
///
/// Returns an empty list when the directory cannot be read. Every caller
/// (plugin model browsers and rescan paths) treats that as "no files" and
/// continues, so an `io::Result` return would only push log-and-continue
/// boilerplate to each site; instead the error is logged here via
/// `eprintln!`, the workspace's logging convention. `NotFound` is not
/// logged — a models directory that hasn't been created yet is a normal
/// pre-first-download state, not an error.
pub fn scan_directory(dir: &std::path::Path, extension: &str) -> Vec<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("scan_directory: {}: {e}", dir.display());
            }
            return Vec::new();
        }
    };
    let mut files: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext.eq_ignore_ascii_case(extension))
                .unwrap_or(false)
        })
        .map(|e| e.path().to_string_lossy().into_owned())
        .collect();
    files.sort();
    files
}
