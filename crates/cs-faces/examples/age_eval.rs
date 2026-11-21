//! Can a named person's faces from known years date a new photo of them?
//!
//! Leave-one-out on a labelled test set (fetch_face_testdata.py layout +
//! manifest.json with "date"): for every dated face of a person, guess its year
//! from that person's *other* dated faces and compare with simple baselines.
//!
//! cargo run --release -p cs-faces --example age_eval -- <models dir> <test set>

use cs_faces::{FaceModels, SAME_PERSON, centroid, similarity};
use std::collections::HashMap;
use std::path::Path;

/// First plausible year in a free-text date; None if it names two different years.
fn year_of(text: &str) -> Option<i16> {
    let b = text.as_bytes();
    let mut years = vec![];
    for i in 0..b.len().saturating_sub(3) {
        let Some(w) = text.get(i..i + 4) else { continue }; // not on a char boundary
        let edge = |j: usize| j >= b.len() || !b[j].is_ascii_digit();
        if (w.starts_with("19") || w.starts_with("20"))
            && w.bytes().all(|c| c.is_ascii_digit())
            && (i == 0 || edge(i - 1))
            && edge(i + 4)
        {
            years.push(w.parse::<i16>().ok()?);
        }
    }
    years.dedup();
    match years.as_slice() {
        [y] => Some(*y),
        [y, rest @ ..] if rest.iter().all(|r| r == y) => Some(*y),
        _ => None,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (models, set) = (Path::new(&args[0]), Path::new(&args[1]));
    let m = FaceModels::load(
        &models.join("face_detection_yunet_2023mar.onnx"),
        &models.join("face_recognition_sface_2021dec.onnx"),
    )?;
    let manifest: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(set.join("manifest.json"))?)?;
    let info: HashMap<String, (&str, Option<i16>, bool)> = manifest
        .iter()
        .filter_map(|e| {
            Some((
                e["file"].as_str()?.to_string(),
                (
                    e["title"].as_str().unwrap_or(""),
                    e["date"].as_str().and_then(year_of),
                    e["wrong_person"].as_bool() == Some(true),
                ),
            ))
        })
        .collect();

    let scan = cs_faces::folder::scan_folder(set, &m, &|_, _| {})?;
    // Faces per photo.
    let mut by_photo: HashMap<&str, Vec<&Vec<f32>>> = HashMap::new();
    for f in &scan.faces {
        by_photo.entry(f.rel.as_str()).or_default().push(&f.embedding);
    }
    // Per person: a reference from trusted portraits (one face, surname in the title).
    let mut refs: HashMap<String, Vec<&Vec<f32>>> = HashMap::new();
    for (rel, faces) in &by_photo {
        let person = rel.split('/').next().unwrap_or("");
        let surname = person.rsplit('_').next().unwrap_or(person);
        if let Some((title, _, wrong)) = info.get(*rel)
            && faces.len() == 1
            && !wrong
            && title.to_lowercase().contains(surname)
        {
            refs.entry(person.to_string()).or_default().push(faces[0]);
        }
    }
    // Labelled dated faces: in each photo of P, the face most like P's reference.
    struct S<'a> {
        person: String,
        rel: &'a str,
        year: i16,
        emb: &'a Vec<f32>,
    }
    let mut samples: Vec<S> = vec![];
    for (rel, faces) in &by_photo {
        let person = rel.split('/').next().unwrap_or("").to_string();
        let (Some(r), Some((_, Some(year), false))) = (refs.get(&person), info.get(*rel)) else { continue };
        let c = centroid(&r.iter().map(|v| v.as_slice()).collect::<Vec<_>>());
        let best = faces.iter().map(|e| (similarity(e, &c), *e)).max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((s, emb)) = best
            && s >= SAME_PERSON
        {
            samples.push(S { person, rel, year: *year, emb });
        }
    }
    samples.sort_by(|a, b| a.rel.cmp(b.rel));
    println!("{} dated, labelled faces", samples.len());

    const NEAR_DUP: f32 = 0.85; // same picture (crop / re-upload): not a fair hint
    let (mut n, mut err_knn, mut err_med, mut err_mid, mut in_range) = (0, 0f32, 0f32, 0f32, 0);
    let mut lines = vec![];
    for s in &samples {
        let others: Vec<(f32, i16)> = samples
            .iter()
            .filter(|o| o.person == s.person && o.rel != s.rel)
            .map(|o| (similarity(s.emb, o.emb), o.year))
            .filter(|(sim, _)| *sim < NEAR_DUP)
            .collect();
        if others.len() < 3 {
            continue;
        }
        let mut ys: Vec<i16> = others.iter().map(|o| o.1).collect();
        ys.sort_unstable();
        let (lo, hi) = (ys[0], ys[ys.len() - 1]);
        let median = ys[ys.len() / 2];
        let mid = (lo + hi) / 2;
        // k nearest by face, weighted by similarity.
        let mut by_sim = others.clone();
        by_sim.sort_by(|a, b| b.0.total_cmp(&a.0));
        let top = &by_sim[..3.min(by_sim.len())];
        let w: f32 = top.iter().map(|t| t.0.max(0.01)).sum();
        let knn = top.iter().map(|t| t.0.max(0.01) * t.1 as f32).sum::<f32>() / w;
        n += 1;
        err_knn += (knn - s.year as f32).abs();
        err_med += (median - s.year).abs() as f32;
        err_mid += (mid - s.year).abs() as f32;
        in_range += usize::from((lo..=hi).contains(&s.year));
        lines.push(format!(
            "{:34} {}  knn {:6.0}  median {}  seen {lo}-{hi}  nearest {:?}",
            s.rel,
            s.year,
            knn,
            median,
            top.iter().map(|t| (t.1, (t.0 * 100.0).round() as i32)).collect::<Vec<_>>()
        ));
    }
    for l in &lines {
        println!("{l}");
    }
    let n = n.max(1) as f32;
    println!("\n{} faces judged", lines.len());
    println!("mean error, nearest-faces guess   : {:.1} years", err_knn / n);
    println!("mean error, person's median year  : {:.1} years", err_med / n);
    println!("mean error, middle of seen range  : {:.1} years", err_mid / n);
    println!("true year inside person's seen range: {in_range}/{}", lines.len());
    Ok(())
}
