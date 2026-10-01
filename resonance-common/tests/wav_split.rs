//! `decode_wav_split` (drums-plugin-rework.md E14): a take split into an
//! in-memory head and an on-disk tail reads back bit-identical to the
//! full `decode_wav_native` decode — every encoding the drum kits use,
//! mono / stereo / wider, with and without resampling, whatever chunks
//! the tail is read in.

use std::io::Write;

use resonance_common::{decode_wav_native, decode_wav_split, PcmEncoding, TailScratch, WavPcmLayout};

/// A deterministic non-trivial signal: a few incommensurate sines plus a
/// hash-noise component, in -0.9..0.9.
fn signal(frame: usize, channel: usize) -> f64 {
    let t = frame as f64;
    let noise = ((frame.wrapping_mul(2_654_435_761) ^ (channel * 977)) % 1000) as f64 / 1000.0 - 0.5;
    0.4 * (t * 0.031 + channel as f64).sin() + 0.3 * (t * 0.0071).sin() + 0.2 * noise
}

/// A WAV of `frames` frames in `encoding` (tag 1 or 3; `extensible`
/// wraps it in WAVE_FORMAT_EXTENSIBLE), with an extra chunk before
/// `data` so the parser has to walk.
fn wav(encoding: PcmEncoding, channels: u16, rate: u32, frames: usize, extensible: bool) -> Vec<u8> {
    let width = encoding.bytes();
    let block_align = channels as usize * width;
    let mut data = Vec::with_capacity(frames * block_align);
    for f in 0..frames {
        for ch in 0..channels as usize {
            let v = signal(f, ch);
            match encoding {
                PcmEncoding::U8 => data.push(((v * 127.0) as i32 + 128) as u8),
                PcmEncoding::S16 => data.extend_from_slice(&((v * 32_767.0) as i16).to_le_bytes()),
                PcmEncoding::S24 => {
                    let s = (v * 8_388_607.0) as i32;
                    data.extend_from_slice(&s.to_le_bytes()[..3]);
                }
                PcmEncoding::S32 => data.extend_from_slice(&((v * 2_147_483_000.0) as i32).to_le_bytes()),
                PcmEncoding::F32 => data.extend_from_slice(&(v as f32).to_le_bytes()),
                PcmEncoding::F64 => data.extend_from_slice(&v.to_le_bytes()),
            }
        }
    }
    let tag: u16 = match encoding {
        PcmEncoding::F32 | PcmEncoding::F64 => 3,
        _ => 1,
    };
    let bits = (width * 8) as u16;
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&(if extensible { 0xFFFEu16 } else { tag }).to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * block_align as u32).to_le_bytes());
    fmt.extend_from_slice(&(block_align as u16).to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    if extensible {
        fmt.extend_from_slice(&22u16.to_le_bytes());
        fmt.extend_from_slice(&bits.to_le_bytes());
        let mask: u32 = if channels == 1 { 0x4 } else { (1 << channels) - 1 };
        fmt.extend_from_slice(&mask.to_le_bytes());
        // KSDATAFORMAT_SUBTYPE_PCM / _IEEE_FLOAT
        fmt.extend_from_slice(&tag.to_le_bytes());
        fmt.extend_from_slice(&[
            0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
        ]);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0u32.to_le_bytes()); // patched below
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    out.extend_from_slice(&fmt);
    // An odd-sized chunk the parser must skip, pad byte included.
    out.extend_from_slice(b"junk");
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(b"abc\0");
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    let riff = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    out
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|s| s.to_bits()).collect()
}

const HEAD: usize = 4_096;
const MIN_TAIL: usize = 512;

/// Split-decode `bytes` (also written to a file) at `target`, read the
/// tail back in uneven chunks, and compare head + tail with the full
/// decode, bit for bit.
fn check_round_trip(bytes: Vec<u8>, target: f32, label: &str) {
    let full = decode_wav_native(bytes.clone(), target).expect("full decode");
    let dir = std::env::temp_dir().join(format!("wav-split-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.wav", label.replace(' ', "_")));
    std::fs::File::create(&path).unwrap().write_all(&bytes).unwrap();

    let split = decode_wav_split(bytes, target, HEAD, MIN_TAIL).expect("split decode");
    assert_eq!(split.channels, full.channels, "{label}: channels");
    assert_eq!(split.frames, full.frames(), "{label}: frames");
    let tail = split.tail.as_ref().unwrap_or_else(|| panic!("{label}: not split"));
    assert_eq!(split.samples.len(), HEAD * split.channels, "{label}: head length");
    assert_eq!(tail.frames() as usize, full.frames());

    let file = std::fs::File::open(&path).unwrap();
    let mut scratch = TailScratch::default();
    let mut rebuilt = split.samples.clone();
    let mut at = HEAD;
    // Uneven chunks: odd sizes, a one-frame read, a big one.
    let sizes = [1usize, 777, 4_096, 3, 10_000, 1_031];
    let mut i = 0;
    while at < split.frames {
        let n = sizes[i % sizes.len()].min(split.frames - at);
        let mut chunk = vec![0.0f32; n * split.channels];
        tail.read(&file, at as u64, &mut chunk, &mut scratch).expect("tail read");
        rebuilt.extend_from_slice(&chunk);
        at += n;
        i += 1;
    }
    assert!(
        bits(&rebuilt) == bits(&full.samples),
        "{label}: head + tail differs from the full decode"
    );
    // A read past the end is refused, not padded.
    let mut one = vec![0.0f32; split.channels];
    assert!(tail.read(&file, split.frames as u64, &mut one, &mut scratch).is_err());
    let _ = std::fs::remove_file(&path);
}

#[test]
fn every_encoding_and_layout_round_trips_bit_identically() {
    let encodings = [
        PcmEncoding::U8,
        PcmEncoding::S16,
        PcmEncoding::S24,
        PcmEncoding::S32,
        PcmEncoding::F32,
        PcmEncoding::F64,
    ];
    for encoding in encodings {
        for channels in [1u16, 2, 3] {
            for (rate, target) in [(48_000u32, 48_000.0f32), (44_100, 48_000.0), (96_000, 48_000.0)] {
                let frames = 20_011;
                let label = format!("{encoding:?} {channels}ch {rate}->{target}");
                check_round_trip(wav(encoding, channels, rate, frames, false), target, &label);
            }
        }
    }
}

#[test]
fn extensible_headers_round_trip() {
    for encoding in [PcmEncoding::S24, PcmEncoding::F32] {
        for channels in [1u16, 2] {
            let label = format!("ext {encoding:?} {channels}ch");
            check_round_trip(wav(encoding, channels, 44_100, 15_000, true), 48_000.0, &label);
        }
    }
}

#[test]
fn short_takes_and_unsplittable_files_come_back_whole() {
    // Shorter than head + margin: whole.
    let short = wav(PcmEncoding::S16, 2, 48_000, HEAD + MIN_TAIL, false);
    let full = decode_wav_native(short.clone(), 48_000.0).unwrap();
    let split = decode_wav_split(short, 48_000.0, HEAD, MIN_TAIL).unwrap();
    assert!(split.tail.is_none());
    assert_eq!(bits(&split.samples), bits(&full.samples));

    // Head 0 never splits.
    let long = wav(PcmEncoding::S16, 1, 44_100, 30_000, false);
    let full = decode_wav_native(long.clone(), 48_000.0).unwrap();
    let split = decode_wav_split(long, 48_000.0, 0, MIN_TAIL).unwrap();
    assert!(split.tail.is_none());
    assert_eq!(bits(&split.samples), bits(&full.samples));
}

#[test]
fn layout_parse_finds_the_data_chunk() {
    let bytes = wav(PcmEncoding::S24, 2, 44_100, 1_000, false);
    let layout = WavPcmLayout::parse(&bytes).expect("layout");
    assert_eq!(layout.channels, 2);
    assert_eq!(layout.sample_rate, 44_100);
    assert_eq!(layout.encoding, PcmEncoding::S24);
    assert_eq!(layout.block_align, 6);
    assert_eq!(layout.frames, 1_000);
    assert_eq!(&bytes[layout.data_offset as usize - 8..layout.data_offset as usize - 4], b"data");
    // A truncated data chunk counts only the whole frames present.
    let cut = &bytes[..bytes.len() - 7];
    assert_eq!(WavPcmLayout::parse(cut).unwrap().frames, 998);
    assert!(WavPcmLayout::parse(b"not a wav at all").is_none());
}
