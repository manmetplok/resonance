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
fn push_slice_matches_per_sample_push() {
    // Same input stream through the bulk path and the per-sample path,
    // interleaved with partial drains, must yield identical output.
    let bulk = SpscRing::new(16);
    let single = SpscRing::new(16);
    let input: Vec<f32> = (0..40).map(|i| i as f32 * 0.5).collect();

    let mut out_bulk = Vec::new();
    let mut out_single = Vec::new();
    let mut dst = [0.0_f32; 7];
    for chunk in input.chunks(5) {
        let n_bulk = bulk.push_slice(chunk);
        let mut n_single = 0;
        for &s in chunk {
            if single.push(s) {
                n_single += 1;
            }
        }
        assert_eq!(n_bulk, n_single);
        let n = bulk.pop_into(&mut dst);
        out_bulk.extend_from_slice(&dst[..n]);
        let n = single.pop_into(&mut dst);
        out_single.extend_from_slice(&dst[..n]);
    }
    assert_eq!(out_bulk, out_single);
}

#[test]
fn push_slice_truncates_at_capacity() {
    let ring = SpscRing::new(8);
    let input: Vec<f32> = (0..12).map(|i| i as f32).collect();
    let n = ring.push_slice(&input);
    assert_eq!(n, 8);
    // Further pushes are dropped entirely.
    assert_eq!(ring.push_slice(&[99.0]), 0);
    let mut dst = [0.0_f32; 8];
    assert_eq!(ring.pop_into(&mut dst), 8);
    assert_eq!(&dst, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
}

#[test]
fn push_slice_wraps_across_buffer_end() {
    let ring = SpscRing::new(8);
    // Advance the indices so the next bulk write straddles the wrap.
    ring.push_slice(&[0.0; 6]);
    let mut dst = [0.0_f32; 8];
    assert_eq!(ring.pop_into(&mut dst), 6);
    // Write 5 samples starting at physical index 6 — wraps after 2.
    let n = ring.push_slice(&[1.0, 2.0, 3.0, 4.0, 5.0]);
    assert_eq!(n, 5);
    assert_eq!(ring.pop_into(&mut dst), 5);
    assert_eq!(&dst[..5], &[1.0, 2.0, 3.0, 4.0, 5.0]);
}

#[test]
fn push_slice_cross_thread_visibility() {
    use std::sync::Arc;

    // Producer bulk-pushes a monotonically increasing stream; consumer
    // must only ever observe a gapless prefix-ordered sequence (the
    // Release commit publishes all bulk-written samples atomically).
    let ring = Arc::new(SpscRing::new(64));
    let producer_ring = ring.clone();
    let total: usize = 10_000;
    let producer = std::thread::spawn(move || {
        let mut next = 0usize;
        let mut chunk = [0.0_f32; 13];
        while next < total {
            let want = 13.min(total - next);
            for (i, slot) in chunk.iter_mut().enumerate().take(want) {
                *slot = (next + i) as f32;
            }
            let pushed = producer_ring.push_slice(&chunk[..want]);
            next += pushed;
            if pushed == 0 {
                std::thread::yield_now();
            }
        }
    });

    let mut expected = 0usize;
    let mut dst = [0.0_f32; 32];
    while expected < total {
        let n = ring.pop_into(&mut dst);
        for &v in &dst[..n] {
            assert_eq!(v, expected as f32, "gap or reorder in consumed stream");
            expected += 1;
        }
        if n == 0 {
            std::thread::yield_now();
        }
    }
    producer.join().unwrap();
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
