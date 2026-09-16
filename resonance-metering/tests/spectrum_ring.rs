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
fn mixed_push_paths_cross_thread_stress() {
    use std::sync::Arc;

    // Two real threads hammering a small ring so producer and consumer are
    // almost always inside the buffer at the same time — the case the
    // raw-pointer discipline in push/push_slice/pop_into exists for (no
    // whole-buffer reference may ever be materialised on either side).
    // The producer alternates the per-sample and bulk paths; the consumer
    // must still observe the exact monotone stream, gapless and in order.
    // Bounded: 20_000 samples through a 32-slot ring finishes in well
    // under a second even with the yields.
    let ring = Arc::new(SpscRing::new(32));
    let producer_ring = ring.clone();
    let total: usize = 20_000;
    let producer = std::thread::spawn(move || {
        let mut next = 0usize;
        let mut chunk = [0.0_f32; 11];
        while next < total {
            let pushed = if next % 2 == 0 {
                // Per-sample path.
                usize::from(producer_ring.push(next as f32))
            } else {
                // Bulk path, deliberately often straddling the wrap.
                let want = 11.min(total - next);
                for (i, slot) in chunk.iter_mut().enumerate().take(want) {
                    *slot = (next + i) as f32;
                }
                producer_ring.push_slice(&chunk[..want])
            };
            next += pushed;
            if pushed == 0 {
                std::thread::yield_now();
            }
        }
    });

    let mut expected = 0usize;
    let mut dst = [0.0_f32; 13];
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
fn clear_request_is_deferred_until_consumer_services_it() {
    let ring = SpscRing::new(16);
    ring.push_slice(&[1.0, 2.0, 3.0]);
    // No request pending yet.
    assert!(!ring.take_clear_request());
    assert_eq!(ring.available(), 3);
    // A request does nothing until the consumer services it.
    ring.request_clear();
    assert_eq!(ring.available(), 3);
    assert!(ring.take_clear_request());
    assert_eq!(ring.available(), 0);
    let mut dst = [0.0_f32; 4];
    assert_eq!(ring.pop_into(&mut dst), 0);
    // The request is one-shot.
    assert!(!ring.take_clear_request());
    // Samples pushed after the service point flow through normally.
    ring.push(9.0);
    assert_eq!(ring.pop_into(&mut dst), 1);
    assert_eq!(dst[0], 9.0);
}

#[test]
fn producer_side_clear_requests_never_corrupt_stream() {
    use std::sync::Arc;

    // The bug this guards: SpectrumAnalyzer::reset() used to call
    // ring.clear() from the producer (audio) thread, writing `head` while
    // the consumer's pop_into was also writing `head` — the producer could
    // then see a whole ring of free space and overwrite cells the consumer
    // was mid-read, and the consumer's later `head` store resurrected
    // "cleared" samples. With request_clear/take_clear_request the clear
    // only ever happens on the consumer thread, between complete pops.
    //
    // Invariant: the producer pushes a strictly increasing integer stream
    // and requests clears at arbitrary points. Whatever the interleaving,
    // the consumed stream must be a strictly increasing subsequence of the
    // pushed values — a clear may drop a contiguous run, but must never
    // duplicate, reorder, resurrect, or garble a sample.
    let ring = Arc::new(SpscRing::new(32));
    let producer_ring = ring.clone();
    let total: usize = 20_000;
    let producer = std::thread::spawn(move || {
        let mut next = 0usize;
        let mut chunk = [0.0_f32; 7];
        while next < total {
            if next.is_multiple_of(611) {
                // What the audio thread's reset() now does.
                producer_ring.request_clear();
            }
            let want = 7.min(total - next);
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

    let mut last_seen: i64 = -1;
    let mut dst = [0.0_f32; 13];
    loop {
        // Consumer service point, exactly as the FFT worker runs it.
        ring.take_clear_request();
        let n = ring.pop_into(&mut dst);
        for &v in &dst[..n] {
            assert!(
                v >= 0.0 && v.fract() == 0.0 && (v as usize) < total,
                "garbled sample {v} popped from the ring"
            );
            let v = v as i64;
            assert!(
                v > last_seen,
                "duplicate/reordered/resurrected sample {v} after {last_seen}"
            );
            last_seen = v;
        }
        if producer.is_finished() && ring.available() == 0 && !ring.take_clear_request() {
            break;
        }
        if n == 0 {
            std::thread::yield_now();
        }
    }
    producer.join().unwrap();
    assert_eq!(ring.pop_into(&mut dst), 0);
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
