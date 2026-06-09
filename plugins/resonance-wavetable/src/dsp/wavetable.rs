/// Wavetable data structures and runtime loader for the pre-generated bundle.
///
/// Wavetables are generated once at plugin build time by `build.rs` (which
/// `#[path]`-includes `wavetable_gen.rs`) and emitted to `$OUT_DIR/wavetables.bin`.
/// At runtime we `include_bytes!` that blob and parse it back into the same
/// nested `Vec<Wavetable>` shape the rest of the engine expects — no additive
/// synthesis, no `sin()` calls, just a handful of allocations and a byte copy.
///
/// The bundled file was generated for 44.1 kHz and is reused for every host
/// sample rate. Band-limiting done at 44.1 is safe for any higher rate (no
/// aliasing risk); the only trade-off is a slightly duller top octave when
/// running projects at 96 kHz, which is inaudible in practice.

#[cfg(target_endian = "big")]
compile_error!("bundled wavetables assume little-endian f32 layout");

pub const WAVETABLE_SIZE: usize = 2048;
pub const NUM_OCTAVES: usize = 11;
pub const NUM_WAVETABLES: usize = 10;

/// One single-cycle waveform with mip-mapped octave levels.
pub struct WavetableFrame {
    /// mip_levels[octave][sample] -- band-limited version per octave.
    pub mip_levels: Vec<Vec<f32>>,
}

/// A wavetable: a collection of frames that can be scanned with a position parameter.
pub struct Wavetable {
    pub frames: Vec<WavetableFrame>,
}

static WAVETABLE_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wavetables.bin"));

/// Parse the embedded bundle into `Vec<Wavetable>`. Runs once per plugin
/// instance at `initialize()` time; typical cost is a few milliseconds.
pub fn load_bundled() -> Vec<Wavetable> {
    let bytes = WAVETABLE_BYTES;
    let mut off = 0usize;

    let wavetable_size = read_u32(bytes, &mut off) as usize;
    let num_octaves = read_u32(bytes, &mut off) as usize;
    let num_tables = read_u32(bytes, &mut off) as usize;
    assert_eq!(
        wavetable_size, WAVETABLE_SIZE,
        "bundled WAVETABLE_SIZE mismatch — rebuild the wavetable plugin"
    );
    assert_eq!(
        num_octaves, NUM_OCTAVES,
        "bundled NUM_OCTAVES mismatch — rebuild the wavetable plugin"
    );
    assert_eq!(
        num_tables, NUM_WAVETABLES,
        "bundled NUM_WAVETABLES mismatch — rebuild the wavetable plugin"
    );

    let mut frame_counts = Vec::with_capacity(num_tables);
    for _ in 0..num_tables {
        frame_counts.push(read_u32(bytes, &mut off) as usize);
    }

    let bytes_per_mip = WAVETABLE_SIZE * std::mem::size_of::<f32>();

    let mut tables = Vec::with_capacity(num_tables);
    for &num_frames in &frame_counts {
        let mut frames = Vec::with_capacity(num_frames);
        for _ in 0..num_frames {
            let mut mip_levels = Vec::with_capacity(NUM_OCTAVES);
            for _ in 0..NUM_OCTAVES {
                let mut mip = vec![0.0f32; WAVETABLE_SIZE];
                // SAFETY: on little-endian targets (asserted above), the raw
                // f32 byte layout written by `wavetable_gen.rs` is identical
                // to the in-memory layout of `[f32]`. We copy the exact
                // number of bytes we allocated.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr().add(off),
                        mip.as_mut_ptr() as *mut u8,
                        bytes_per_mip,
                    );
                }
                off += bytes_per_mip;
                mip_levels.push(mip);
            }
            frames.push(WavetableFrame { mip_levels });
        }
        tables.push(Wavetable { frames });
    }

    debug_assert_eq!(off, bytes.len(), "trailing bytes in wavetables.bin");
    tables
}

fn read_u32(bytes: &[u8], off: &mut usize) -> u32 {
    let v = u32::from_le_bytes(bytes[*off..*off + 4].try_into().unwrap());
    *off += 4;
    v
}
