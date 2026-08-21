//! Debug logging / instrumentation helpers, mirroring the Wayland
//! runtime's `WPG_*` hooks with a `CPG_*` prefix:
//!
//! - `CPG_DUMP_FRAME=<path>` writes a painted frame to `<path>` as a PPM
//!   (`CPG_DUMP_FRAME_AT=<n>` picks a later, settled frame; default 1).
//! - `CPG_TEST_CLOSE_AT=<n>` performs a window close (through the real
//!   `performClose:` → `windowShouldClose:` → `on_close` path) once the
//!   n-th frame has painted, so the close plumbing can be verified without
//!   an external click injector. No effect unless set.

/// Same PPM writer as the Wayland runtime's, minus the row flip: GL
/// `ReadPixels` is bottom-up there too, so rows are written in reverse.
pub(super) fn dump_ppm(path: &str, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    write!(f, "P6\n{} {}\n255\n", w, h)?;
    for y in (0..h).rev() {
        let row = (y * w * 4) as usize;
        for x in 0..w {
            let i = row + (x as usize * 4);
            f.write_all(&rgba[i..i + 3])?;
        }
    }
    Ok(())
}
