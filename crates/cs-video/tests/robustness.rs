//! Robustness ("fuzz") test for the video poster code: damaged and hostile
//! MP4/MOV files must give an error, never a crash or a hang.
//!
//! In the app this code runs in a separate helper process with a time limit,
//! so a crash there cannot take ChronoSort down, but it must still not happen.
//! Deterministic mutation fuzzer (see cs-image/tests/robustness.rs for why not
//! `cargo fuzz`) over tiny generated clips in `tests/seeds` (FFmpeg testsrc2,
//! 160x90, H.264 / HEVC 10-bit / VP9 / AV1 / rotated MOV).
//!
//! Normal run: 300 cases (debug) / 3000 (release). Long run:
//! `CS_FUZZ_CASES=100000 cargo test -p cs-video --release --test robustness -- --nocapture`

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

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
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/seeds");
    let mut out: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap()))
        .collect();
    out.sort();
    out
}

const INTERESTING: [u8; 8] = [0x00, 0xFF, 0x7F, 0x80, 0x01, 0xFE, 0x10, 0xEF];

fn mutate(r: &mut Rng, seed: &[u8], other: &[u8]) -> Vec<u8> {
    let mut b = seed.to_vec();
    for _ in 0..1 + r.below(4) {
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
                let a = r.below(n);
                let e = (a + 1 + r.below(64)).min(n);
                b.drain(a..e);
            }
            4 => {
                let a = r.below(n);
                let e = (a + 1 + r.below(64)).min(n);
                let chunk = b[a..e].to_vec();
                let at = r.below(n);
                b.splice(at..at, chunk);
            }
            5 => {
                // Huge size / count field anywhere (box sizes, sample tables, codec headers).
                let i = r.below(n);
                let v = if r.below(2) == 0 { 0xFF } else { 0x7F };
                b[i..(i + 4).min(n)].fill(v);
            }
            6 if !other.is_empty() => {
                let a = r.below(n);
                let o = r.below(other.len());
                b.truncate(a);
                b.extend_from_slice(&other[o..]);
            }
            _ => {
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

#[test]
fn seeds_decode() {
    for (name, s) in seeds() {
        let img = cs_video::poster_bytes(&s, 320).unwrap_or_else(|e| panic!("seed {name}: {e}"));
        let portrait = name.contains("cw90");
        assert_eq!((img.width() > img.height()), !portrait, "{name}: {}x{}", img.width(), img.height());
    }
}

#[test]
fn damaged_videos_never_crash_or_hang() {
    let cases: usize = std::env::var("CS_FUZZ_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(if cfg!(debug_assertions) { 300 } else { 3000 });
    let seeds = seeds();
    assert!(seeds.len() >= 4);
    let mut r = Rng(0x0BAD_F00D_D15E_A5E5);
    let mut slowest = (Duration::ZERO, String::new());
    let (mut panics, mut ok) = (vec![], 0usize);
    let started = Instant::now();
    for case in 0..cases {
        let (name, seed) = &seeds[case % seeds.len()];
        let other = &seeds[r.below(seeds.len())].1;
        let input = mutate(&mut r, seed, other);
        let t = Instant::now();
        let res = catch_unwind(AssertUnwindSafe(|| cs_video::poster_bytes(&input, 160).is_ok()));
        let took = t.elapsed();
        if took > slowest.0 {
            slowest = (took, format!("case {case} ({name}, {} bytes)", input.len()));
        }
        match res {
            Ok(true) => ok += 1,
            Ok(false) => {}
            Err(p) => {
                let msg =
                    p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()));
                let dir = std::env::temp_dir().join("cs_video_fuzz_findings");
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(dir.join(format!("case_{case}.mp4")), &input);
                panics.push(format!("case {case} ({name}): {}", msg.unwrap_or_default()));
            }
        }
        assert!(took < Duration::from_secs(10), "case {case} ({name}) took {took:?}: possible hang");
    }
    eprintln!(
        "{cases} cases in {:?}; {ok} still gave a picture; slowest {:?} at {}",
        started.elapsed(),
        slowest.0,
        slowest.1
    );
    assert!(
        panics.is_empty(),
        "{} panics (inputs saved in the temp folder, cs_video_fuzz_findings):\n{}",
        panics.len(),
        panics.join("\n")
    );
}
