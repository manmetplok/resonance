//! Shared synthetic signal generators for the integration tests.

use std::f32::consts::TAU;

/// Generate a mono 1 kHz sine at the given dBFS amplitude for `secs`
/// seconds. Returns (left, right). For a "mono" test, both channels are
/// identical.
pub fn sine_mono(sr: f32, freq: f32, dbfs: f32, secs: f32) -> (Vec<f32>, Vec<f32>) {
    let amp = 10.0_f32.powf(dbfs / 20.0);
    let n = (sr * secs) as usize;
    let mut l = vec![0.0_f32; n];
    let mut r = vec![0.0_f32; n];
    for i in 0..n {
        let s = (i as f32 / sr * freq * TAU).sin() * amp;
        l[i] = s;
        r[i] = s;
    }
    (l, r)
}

/// Concatenate two stereo buffers.
pub fn concat(a: (Vec<f32>, Vec<f32>), b: (Vec<f32>, Vec<f32>)) -> (Vec<f32>, Vec<f32>) {
    let mut l = a.0;
    let mut r = a.1;
    l.extend(b.0);
    r.extend(b.1);
    (l, r)
}
