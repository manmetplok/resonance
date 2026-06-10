//! State-machine tests for `SwapFader`: idle, direct install, fade-in
//! from empty, the fade-out -> swap -> fade-in crossfade, and pending
//! replacement mid-fade.

use resonance_dsp::SwapFader;

#[test]
fn idle_with_no_payload_is_unity_gain() {
    let mut fader: SwapFader<u32> = SwapFader::new(4);
    assert!(fader.active().is_none());
    assert!(!fader.is_fading_out());
    for _ in 0..8 {
        let (gain, payload) = fader.next();
        assert_eq!(gain, 1.0);
        assert!(payload.is_none());
    }
}

#[test]
fn install_is_immediate_with_no_fade() {
    let mut fader = SwapFader::new(4);
    fader.install(7u32);
    assert_eq!(fader.active(), Some(&7));
    let (gain, payload) = fader.next();
    assert_eq!(gain, 1.0);
    assert_eq!(payload.copied(), Some(7));
}

#[test]
fn swap_into_empty_fades_in_immediately() {
    let mut fader = SwapFader::new(4);
    fader.begin_swap(1u32);
    assert_eq!(fader.active(), Some(&1));
    assert!(!fader.is_fading_out());
    let gains: Vec<f32> = (0..6).map(|_| fader.next().0).collect();
    assert_eq!(gains, vec![0.25, 0.5, 0.75, 1.0, 1.0, 1.0]);
}

#[test]
fn swap_over_active_crossfades_out_then_in() {
    let mut fader = SwapFader::new(4);
    fader.install(1u32);
    fader.begin_swap(2u32);
    assert!(fader.is_fading_out());

    let log: Vec<(f32, u32)> = (0..9)
        .map(|_| {
            let (gain, payload) = fader.next();
            (gain, payload.copied().unwrap())
        })
        .collect();
    assert_eq!(
        log,
        vec![
            // Fade-out runs on the old payload...
            (0.75, 1),
            (0.5, 1),
            (0.25, 1),
            // ...the swap lands on the silent sample...
            (0.0, 2),
            // ...and the new payload fades in.
            (0.25, 2),
            (0.5, 2),
            (0.75, 2),
            (1.0, 2),
            (1.0, 2),
        ]
    );
}

#[test]
fn second_swap_mid_fade_replaces_pending_and_restarts_fade_out() {
    let mut fader = SwapFader::new(4);
    fader.install(1u32);
    fader.begin_swap(2u32);
    fader.next();
    fader.next();

    // A newer payload arrives before the swap lands: it supersedes the
    // pending one and the fade-out restarts from the top.
    fader.begin_swap(3u32);
    let log: Vec<(f32, u32)> = (0..5)
        .map(|_| {
            let (gain, payload) = fader.next();
            (gain, payload.copied().unwrap())
        })
        .collect();
    assert_eq!(
        log,
        vec![(0.75, 1), (0.5, 1), (0.25, 1), (0.0, 3), (0.25, 3)]
    );
}
