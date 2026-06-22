//! Tests for the reference-A/B monitor tap (todo #693).
//!
//! Exercises the audio-thread [`ReferenceMonitor`] together with the
//! control-thread publish path ([`ReferencePlayer::publish`]) that feeds
//! it: switching the monitored source swaps the output buffer between the
//! mix (untouched) and the reference PCM, the gain/cursor/loop-to-mix
//! knobs apply, and a free-running cursor advances + wraps. All headless —
//! no cpal stream, no engine thread.

use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::unbounded;

use resonance_audio::types::{ABSource, AudioEvent, ReferenceId};
use resonance_audio::{
    handle_reference_analyzed, handle_set_ab_source, handle_set_active_reference,
    handle_set_ref_loop_to_mix, handle_set_ref_trim, register_reference, ReferenceMonitor,
    ReferencePlayer,
};

/// Build a player whose single registered reference is active and carries
/// the given decoded interleaved-stereo `pcm`.
fn player_with_active_pcm(pcm: Vec<f32>) -> ReferencePlayer {
    let mut player = ReferencePlayer::new();
    let (tx, _rx) = unbounded::<AudioEvent>();
    let id = register_reference(&mut player, Some(ReferenceId(1)), PathBuf::from("/ref.wav"));
    handle_set_active_reference(&mut player, &tx, id);
    handle_reference_analyzed(&mut player, id, Arc::new(pcm), -14.0);
    player
}

/// Per-frame ramp PCM (L == R == frame * 0.1) so each rendered frame is
/// identifiable by value.
fn ramp_pcm(frames: usize) -> Vec<f32> {
    let mut v = Vec::with_capacity(frames * 2);
    for f in 0..frames {
        let s = f as f32 * 0.1;
        v.push(s);
        v.push(s);
    }
    v
}

#[test]
fn mix_source_leaves_the_buffer_untouched() {
    let player = player_with_active_pcm(vec![0.5; 8]);
    let monitor = ReferenceMonitor::default();
    // ab_source defaults to Mix.
    player.publish(&monitor, true);

    let mut buf = vec![1.0f32; 8];
    assert!(
        !monitor.render(&mut buf, 2, 4, 0),
        "Mix source must not engage the reference"
    );
    assert_eq!(buf, vec![1.0f32; 8], "buffer (the mix) must be preserved");
}

#[test]
fn switching_ab_source_mid_monitor_swaps_the_buffer() {
    let mut player = player_with_active_pcm(vec![0.5; 8]); // 4 frames, all 0.5
    let monitor = ReferenceMonitor::default();
    let (tx, _rx) = unbounded::<AudioEvent>();

    // Start on the mix: the processed mix passes through untouched.
    player.publish(&monitor, false);
    let mut buf = vec![1.0f32; 8];
    assert!(!monitor.render(&mut buf, 2, 4, 0));
    assert_eq!(buf, vec![1.0f32; 8]);

    // Flip to the reference: the buffer is replaced with the reference PCM.
    handle_set_ab_source(&mut player, &tx, ABSource::Reference);
    player.publish(&monitor, false);
    assert!(monitor.render(&mut buf, 2, 4, 0));
    assert!(
        buf.iter().all(|&s| (s - 0.5).abs() < 1e-6),
        "reference PCM must replace the output, got {buf:?}"
    );

    // Flip back to the mix: the reference is dropped again.
    handle_set_ab_source(&mut player, &tx, ABSource::Mix);
    player.publish(&monitor, false);
    let mut buf2 = vec![1.0f32; 8];
    assert!(!monitor.render(&mut buf2, 2, 4, 0));
    assert_eq!(buf2, vec![1.0f32; 8]);
}

#[test]
fn free_run_cursor_advances_and_wraps() {
    let mut player = player_with_active_pcm(ramp_pcm(6)); // 6 frames
    let monitor = ReferenceMonitor::default();
    let (tx, _rx) = unbounded::<AudioEvent>();
    handle_set_ab_source(&mut player, &tx, ABSource::Reference);
    player.publish(&monitor, true); // cursor reset to 0

    // First block: frames 0..4 -> 0.0, 0.1, 0.2, 0.3.
    let mut buf = vec![0.0f32; 8];
    assert!(monitor.render(&mut buf, 2, 4, 0));
    let left: Vec<f32> = (0..4).map(|f| buf[f * 2]).collect();
    assert!(
        left.iter()
            .zip([0.0, 0.1, 0.2, 0.3])
            .all(|(a, b)| (a - b).abs() < 1e-6),
        "got {left:?}"
    );
    assert_eq!(monitor.cursor_for_test(), 4);

    // Second block wraps: frames 4, 5, 0, 1 -> 0.4, 0.5, 0.0, 0.1.
    assert!(monitor.render(&mut buf, 2, 4, 0));
    let left: Vec<f32> = (0..4).map(|f| buf[f * 2]).collect();
    assert!(
        left.iter()
            .zip([0.4, 0.5, 0.0, 0.1])
            .all(|(a, b)| (a - b).abs() < 1e-6),
        "got {left:?}"
    );
    assert_eq!(monitor.cursor_for_test(), 2, "cursor wraps modulo length");
}

#[test]
fn loop_to_mix_reads_from_the_playhead_and_holds_the_cursor() {
    let mut player = player_with_active_pcm(ramp_pcm(6));
    let monitor = ReferenceMonitor::default();
    let (tx, _rx) = unbounded::<AudioEvent>();
    handle_set_ab_source(&mut player, &tx, ABSource::Reference);
    handle_set_ref_loop_to_mix(&mut player, &tx, true);
    player.publish(&monitor, true);

    // Playhead at frame 3 -> reads reference frames 3, 4 (0.3, 0.4).
    let mut buf = vec![0.0f32; 4];
    assert!(monitor.render(&mut buf, 2, 2, 3));
    assert!((buf[0] - 0.3).abs() < 1e-6 && (buf[2] - 0.4).abs() < 1e-6, "got {buf:?}");
    // The free-run cursor is untouched in loop-to-mix mode.
    assert_eq!(monitor.cursor_for_test(), 0);
}

#[test]
fn trim_gain_scales_and_clamps_the_reference() {
    let mut player = player_with_active_pcm(vec![0.3, 0.3, 0.8, 0.8]); // 2 frames
    let monitor = ReferenceMonitor::default();
    let (tx, _rx) = unbounded::<AudioEvent>();
    handle_set_ab_source(&mut player, &tx, ABSource::Reference);
    // +6.0206 dB ~= linear gain 2.0.
    handle_set_ref_trim(&mut player, &tx, 6.020_6);
    player.publish(&monitor, true);

    let mut buf = vec![0.0f32; 4];
    assert!(monitor.render(&mut buf, 2, 2, 0));
    // Frame 0: 0.3 * 2 = 0.6. Frame 1: 0.8 * 2 = 1.6 -> clamped to 1.0.
    assert!((buf[0] - 0.6).abs() < 1e-3, "frame 0 gain, got {}", buf[0]);
    assert!((buf[2] - 1.0).abs() < 1e-6, "frame 1 must clamp, got {}", buf[2]);
}

#[test]
fn reference_active_but_undecoded_stays_on_the_mix() {
    // Active reference with no decoded PCM yet (decode still in flight):
    // the monitor must not engage, so the user keeps hearing the mix.
    let mut player = ReferencePlayer::new();
    let (tx, _rx) = unbounded::<AudioEvent>();
    let id = register_reference(&mut player, Some(ReferenceId(1)), PathBuf::from("/ref.wav"));
    handle_set_active_reference(&mut player, &tx, id);
    handle_set_ab_source(&mut player, &tx, ABSource::Reference);
    let monitor = ReferenceMonitor::default();
    player.publish(&monitor, true);

    let mut buf = vec![1.0f32; 8];
    assert!(
        !monitor.render(&mut buf, 2, 4, 0),
        "no decoded PCM -> reference must not engage"
    );
    assert_eq!(buf, vec![1.0f32; 8]);
}
