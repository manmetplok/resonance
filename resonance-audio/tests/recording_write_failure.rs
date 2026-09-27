//! A WAV write error mid-recording must keep what reached the disk
//! (code review ENG-03).
//!
//! The drain used to drop the take's writer on the first failed write,
//! and `finalize_recording` then read "writer already closed" as a
//! corrupt file: no clip, no `RecordingFinished`, only an `eprintln!` —
//! a 19-minute take that hit "Disk quota exceeded" at minute 19 simply
//! vanished from the project.
//!
//! The failure is a real kernel one: `RLIMIT_FSIZE` caps how large this
//! process may grow a file, so writes past the cap fail with `EFBIG`
//! exactly like a full disk fails with `ENOSPC` (including the partial
//! last write). The limit is process-wide, which is why these tests live
//! in their own binary and serialize on a lock.
#![cfg(target_os = "linux")]

use std::path::PathBuf;
use std::sync::Mutex;

use crossbeam_channel::{unbounded, Receiver};
use ringbuf::traits::{Producer, Split};
use ringbuf::{HeapProd, HeapRb};

use resonance_audio::types::{AudioClip, AudioEvent};
use resonance_audio::RecordingState;

static FSIZE_LOCK: Mutex<()> = Mutex::new(());

const SR: u32 = 48_000;
/// Bytes this process may write into any one file while the cap is on:
/// the WAV header plus a little over 10 000 stereo f32 frames, and an
/// odd byte so the cut lands mid-frame.
const FILE_CAP: u64 = 80_000 + 83;

/// Run `f` with `RLIMIT_FSIZE` lowered to `FILE_CAP` (and `SIGXFSZ`
/// ignored, so an over-cap write returns `EFBIG` instead of killing the
/// process), restoring both afterwards.
fn with_file_cap<T>(f: impl FnOnce() -> T) -> T {
    unsafe {
        let mut old = sys::Rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        assert_eq!(sys::getrlimit(sys::RLIMIT_FSIZE, &mut old), 0);
        sys::signal(sys::SIGXFSZ, sys::SIG_IGN);
        let capped = sys::Rlimit {
            rlim_cur: FILE_CAP,
            rlim_max: old.rlim_max,
        };
        assert_eq!(sys::setrlimit(sys::RLIMIT_FSIZE, &capped), 0);
        let out = f();
        assert_eq!(sys::setrlimit(sys::RLIMIT_FSIZE, &old), 0);
        out
    }
}

/// The three libc calls this needs, declared directly: `libc` is not a
/// Linux dependency of this crate, and std already links the C library.
/// Constants are the Linux values (identical on x86_64 and aarch64).
mod sys {
    #[repr(C)]
    pub struct Rlimit {
        pub rlim_cur: u64,
        pub rlim_max: u64,
    }
    pub const RLIMIT_FSIZE: i32 = 1;
    pub const SIGXFSZ: i32 = 25;
    pub const SIG_IGN: usize = 1;
    extern "C" {
        pub fn getrlimit(resource: i32, rlim: *mut Rlimit) -> i32;
        pub fn setrlimit(resource: i32, rlim: *const Rlimit) -> i32;
        pub fn signal(signum: i32, handler: usize) -> usize;
    }
}

fn make_tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-rec-fail-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A recording session on track 7 writing `clip_<clip_id>.wav`.
fn session(project_dir: &PathBuf, clip_id: u64) -> (RecordingState, HeapProd<f32>) {
    let mut rec = RecordingState::new(SR);
    let ring: HeapRb<f32> = HeapRb::new(SR as usize * 2 * 4);
    let (prod, cons) = ring.split();
    rec.ring_consumer = Some(cons);
    rec.input_channels = 2;
    rec.input_sample_rate = SR;
    rec.start_sample = 1_000;
    let buf =
        RecordingState::create_track_buf(project_dir, 7, clip_id, SR, SR, 0, false).unwrap();
    rec.buffers.insert(7, buf);
    (rec, prod)
}

fn sample(frame: usize) -> f32 {
    ((frame % 1000) as f32) * 1e-3
}

/// Push `frames` frames of the ramp starting at `first`, draining every
/// 1000 frames like the engine loop does.
fn feed(rec: &mut RecordingState, prod: &mut HeapProd<f32>, first: usize, frames: usize) {
    let mut chunk = Vec::with_capacity(2000);
    let mut f = first;
    while f < first + frames {
        chunk.clear();
        let n = 1000.min(first + frames - f);
        for i in 0..n {
            chunk.push(sample(f + i));
            chunk.push(-sample(f + i));
        }
        assert_eq!(prod.push_slice(&chunk), chunk.len());
        rec.drain_ring_to_buffers();
        f += n;
    }
}

fn errors(rx: &Receiver<AudioEvent>) -> Vec<String> {
    rx.try_iter()
        .filter_map(|e| match e {
            AudioEvent::Error(err) => Some(err.message),
            _ => None,
        })
        .collect()
}

fn frames_of(clip: &AudioClip) -> Vec<(f32, f32)> {
    let frames = clip.source.as_frames();
    frames.chunks_exact(2).map(|c| (c[0], c[1])).collect()
}

#[test]
fn write_failure_keeps_the_audio_on_disk_and_reports_it() {
    let _g = FSIZE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = make_tempdir("finalize");
    let (tx, rx) = unbounded();
    let (mut rec, mut prod) = session(&dir, 1);

    // One second of input against a cap of ~10 000 frames.
    with_file_cap(|| feed(&mut rec, &mut prod, 0, SR as usize));

    rec.poll_write_errors(&tx);
    let reported = errors(&rx);
    assert_eq!(reported.len(), 1, "one error per failed take: {reported:?}");
    assert!(reported[0].contains("clip_1.wav"), "{}", reported[0]);

    let mut clips: Vec<std::sync::Arc<resonance_audio::types::AudioClip>> = Vec::new();
    let emitted = rec.finalize_recording(SR, &mut clips, &tx);
    assert_eq!(emitted, 1, "the salvaged take must become a clip");
    assert!(errors(&rx).is_empty(), "the failure is reported only once");

    let clips = clips;
    let got = frames_of(&clips[0]);
    assert!(
        (9_000..=10_100).contains(&got.len()),
        "clip should hold the ~10 000 frames that fit under the cap, got {}",
        got.len()
    );
    for (i, &(l, r)) in got.iter().enumerate() {
        assert_eq!((l, r), (sample(i), -sample(i)), "frame {i} corrupted");
    }
    assert_eq!(clips[0].start_sample, 1_000);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_failure_in_a_cycle_record_pass_keeps_that_take_and_the_next() {
    let _g = FSIZE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = make_tempdir("roll");
    let audio_dir = dir.join("audio");
    let (tx, rx) = unbounded();
    let (mut rec, mut prod) = session(&dir, 1);
    let mut clips: Vec<std::sync::Arc<resonance_audio::types::AudioClip>> = Vec::new();
    let mut next_clip_id = 2;

    with_file_cap(|| feed(&mut rec, &mut prod, 0, SR as usize));
    // Loop seam: the damaged pass rolls into a take, a fresh writer opens.
    let first = rec.roll_audio_pass(SR, 0, &mut clips, &audio_dir, &mut next_clip_id, true);
    assert_eq!(first.len(), 1, "salvaged pass must still produce a take");
    assert!((9_000..=10_100).contains(&first[0].duration_samples));

    // The next pass records normally into its own file.
    feed(&mut rec, &mut prod, 0, 20_000);
    let second = rec.roll_audio_pass(SR, 0, &mut clips, &audio_dir, &mut next_clip_id, false);
    assert_eq!(second.len(), 1);
    assert_ne!(second[0].clip_id, first[0].clip_id);
    assert_eq!(second[0].duration_samples, 20_000);

    rec.poll_write_errors(&tx);
    assert_eq!(errors(&rx).len(), 1);
    assert_eq!(clips.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// When even the in-place header repair fails (on a copy-on-write
/// filesystem that is full, overwriting a header block needs free space
/// too), the take must still survive: its frames are read back into
/// memory, it becomes a clip, and the error says the header was not
/// repaired (FU-F2b). The repair failure is made real by taking write
/// permission away from the file: the recorder's own descriptor keeps
/// writing, but the repair's fresh read-write open is refused.
#[test]
fn failed_header_repair_keeps_the_take_in_memory_and_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let _g = FSIZE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = make_tempdir("header");
    let (tx, rx) = unbounded();
    let (mut rec, mut prod) = session(&dir, 1);
    let wav = dir.join("audio").join("clip_1.wav");
    std::fs::set_permissions(&wav, std::fs::Permissions::from_mode(0o444)).unwrap();
    if std::fs::OpenOptions::new().write(true).open(&wav).is_ok() {
        // Running as root: permissions do not bind, nothing to test.
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }

    with_file_cap(|| feed(&mut rec, &mut prod, 0, SR as usize));

    rec.poll_write_errors(&tx);
    let reported = errors(&rx);
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert!(
        reported[0].contains("header could not be repaired"),
        "{}",
        reported[0]
    );

    let mut clips: Vec<std::sync::Arc<resonance_audio::types::AudioClip>> = Vec::new();
    let emitted = rec.finalize_recording(SR, &mut clips, &tx);
    assert_eq!(emitted, 1, "the take must survive a failed header repair");
    let clips = clips;
    assert!(clips[0].source.mapped_path().is_none(), "held in memory");
    let got = frames_of(&clips[0]);
    assert!(
        (9_000..=10_100).contains(&got.len()),
        "clip should hold the ~10 000 frames that reached the disk, got {}",
        got.len()
    );
    for (i, &(l, r)) in got.iter().enumerate() {
        assert_eq!((l, r), (sample(i), -sample(i)), "frame {i} corrupted");
    }
    assert!(wav.exists(), "the raw file is never deleted");
    let _ = std::fs::remove_dir_all(&dir);
}
