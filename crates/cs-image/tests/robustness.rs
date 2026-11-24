//! Robustness ("fuzz") test for the image code: damaged and hostile files must
//! give an error, never a crash or a hang.
//!
//! `cargo fuzz` needs nightly Rust + libFuzzer, which the project's
//! windows-gnu toolchain does not have, so this is a deterministic mutation
//! fuzzer that runs with the normal tests:
//! seeds = every format ChronoSort can save + real HEIC/JPEG samples when present,
//! mutations = truncation, bit flips, "interesting" bytes, removed / repeated
//! chunks, splices between formats, and huge size fields.
//!
//! Normal run: 500 cases (debug) / 3000 (release), seconds. Long run:
//! `CS_FUZZ_CASES=200000 cargo test -p cs-image --release --test robustness -- --nocapture`

use cs_image::{
    ChannelLayout, Edit, Image, SampleType, SaveOptions, apply, open_bytes, preview_rgb8, save, save_formats,
};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

/// Small, fast, reproducible PRNG (xorshift64*).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

fn seeds() -> Vec<(String, Vec<u8>)> {
    let d = tempfile::tempdir().unwrap();
    let data: Vec<u8> = (0..48 * 32 * 3).map(|i| (i * 7 % 251) as u8).collect();
    let img = Image::from_raw(48, 32, ChannelLayout::Rgb, SampleType::U8, data).unwrap();
    let mut out = vec![];
    for (fmt, name, ext) in save_formats() {
        let p = d.path().join(format!("seed.{ext}"));
        save(&img, fmt, SaveOptions { quality: 80 }, &p).unwrap();
        out.push((name.to_string(), std::fs::read(&p).unwrap()));
    }
    // Real files (local test data; skipped when absent): HEIC and camera/NASA JPEGs with EXIF.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../temp");
    for f in ["heic_samples/example.heic", "cs_faces/john_glenn/john_glenn_01.jpg"] {
        if let Ok(b) = std::fs::read(root.join(f))
            && b.len() < 4_000_000
        {
            out.push((f.to_string(), b));
        }
    }
    out
}

const INTERESTING: [u8; 8] = [0x00, 0xFF, 0x7F, 0x80, 0x01, 0xFE, 0x10, 0xEF];

fn mutate(r: &mut Rng, seed: &[u8], other: &[u8]) -> Vec<u8> {
    let mut b = seed.to_vec();
    let rounds = 1 + r.below(4);
    for _ in 0..rounds {
        if b.is_empty() {
            break;
        }
        let n = b.len();
        match r.below(8) {
            0 => b.truncate(r.below(n)),
            1 => {
                let i = r.below(n);
                b[i] ^= 1 << r.below(8);
            }
            2 => {
                let i = r.below(n);
                b[i] = INTERESTING[r.below(INTERESTING.len())];
            }
            3 => {
                // Remove a chunk.
                let a = r.below(n);
                let e = (a + 1 + r.below(64)).min(n);
                b.drain(a..e);
            }
            4 => {
                // Repeat a chunk.
                let a = r.below(n);
                let e = (a + 1 + r.below(64)).min(n);
                let chunk = b[a..e].to_vec();
                let at = r.below(n);
                b.splice(at..at, chunk);
            }
            5 => {
                // Huge size field: 4 bytes of 0xFF / 0x7F somewhere in the header.
                let i = r.below(n.min(256));
                let v = if r.below(2) == 0 { 0xFF } else { 0x7F };
                b[i..(i + 4).min(n)].fill(v);
            }
            6 if !other.is_empty() => {
                // Splice: this header, another file's body.
                let a = r.below(n);
                let o = r.below(other.len());
                b.truncate(a);
                b.extend_from_slice(&other[o..]);
            }
            _ => {
                // Random bytes in place.
                let i = r.below(n);
                let e = (i + 1 + r.below(16)).min(n);
                for x in &mut b[i..e] {
                    *x = r.next() as u8;
                }
            }
        }
    }
    b
}

/// Everything the viewer does with a file it opened.
fn exercise(bytes: &[u8]) {
    if let Ok(ph) = open_bytes(bytes) {
        let (w, h) = ph.image.dimensions();
        assert!(w > 0 && h > 0, "opened with an empty size");
        let _ = preview_rgb8(&ph.image, 256);
        let _ = apply(ph.image.clone(), Edit::RotateRight);
        let _ = apply(ph.image, Edit::Crop { x: 0, y: 0, w: w.min(8), h: h.min(8) });
    }
}

#[test]
fn damaged_files_never_crash_or_hang() {
    let cases: usize = std::env::var("CS_FUZZ_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(if cfg!(debug_assertions) { 500 } else { 3000 });
    let seeds = seeds();
    assert!(seeds.len() >= 4);
    // The undamaged seeds must open.
    for (name, s) in &seeds {
        assert!(open_bytes(s).is_ok(), "seed {name} does not open");
    }
    let mut r = Rng(0x00C0_FFEE_D15E_A5E5);
    let mut slowest = (Duration::ZERO, String::new());
    let mut panics = vec![];
    let started = Instant::now();
    for case in 0..cases {
        let (name, seed) = &seeds[case % seeds.len()];
        let other = &seeds[r.below(seeds.len())].1;
        let input = mutate(&mut r, seed, other);
        let t = Instant::now();
        // A panic in a decoder is a finding (the app turns it into an error, but it must not happen).
        let res = catch_unwind(AssertUnwindSafe(|| exercise(&input)));
        let took = t.elapsed();
        if took > slowest.0 {
            slowest = (took, format!("case {case} ({name}, {} bytes)", input.len()));
        }
        if let Err(p) = res {
            let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
            let dir = std::env::temp_dir().join("cs_fuzz_findings");
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join(format!("case_{case}.bin")), &input);
            panics.push(format!("case {case} ({name}): {}", msg.unwrap_or_default()));
        }
        assert!(took < Duration::from_secs(10), "case {case} ({name}) took {took:?}: possible hang");
    }
    eprintln!("{cases} cases in {:?}; slowest {:?} at {}", started.elapsed(), slowest.0, slowest.1);
    assert!(
        panics.is_empty(),
        "{} panics (inputs saved in the temp folder, cs_fuzz_findings):\n{}",
        panics.len(),
        panics.join("\n")
    );
}
