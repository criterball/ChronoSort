//! Can "birth year + the AI's age estimate" date a photo of a known person?
//!
//! For every dated photo of a person whose birth year is public, take that
//! person's face (most like their portraits), ask the vision model for the age,
//! and compare birth year + age with the photo's real year.
//!
//! cargo run --release -p cs-vision --example age_eval -- <llama-server> <model> <mmproj> <faces models> <test set>

use cs_faces::{FaceModels, SAME_PERSON, centroid, similarity};
use cs_vision::{Engine, EngineFiles};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

/// Public birth years of the people in the test set.
const BORN: &[(&str, i32)] = &[
    ("john_glenn", 1921),
    ("buzz_aldrin", 1930),
    ("neil_armstrong", 1930),
    ("sally_ride", 1951),
    ("scott_kelly", 1964),
    ("mark_kelly", 1964),
    ("peggy_whitson", 1960),
    ("barack_obama", 1961),
    ("joe_biden", 1942),
];

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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (faces_dir, set) = (Path::new(&a[3]), Path::new(&a[4]));
    let fm = FaceModels::load(
        &faces_dir.join("face_detection_yunet_2023mar.onnx"),
        &faces_dir.join("face_recognition_sface_2021dec.onnx"),
    )?;
    let manifest: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(set.join("manifest.json"))?)?;
    let info: HashMap<String, (String, Option<i32>, bool)> = manifest
        .iter()
        .filter_map(|e| {
            Some((
                e["file"].as_str()?.to_string(),
                (
                    e["title"].as_str().unwrap_or("").to_lowercase(),
                    e["date"].as_str().and_then(year_of),
                    e["wrong_person"].as_bool() == Some(true),
                ),
            ))
        })
        .collect();
    let scan = cs_faces::folder::scan_folder(set, &fm, &|_, _| {})?;
    let mut by_photo: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, f) in scan.faces.iter().enumerate() {
        by_photo.entry(f.rel.as_str()).or_default().push(i);
    }
    // Reference per person from trusted single-face portraits.
    let mut refs: HashMap<&str, Vec<&[f32]>> = HashMap::new();
    for (rel, idx) in &by_photo {
        let person = rel.split('/').next().unwrap_or("");
        let surname = person.rsplit('_').next().unwrap_or(person);
        if let Some((title, _, false)) = info.get(*rel)
            && idx.len() == 1
            && title.contains(surname)
        {
            refs.entry(person).or_default().push(&scan.faces[idx[0]].embedding);
        }
    }
    let eng = Engine::start(
        &EngineFiles {
            server: Path::new(&a[0]),
            model: Path::new(&a[1]),
            projector: Path::new(&a[2]),
            work_dir: &std::env::temp_dir().join("cs-vision-eval"),
        },
        true,
        Duration::from_secs(300),
    )?;
    let mut seen = std::collections::HashSet::new();
    let (mut n, mut err, mut within5) = (0, 0, 0);
    let mut rels: Vec<&&str> = by_photo.keys().collect();
    rels.sort();
    for rel in rels {
        let person = rel.split('/').next().unwrap_or("");
        let (Some((title, Some(year), false)), Some(&(_, born)), Some(r)) =
            (info.get(*rel), BORN.iter().find(|(p, _)| *p == person), refs.get(person))
        else {
            continue;
        };
        if !seen.insert((title.clone(), person)) {
            continue;
        }
        let c = centroid(r);
        let Some((s, i)) = by_photo[*rel]
            .iter()
            .map(|&i| (similarity(&scan.faces[i].embedding, &c), i))
            .max_by(|x, y| x.0.total_cmp(&y.0))
        else {
            continue;
        };
        if s < SAME_PERSON {
            continue;
        }
        let f = &scan.faces[i];
        let img = image::open(set.join(&f.rel))?.to_rgb8();
        // The detector ran on a downscaled copy (LOAD_SIDE): scale the box back.
        let k = (img.width().max(img.height()) as f32 / cs_faces::folder::LOAD_SIDE as f32).max(1.0);
        let mut face = f.face.clone();
        face.bbox.iter_mut().for_each(|v| *v *= k);
        let crop = cs_faces::face_crop(&img, &face, 336);
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90).encode_image(&crop)?;
        let age = eng.estimate_age(&jpeg)?;
        let guess = born + age;
        n += 1;
        err += (guess - year).abs();
        within5 += i32::from((guess - year).abs() <= 5);
        println!("{rel:40} true {year} (age {:2})  AI age {age:2} -> {guess}  off {:+}", year - born, guess - year);
    }
    println!("\n{n} faces; mean error {:.1} years; within 5 years: {within5}/{n}", err as f64 / n.max(1) as f64);
    Ok(())
}
