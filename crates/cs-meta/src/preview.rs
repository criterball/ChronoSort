//! The largest JPEG preview inside a RAW file (NEF, CR2, ARW, DNG, CR3, …).
//!
//! Cameras store a full- or half-size JPEG next to the raw sensor data. Rather
//! than understanding every maker's layout, look for complete JPEG streams
//! (start marker, a frame header, a proper end) in the first part of the file
//! and take the biggest. Bounded: at most `limit` bytes are read and at most a
//! fixed number of candidates and segments are walked.

use std::io::Read;
use std::path::Path;

const MAX_CANDIDATES: usize = 64;
const MAX_SEGMENTS: usize = 512;

/// End offset (exclusive) of a well-formed JPEG starting at `start`, if any.
fn jpeg_end(b: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 2;
    let mut frame = false;
    for _ in 0..MAX_SEGMENTS {
        // Markers may be padded with extra 0xFF bytes.
        while *b.get(i)? == 0xFF && *b.get(i + 1)? == 0xFF {
            i += 1;
        }
        if *b.get(i)? != 0xFF {
            return None;
        }
        let m = *b.get(i + 1)?;
        match m {
            0xD9 => return frame.then_some(i + 2),
            0xD0..=0xD7 | 0x01 => i += 2,
            _ => {
                let len = u16::from_be_bytes([*b.get(i + 2)?, *b.get(i + 3)?]) as usize;
                if len < 2 {
                    return None;
                }
                if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
                    frame = true;
                }
                i += 2 + len;
                if m == 0xDA {
                    // Entropy-coded data: runs until a marker that is not a
                    // stuffed byte (FF00) or a restart marker (FFD0-FFD7).
                    loop {
                        let p = i + b.get(i..)?.iter().position(|&x| x == 0xFF)?;
                        let n = *b.get(p + 1)?;
                        if n == 0x00 || (0xD0..=0xD7).contains(&n) {
                            i = p + 2;
                        } else {
                            i = p;
                            break;
                        }
                    }
                }
            }
        }
    }
    None
}

/// The largest complete embedded JPEG in `b`.
pub fn largest_jpeg(b: &[u8]) -> Option<&[u8]> {
    let mut best: Option<&[u8]> = None;
    let mut at = 0;
    let mut tried = 0;
    while let Some(p) = b.get(at..)?.windows(3).position(|w| w == [0xFF, 0xD8, 0xFF]) {
        let s = at + p;
        tried += 1;
        if tried > MAX_CANDIDATES {
            break;
        }
        match jpeg_end(b, s) {
            Some(e) => {
                if best.is_none_or(|x| e - s > x.len()) {
                    best = Some(&b[s..e]);
                }
                at = e;
            }
            None => at = s + 3,
        }
    }
    best
}

/// Read up to `limit` bytes of a RAW file and return its largest JPEG preview.
pub fn raw_preview(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    std::fs::File::open(path).ok()?.take(limit).read_to_end(&mut buf).ok()?;
    largest_jpeg(&buf).map(<[u8]>::to_vec)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal well-formed JPEG: SOI, SOF0, SOS + `n` bytes of data, EOI.
    fn jpeg(n: usize) -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8];
        j.extend([0xFF, 0xC0, 0x00, 0x0B, 8, 0, 1, 0, 1, 1, 1, 0x11, 0]);
        j.extend([0xFF, 0xDA, 0x00, 0x08, 1, 1, 0, 0, 0x3F, 0]);
        j.extend(
            (0..n)
                .map(|i| if i % 50 == 0 { 0xFF } else { 0x12 })
                .flat_map(|x| if x == 0xFF { vec![0xFF, 0x00] } else { vec![x] }),
        );
        j.extend([0xFF, 0xD9]);
        j
    }

    #[test]
    fn picks_the_biggest_complete_preview() {
        let (small, big) = (jpeg(100), jpeg(5000));
        let mut raw = b"II*\0 maker header ".to_vec();
        raw.extend(&small);
        raw.extend([0u8; 300]);
        raw.extend([0xFF, 0xD8, 0xFF, 0xE1, 0x00]); // a broken start: ignored
        raw.extend(&big);
        raw.extend([7u8; 300]);
        assert_eq!(largest_jpeg(&raw), Some(&big[..]));
    }

    #[test]
    fn truncated_or_garbage_is_none_never_a_panic() {
        let big = jpeg(2000);
        assert!(largest_jpeg(&big[..big.len() - 1]).is_none());
        let mut g = big.clone();
        for i in (2..g.len()).step_by(5) {
            g[i] = 0xFF;
            let _ = largest_jpeg(&g);
        }
        assert!(largest_jpeg(b"").is_none());
    }
}
