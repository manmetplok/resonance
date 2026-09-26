//! Clip edits that arrive before their clip finished loading (ba doc
//! #276 BUG 1).
//!
//! `LoadClipFromWav` hands the mmap + waveform decimation to a worker
//! thread, so for the first few milliseconds of a clip's life it is NOT
//! in the engine's clip list. Every clip handler follows the engine's
//! "missing lookup ⇒ silent no-op" convention, which is right for a
//! deleted clip and catastrophic for one that simply has not landed yet:
//! a client that placed a clip and immediately trimmed it lost the trim
//! outright.
//!
//! Nothing reported an error. The app mirror had already applied the
//! geometry optimistically, so `clip.trim` returned the right offsets,
//! `song.tracks` reported the right lengths, and the render played every
//! clip's WHOLE source from its placement point — a 112-bar arrangement
//! bounced 44% too long, with a balance pass that had converged to
//! within 0.01 LU against material that was wrong.
//!
//! The fix parks such a command and replays it once the clip appears.
//! These tests pin the queue's rules: order, expiry, and that a landed
//! clip releases exactly its own commands.

use std::time::{Duration, Instant};

use resonance_audio::types::AudioCommand;
use resonance_audio::{partition_deferred_clip_commands, DeferredClipCommand};

const TIMEOUT: Duration = Duration::from_secs(10);

fn parked(clip_id: u64, command: AudioCommand, parked_at: Instant) -> DeferredClipCommand {
    DeferredClipCommand {
        clip_id,
        command,
        parked_at,
    }
}

fn trim(clip_id: u64, trim_start_frames: u64) -> AudioCommand {
    AudioCommand::TrimClip {
        clip_id,
        new_start_sample: 0,
        trim_start_frames,
        trim_end_frames: 0,
    }
}

fn gain(clip_id: u64, gain_db: f32) -> AudioCommand {
    AudioCommand::SetClipGain { clip_id, gain_db }
}

/// A command whose clip has not landed stays parked — it is not run
/// against nothing, and it is not dropped.
#[test]
fn a_command_for_a_missing_clip_waits() {
    let now = Instant::now();
    let mut queue = vec![parked(7, trim(7, 100), now)];

    let (ready, expired) = partition_deferred_clip_commands(&mut queue, |_| false, now, TIMEOUT);

    assert!(ready.is_empty(), "nothing to run yet");
    assert!(expired.is_empty(), "and nothing given up on");
    assert_eq!(queue.len(), 1, "it is still waiting");
}

/// The moment the clip lands, its command runs.
#[test]
fn a_command_runs_once_its_clip_lands() {
    let now = Instant::now();
    let mut queue = vec![parked(7, trim(7, 100), now)];

    let (ready, _) = partition_deferred_clip_commands(&mut queue, |id| id == 7, now, TIMEOUT);

    assert_eq!(ready.len(), 1, "the trim is released");
    assert!(queue.is_empty(), "and leaves the queue");
    match &ready[0] {
        AudioCommand::TrimClip {
            clip_id,
            trim_start_frames,
            ..
        } => {
            assert_eq!(*clip_id, 7);
            assert_eq!(*trim_start_frames, 100, "with its parameters intact");
        }
        other => panic!("expected the parked trim, got {other:?}"),
    }
}

/// Several edits on one clip replay in the order they were made — a trim
/// followed by a gain change must not land the other way round.
#[test]
fn commands_replay_in_the_order_they_were_parked() {
    let now = Instant::now();
    let mut queue = vec![
        parked(7, trim(7, 100), now),
        parked(7, gain(7, -6.0), now),
        parked(7, trim(7, 200), now),
    ];

    let (ready, _) = partition_deferred_clip_commands(&mut queue, |_| true, now, TIMEOUT);

    assert_eq!(ready.len(), 3);
    assert!(matches!(ready[0], AudioCommand::TrimClip { trim_start_frames: 100, .. }));
    assert!(matches!(ready[1], AudioCommand::SetClipGain { .. }));
    assert!(matches!(ready[2], AudioCommand::TrimClip { trim_start_frames: 200, .. }));
}

/// One clip landing releases only its own commands; another clip's stay
/// parked. (The 12-clip repro places and trims twelve clips in a row,
/// each finishing its load at a different moment.)
#[test]
fn one_clip_landing_releases_only_its_own_commands() {
    let now = Instant::now();
    let mut queue = vec![
        parked(1, trim(1, 10), now),
        parked(2, trim(2, 20), now),
        parked(1, gain(1, -3.0), now),
    ];

    let (ready, _) = partition_deferred_clip_commands(&mut queue, |id| id == 1, now, TIMEOUT);

    assert_eq!(ready.len(), 2, "both of clip 1's edits");
    assert_eq!(queue.len(), 1, "clip 2's edit keeps waiting");
    assert_eq!(queue[0].clip_id, 2);
}

/// A clip that never arrives (its load failed) must not leave commands
/// queued forever — they expire and are reported, rather than firing
/// minutes later against a recycled id.
#[test]
fn a_command_expires_when_its_clip_never_arrives() {
    let parked_at = Instant::now();
    let mut queue = vec![parked(7, trim(7, 100), parked_at)];

    let later = parked_at + TIMEOUT + Duration::from_millis(1);
    let (ready, expired) = partition_deferred_clip_commands(&mut queue, |_| false, later, TIMEOUT);

    assert!(ready.is_empty());
    assert_eq!(expired, vec![7], "the caller is told which clip was lost");
    assert!(queue.is_empty());
}

/// Just before the deadline it is still waiting — expiry is a timeout,
/// not a race with the loader.
#[test]
fn a_command_just_inside_the_deadline_still_waits() {
    let parked_at = Instant::now();
    let mut queue = vec![parked(7, trim(7, 100), parked_at)];

    let nearly = parked_at + TIMEOUT - Duration::from_millis(1);
    let (ready, expired) = partition_deferred_clip_commands(&mut queue, |_| false, nearly, TIMEOUT);

    assert!(ready.is_empty() && expired.is_empty());
    assert_eq!(queue.len(), 1);
}
