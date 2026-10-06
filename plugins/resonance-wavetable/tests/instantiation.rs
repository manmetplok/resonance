//! Instantiation cost: a project with several wavetable tracks builds one
//! `SynthEngine` per plugin instance, and each used to parse the ~36 MB
//! wavetable bundle into its own heap tree.
//!
//! The tables are now borrowed from the embedded blob, so `initialize()` does
//! no bulk copying and every instance shares the same read-only pages. These
//! tests pin that property: they would fail loudly if someone reintroduced a
//! per-instance copy.

use std::time::Instant;

use resonance_wavetable::dsp::engine::SynthEngine;

const SR: f32 = 48_000.0;

/// Parsing the bundle into per-instance `Vec`s took tens of milliseconds and
/// ~36 MB of allocation. Borrowing it is sub-millisecond. The bound is
/// deliberately loose (a 40x margin over what this actually costs) so the test
/// is not flaky under load, while still catching a reintroduced bulk copy.
#[test]
fn initialize_is_cheap() {
    // Warm the page cache / first-touch faults.
    let mut warm = SynthEngine::new();
    warm.initialize(SR);

    let start = Instant::now();
    const N: usize = 8;
    let mut engines = Vec::with_capacity(N);
    for _ in 0..N {
        let mut e = SynthEngine::new();
        e.initialize(SR);
        engines.push(e);
    }
    let per_instance = start.elapsed() / N as u32;

    eprintln!("initialize() = {per_instance:?} per instance");
    assert!(
        per_instance.as_millis() < 20,
        "initialize() took {per_instance:?} per instance — did the wavetable \
         bundle go back to being copied per instance?"
    );
}

/// Every instance must hand out views over the *same* backing memory. If a
/// future change reintroduces per-instance table storage, the pointers will
/// diverge and this fails.
#[test]
fn instances_share_wavetable_storage() {
    let mut a = SynthEngine::new();
    a.initialize(SR);
    let mut b = SynthEngine::new();
    b.initialize(SR);

    assert!(!a.wavetables.is_empty());
    assert_eq!(a.wavetables.len(), b.wavetables.len());

    for (ta, tb) in a.wavetables.iter().zip(b.wavetables.iter()) {
        assert_eq!(ta.num_frames(), tb.num_frames());
        assert_eq!(
            ta.mip(0, 0).as_ptr(),
            tb.mip(0, 0).as_ptr(),
            "wavetable storage is not shared between instances"
        );
    }
}

/// DSP2-16: parameter ids and names built at runtime are interned, so a
/// second instance reuses the first one's strings instead of leaking its
/// own copy of every one.
#[test]
fn param_ids_and_names_are_shared_between_instances() {
    use resonance_wavetable::params::{WavetableParams, PARAM_COUNT};
    let a = WavetableParams::new();
    let b = WavetableParams::new();
    for i in 0..PARAM_COUNT {
        let (pa, pb) = (a.param_at(i), b.param_at(i));
        assert_eq!(pa.id().as_ptr(), pb.id().as_ptr(), "id {} leaked again", pa.id());
        assert_eq!(pa.name().as_ptr(), pb.name().as_ptr(), "name {} leaked again", pa.name());
    }
}

/// The bundle is file-backed and demand-paged: a cold page read by a note is
/// a page fault on the audio thread (field report 2026-10-06 §1). After
/// `initialize()` every page of it must be resident, and stay so under
/// reclaim — a freshly built test binary sits in the page cache anyway, so
/// residency alone would pass vacuously. So the test drops the range from
/// this process's page tables and the page cache first; only `mlock`ed
/// pages survive that.
#[cfg(target_os = "linux")]
#[test]
fn bundle_is_resident_after_initialize() {
    let mut e = SynthEngine::new();
    e.initialize(SR);

    let bytes = resonance_wavetable::dsp::wavetable::bundle_bytes();
    // SAFETY: sysconf has no preconditions.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as usize;
    let start = bytes.as_ptr() as usize & !(page - 1);
    let len = (bytes.as_ptr() as usize + bytes.len()).next_multiple_of(page) - start;

    // Without a memlock allowance `initialize()` only prefaults, which
    // reclaim may undo by design; check plain residency then.
    let mut lim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `lim` is a valid out-pointer.
    unsafe { libc::getrlimit(libc::RLIMIT_MEMLOCK, &mut lim) };
    if lim.rlim_cur == libc::RLIM_INFINITY || lim.rlim_cur as usize >= len {
        evict(start, len);
    }

    let mut vec = vec![0u8; len / page];
    // SAFETY: the range spans mapped pages and `vec` has one entry per page.
    let rc = unsafe { libc::mincore(start as *mut libc::c_void, len, vec.as_mut_ptr()) };
    assert_eq!(rc, 0, "mincore: {}", std::io::Error::last_os_error());
    let cold = vec.iter().filter(|&&v| v & 1 == 0).count();
    assert_eq!(cold, 0, "{cold} of {} bundle pages not resident", vec.len());
}

/// Best-effort reclaim of `[start, start + len)`, a range of this binary's
/// file mapping: unmap it from our page tables, then drop it from the page
/// cache through the file offset `/proc/self/maps` gives for it.
#[cfg(target_os = "linux")]
fn evict(start: usize, len: usize) {
    use std::os::fd::AsRawFd;

    // SAFETY: advisory only; the range is mapped read-only static data.
    unsafe { libc::madvise(start as *mut libc::c_void, len, libc::MADV_PAGEOUT) };
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
    let (lo, off) = maps
        .lines()
        .find_map(|l| {
            let mut f = l.split_whitespace();
            let (lo, hi) = f.next()?.split_once('-')?;
            let (lo, hi) = (
                usize::from_str_radix(lo, 16).ok()?,
                usize::from_str_radix(hi, 16).ok()?,
            );
            let off = usize::from_str_radix(f.nth(1)?, 16).ok()?;
            (lo <= start && start < hi).then_some((lo, off))
        })
        .expect("bundle range not in /proc/self/maps");
    let exe = std::fs::File::open("/proc/self/exe").unwrap();
    let file_off = (off + start - lo) as libc::off_t;
    // SAFETY: advisory only, on a file descriptor we own.
    unsafe {
        libc::posix_fadvise(
            exe.as_raw_fd(),
            file_off,
            len as libc::off_t,
            libc::POSIX_FADV_DONTNEED,
        )
    };
}
