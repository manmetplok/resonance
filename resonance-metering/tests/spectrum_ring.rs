use resonance_metering::spectrum::ring::SpscRing;

#[test]
fn push_and_pop_round_trip() {
    let ring = SpscRing::new(16);
    for i in 0..10 {
        ring.push(i as f32);
    }
    assert_eq!(ring.available(), 10);
    let mut dst = [0.0_f32; 16];
    let n = ring.pop_into(&mut dst);
    assert_eq!(n, 10);
    for (i, v) in dst.iter().take(10).enumerate() {
        assert_eq!(*v, i as f32);
    }
    assert_eq!(ring.available(), 0);
}

#[test]
fn full_ring_drops_new_pushes() {
    let ring = SpscRing::new(4);
    let mut accepted = 0;
    for i in 0..10 {
        if ring.push(i as f32) {
            accepted += 1;
        }
    }
    assert_eq!(accepted, 4);
    // The first four samples are the ones that made it in.
    let mut dst = [0.0_f32; 4];
    let n = ring.pop_into(&mut dst);
    assert_eq!(n, 4);
    assert_eq!(dst, [0.0, 1.0, 2.0, 3.0]);
}

#[test]
fn wraps_around_zero() {
    let ring = SpscRing::new(8);
    // Fill, drain, fill again — exercises wrap arithmetic.
    for i in 0..6 {
        ring.push(i as f32);
    }
    let mut dst = [0.0_f32; 8];
    let n = ring.pop_into(&mut dst);
    assert_eq!(n, 6);
    for i in 10..14 {
        ring.push(i as f32);
    }
    let n = ring.pop_into(&mut dst);
    assert_eq!(n, 4);
    assert_eq!(&dst[..4], &[10.0, 11.0, 12.0, 13.0]);
}
