//! Poor man's fuzzing that runs on every `cargo test`: corrupt every corpus file
//! in many ways and push the results through the full pipeline. Nothing may
//! panic, hang or error out; damaged input must only lead to review/flags.
//! (Proper coverage-guided fuzzing with cargo-fuzz runs on Linux CI later.)

use cs_core::{DatePolicy, analyze};
use cs_model::civil::unix;
use std::time::{Duration, Instant};

struct XorShift(u64);
impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

fn mutate(rng: &mut XorShift, src: &[u8]) -> Vec<u8> {
    let mut b = src.to_vec();
    match rng.below(5) {
        0 => {
            for _ in 0..1 + rng.below(8) {
                let i = rng.below(b.len());
                if let Some(x) = b.get_mut(i) {
                    *x ^= 1 << rng.below(8);
                }
            }
        }
        1 => b.truncate(rng.below(b.len())),
        2 => {
            // overwrite a 4-byte field (lengths/offsets) with an extreme value
            let i = rng.below(b.len().saturating_sub(4));
            let v: [u8; 4] = [[0xFF; 4], [0; 4], [0x7F, 0xFF, 0xFF, 0xFF], [0, 0, 0, 1]][rng.below(4)];
            if i + 4 <= b.len() {
                b[i..i + 4].copy_from_slice(&v);
            }
        }
        3 => {
            let i = rng.below(b.len());
            let junk: Vec<u8> = (0..rng.below(512)).map(|_| rng.next() as u8).collect();
            b.splice(i..i, junk);
        }
        _ => b = (0..rng.below(4096)).map(|_| rng.next() as u8).collect(),
    }
    b
}

#[test]
fn corrupted_files_never_crash_the_pipeline() {
    let d = tempfile::tempdir().unwrap();
    let manifest = cs_testgen::generate(d.path()).unwrap();
    let src = d.path().join(cs_testgen::SOURCE_DIR);
    let out = d.path().join("mutants");
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    let mut n = 0;
    for e in &manifest.entries {
        let bytes = std::fs::read(src.join(&e.path)).unwrap();
        let name = e.path.rsplit('/').next().unwrap();
        for k in 0..30 {
            let dir = out.join(format!("m{n:05}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{k}_{name}")), mutate(&mut rng, &bytes)).unwrap();
            n += 1;
        }
    }
    let t = Instant::now();
    let a = analyze(&out, &DatePolicy::new(unix(2026, 11, 20, 0, 0, 0))).unwrap();
    assert_eq!(a.items.len(), n, "every mutant must be reported, none silently dropped");
    assert!(t.elapsed() < Duration::from_secs(60), "took {:?}", t.elapsed());
    eprintln!("{n} mutants analysed in {:?}", t.elapsed());
}
