/// Wavetable data structures and runtime loader for the pre-generated bundle.
///
/// Wavetables are generated once at plugin build time by `build.rs` (which
/// `#[path]`-includes `wavetable_gen.rs`) and emitted to `$OUT_DIR/wavetables.bin`.
/// At runtime we `include_bytes!` that blob and hand out *borrowed views* into
/// it — no parsing, no allocation, no copy.
///
/// Storage layout
/// --------------
/// The bundle is ~40 MB. The previous loader parsed it into a
/// `Vec<Wavetable> -> Vec<WavetableFrame> -> Vec<Vec<f32>>` tree **per plugin
/// instance**, which meant:
///
/// * ~36 MB of heap and a 36 MB memcpy for every instance (a 5-instance
///   project paid ~180 MB and ~50 ms of instantiation time);
/// * three pointer hops per oscillator sample to reach the mip level
///   (`frames[f]` -> `mip_levels[o]` -> `[f32]`), each a potential cache miss
///   on the hot path;
/// * five disjoint copies of identical data competing for the 32 MB L3.
///
/// The blob is already little-endian `f32` in exactly the order the engine
/// wants, so it is now aliased in place through an over-aligned wrapper. A
/// [`Wavetable`] is a `(&'static [f32], num_frames)` descriptor — 24 bytes —
/// and mip levels are reached with one multiply-add into a flat slice. Every
/// instance shares the same read-only pages, so the OS keeps a single physical
/// copy for the whole process.
#[cfg(target_endian = "big")]
compile_error!("bundled wavetables assume little-endian f32 layout");

pub const WAVETABLE_SIZE: usize = 2048;
/// Mip levels per frame. Level `k` holds partials up to
/// `22050 / (8.1758 * 2^k)`; the top one (11) is the fundamental alone, the
/// alias-free read above C9 (FU-G2b).
pub const NUM_OCTAVES: usize = 12;
pub const NUM_WAVETABLES: usize = 10;

/// The `oscN_wavetable` value that selects that oscillator's *user* table —
/// one past the bundled ones, so every bundled index keeps its meaning. Only
/// live once a table has been imported: until then the oscillator plays
/// bundled table 0 (see [`crate::dsp::engine::SynthEngine::install_user_table`]).
pub const USER_WAVETABLE_INDEX: usize = NUM_WAVETABLES;

/// Number of `f32`s spanned by all mip levels of one frame.
pub const FRAME_STRIDE: usize = NUM_OCTAVES * WAVETABLE_SIZE;

/// A wavetable: a flat `frames × NUM_OCTAVES × WAVETABLE_SIZE` block of
/// samples borrowed from the embedded bundle.
#[derive(Clone, Copy)]
pub struct Wavetable {
    data: &'static [f32],
    num_frames: usize,
}

impl Wavetable {
    /// A view over caller-owned mip data laid out like the bundle
    /// (`num_frames × NUM_OCTAVES × WAVETABLE_SIZE`) — how a runtime-built
    /// [`crate::dsp::user_table::UserTable`] is read by the same oscillator
    /// code as the bundled tables.
    ///
    /// # Safety
    ///
    /// The descriptor claims `'static` but borrows `data`: it must not be
    /// read after `data` is freed. The engine upholds this by keeping user
    /// views only in its own table slots and overwriting a slot in the same
    /// call that takes its storage out.
    pub unsafe fn from_raw_parts(data: &[f32], num_frames: usize) -> Self {
        assert_eq!(data.len(), num_frames * FRAME_STRIDE, "not a whole number of frames");
        Self {
            // SAFETY: the lifetime extension is the caller's contract above.
            data: unsafe { std::slice::from_raw_parts(data.as_ptr(), data.len()) },
            num_frames,
        }
    }

    #[inline]
    pub fn num_frames(&self) -> usize {
        self.num_frames
    }

    /// One band-limited mip level.
    ///
    /// Returns a fixed-size array reference, not a slice: combined with the
    /// power-of-two index mask in [`crate::dsp::oscillator`], that lets the
    /// optimizer prove every table access in bounds and emit the wrap as a
    /// bare `and` with no bounds check and no length assert.
    #[inline]
    pub fn mip(&self, frame: usize, octave: usize) -> &'static [f32; WAVETABLE_SIZE] {
        let off = frame * FRAME_STRIDE + octave * WAVETABLE_SIZE;
        self.data[off..off + WAVETABLE_SIZE]
            .try_into()
            .expect("mip slice is WAVETABLE_SIZE by construction")
    }
}

/// Wrapper that gives the embedded byte blob `f32` alignment. `include_bytes!`
/// yields a `[u8; N]` with alignment 1, which is not enough to reinterpret as
/// `[f32]`; unsizing a `#[repr(C, align(4))]` newtype fixes that at zero
/// runtime cost.
#[repr(C, align(64))]
struct Aligned<T: ?Sized>(T);

static WAVETABLE_BLOB: &Aligned<[u8]> = &Aligned(*include_bytes!(concat!(
    env!("OUT_DIR"),
    "/wavetables.bin"
)));

/// Build borrowed views over the embedded bundle. Runs once per plugin
/// instance at `initialize()` time and costs a few hundred nanoseconds: it
/// only walks the small header and slices the payload.
pub fn load_bundled() -> Vec<Wavetable> {
    let bytes = &WAVETABLE_BLOB.0;
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

    // The payload starts right after the header. `off` is a multiple of 4
    // (three u32s plus one per table) and the blob itself is 64-byte aligned,
    // so the payload is `f32`-aligned.
    assert_eq!(off % std::mem::align_of::<f32>(), 0);
    let payload = &bytes[off..];
    assert_eq!(
        payload.len() % std::mem::size_of::<f32>(),
        0,
        "wavetables.bin payload is not a whole number of f32s"
    );
    // SAFETY: `payload` starts at a 4-byte-aligned offset into a 64-byte
    // aligned static, its length is a whole number of `f32`s, and every bit
    // pattern is a valid `f32`. The bundle was written as native-endian `f32`
    // bytes by `wavetable_gen.rs` and the file is little-endian-only (asserted
    // at the top of this module). The resulting slice borrows `'static`
    // read-only data.
    let samples: &'static [f32] = unsafe {
        std::slice::from_raw_parts(
            payload.as_ptr() as *const f32,
            payload.len() / std::mem::size_of::<f32>(),
        )
    };

    let mut tables = Vec::with_capacity(num_tables);
    let mut cursor = 0usize;
    for &num_frames in &frame_counts {
        let len = num_frames * FRAME_STRIDE;
        tables.push(Wavetable {
            data: &samples[cursor..cursor + len],
            num_frames,
        });
        cursor += len;
    }

    debug_assert_eq!(cursor, samples.len(), "trailing samples in wavetables.bin");
    tables
}

fn read_u32(bytes: &[u8], off: &mut usize) -> u32 {
    let v = u32::from_le_bytes(bytes[*off..*off + 4].try_into().unwrap());
    *off += 4;
    v
}
