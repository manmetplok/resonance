//! Debug logging / instrumentation helpers.
//!
//! Opt-in via the `WPG_DUMP_FRAME` environment variable; the first frame after
//! startup is written to the given path as a PPM image.

#[allow(dead_code)]
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
