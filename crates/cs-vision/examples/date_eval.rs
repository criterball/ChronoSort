//! How well does the local vision model date photos? Runs every dated photo of
//! a test set (manifest.json with "date", as made by fetch_face_testdata.py).
//!
//! cargo run --release -p cs-vision --example date_eval -- <llama-server> <model.gguf> <mmproj.gguf> <test set> [limit]
//!
//! Caveat: famous photos may be recognised (e.g. Apollo 11), which flatters the result.

use cs_vision::{Engine, EngineFiles};
use std::path::Path;
use std::time::{Duration, Instant};

fn year_of(text: &str) -> Option<i32> {
    let b = text.as_bytes();
    let mut years = vec![];
    for i in 0..b.len().saturating_sub(3) {
        let Some(w) = text.get(i..i + 4) else { continue };
        let edge = |j: usize| j >= b.len() || !b[j].is_ascii_digit();
        if (w.starts_with("19") || w.starts_with("20"))
            && w.bytes().all(|c| c.is_ascii_digit())
            && (i == 0 || edge(i - 1))
            && edge(i + 4)
        {
            years.push(w.parse::<i32>().ok()?);
        }
    }
    years.dedup();
    if years.len() == 1 { Some(years[0]) } else { None }
}

fn jpeg_for_model(path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let img = image::open(path)?.thumbnail(896, 896).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88).encode_image(&img)?;
    Ok(out)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let set = Path::new(&a[3]);
    let limit: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    let work = std::env::temp_dir().join("cs-vision-eval");
    let t = Instant::now();
    let eng = Engine::start(
        &EngineFiles {
            server: Path::new(&a[0]),
            model: Path::new(&a[1]),
            projector: Path::new(&a[2]),
            work_dir: &work,
        },
        true,
        Duration::from_secs(300),
    )?;
    println!("engine ready in {:.1?} (log {})", t.elapsed(), eng.log.display());
    let manifest: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(set.join("manifest.json"))?)?;
    let mut seen = std::collections::HashSet::new();
    let (mut n, mut abs_err, mut inside, mut within5, mut secs) = (0, 0i32, 0, 0, 0f64);
    for e in &manifest {
        let (Some(file), Some(year)) = (e["file"].as_str(), e["date"].as_str().and_then(year_of)) else { continue };
        // The same picture can be listed under several people.
        if !seen.insert(e["title"].as_str().unwrap_or(file).to_string()) || n >= limit {
            continue;
        }
        let jpeg = jpeg_for_model(&set.join(file))?;
        let t = Instant::now();
        let g = match eng.date_photo(&jpeg) {
            Ok(g) => g,
            Err(x) => {
                println!("{file}: ERROR {x}");
                continue;
            }
        };
        secs += t.elapsed().as_secs_f64();
        n += 1;
        abs_err += (g.best_year - year).abs();
        inside += usize::from((g.earliest_year..=g.latest_year).contains(&year));
        within5 += usize::from((g.best_year - year).abs() <= 5);
        println!(
            "{file:38} true {year}  guess {} ({}-{})  stamp {:?}  {:?}",
            g.best_year, g.earliest_year, g.latest_year, g.date_stamp, g.clues
        );
    }
    let nf = n.max(1) as f64;
    println!("\n{n} photos, {:.1} s per photo", secs / nf);
    println!("mean error of best year : {:.1} years", abs_err as f64 / nf);
    println!("best year within 5 years: {within5}/{n}");
    println!("true year inside range  : {inside}/{n}");
    Ok(())
}
