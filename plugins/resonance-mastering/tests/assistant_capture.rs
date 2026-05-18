use resonance_mastering::assistant::capture::CaptureBuffer;

#[test]
fn push_then_snapshot_is_chronological() {
    let mut c = CaptureBuffer::new(8, 48_000.0);
    let l = [1.0, 2.0, 3.0, 4.0];
    let r = [-1.0, -2.0, -3.0, -4.0];
    c.push(&l, &r);
    let (ls, rs) = c.snapshot_chrono();
    assert_eq!(ls, &[1.0, 2.0, 3.0, 4.0]);
    assert_eq!(rs, &[-1.0, -2.0, -3.0, -4.0]);
}

#[test]
fn wrap_around_yields_most_recent() {
    let mut c = CaptureBuffer::new(4, 48_000.0);
    for i in 1..=10 {
        c.push(&[i as f32], &[-(i as f32)]);
    }
    // Should contain the last 4: 7, 8, 9, 10.
    let (ls, _) = c.snapshot_chrono();
    assert_eq!(ls, &[7.0, 8.0, 9.0, 10.0]);
}

#[test]
fn clear_resets_everything() {
    let mut c = CaptureBuffer::new(4, 48_000.0);
    c.push(&[1.0, 2.0], &[3.0, 4.0]);
    c.clear();
    let (ls, rs) = c.snapshot_chrono();
    assert!(ls.is_empty());
    assert!(rs.is_empty());
}
