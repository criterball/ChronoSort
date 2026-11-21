//! Evaluate detection + recognition on a folder of `<person>/<photo>.jpg`.
//! cargo run --release -p cs-faces --example eval -- MODELS_DIR TESTSET_DIR [OUT_DIR]
//!
//! Photos with exactly one face are "labelled" samples (the folder name is the
//! person). Prints same-person / different-person similarity statistics and
//! the error rates at the SAME_PERSON threshold. With OUT_DIR, writes the
//! aligned 112×112 crops so alignment can be checked by eye.
use cs_faces::{CLUSTER, FaceModels, MIN_SCORE, SAME_PERSON, cluster, similarity};
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let models = PathBuf::from(&a[0]);
    let set = PathBuf::from(&a[1]);
    let out = a.get(2).map(PathBuf::from);
    let t = Instant::now();
    let m = FaceModels::load(
        &models.join("face_detection_yunet_2023mar.onnx"),
        &models.join("face_recognition_sface_2021dec.onnx"),
    )?;
    println!("models loaded in {:.2?}", t.elapsed());

    // Labels come from folder names, but searches return other people too (a
    // Biden portrait under "Obama"). With a manifest.json from
    // fetch_face_testdata.py, only trust a label if the file title has the surname.
    // Entries checked by eye as someone else carry "wrong_person": true (e.g. a
    // namesake: "Col Mark D. Kelly" is not the astronaut Mark E. Kelly).
    let titles: std::collections::HashMap<String, String> = std::fs::read(set.join("manifest.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<serde_json::Value>>(&b).ok())
        .unwrap_or_default()
        .iter()
        .filter_map(|e| {
            let title = if e["wrong_person"].as_bool() == Some(true) {
                String::new()
            } else {
                e["title"].as_str()?.to_lowercase()
            };
            Some((e["file"].as_str()?.to_string(), title))
        })
        .collect();
    let trusted = |person: &str, name: &str| {
        let surname = person.rsplit('_').next().unwrap_or(person);
        titles.get(&format!("{person}/{name}")).is_none_or(|t| t.contains(surname))
    };
    let mut untrusted = 0;
    let mut samples: Vec<(String, String, Vec<f32>)> = vec![]; // person, file, embedding
    let mut all: Vec<(String, Vec<f32>)> = vec![]; // every face: label ("?" in group photos)
    let (mut n_img, mut n_faces, mut t_det, mut t_emb) = (0, 0, 0.0f64, 0.0f64);
    let mut people: Vec<_> = std::fs::read_dir(&set)?.flatten().filter(|e| e.path().is_dir()).collect();
    people.sort_by_key(|e| e.file_name());
    for p in people {
        let person = p.file_name().to_string_lossy().into_owned();
        let mut files: Vec<_> = std::fs::read_dir(p.path())?.flatten().map(|e| e.path()).collect();
        files.sort();
        for f in files {
            let Ok(img) = image::open(&f) else { continue };
            let img = img.to_rgb8();
            n_img += 1;
            let t = Instant::now();
            let faces = m.detect(&img, MIN_SCORE)?;
            t_det += t.elapsed().as_secs_f64();
            n_faces += faces.len();
            let name = f.file_name().unwrap().to_string_lossy().into_owned();
            let best = faces.first().map(|x| x.score).unwrap_or(0.0);
            println!(
                "{person}/{name}: {} face(s){}",
                faces.len(),
                if faces.is_empty() { String::new() } else { format!(", best {best:.2}") }
            );
            // Faces in group photos have no reliable label.
            for fc in faces.iter().skip(usize::from(faces.len() == 1)) {
                all.push((format!("?{person}"), m.embed(&img, fc)?));
            }
            if faces.len() == 1 && !trusted(&person, &name) {
                untrusted += 1;
                all.push((format!("?{person}"), m.embed(&img, &faces[0])?));
            } else if faces.len() == 1 {
                let t = Instant::now();
                let e = m.embed(&img, &faces[0])?;
                t_emb += t.elapsed().as_secs_f64();
                if let Some(o) = &out {
                    std::fs::create_dir_all(o)?;
                    cs_faces::align(&img, &faces[0].landmarks).save(o.join(format!("{person}__{name}.png")))?;
                }
                all.push((person.clone(), e.clone()));
                samples.push((person.clone(), name, e));
            }
        }
    }
    println!(
        "\n{n_img} images, {n_faces} faces; detect {:.0} ms/image, fingerprint {:.0} ms/face",
        1000.0 * t_det / n_img.max(1) as f64,
        1000.0 * t_emb / samples.len().max(1) as f64
    );

    let (mut same, mut diff) = (vec![], vec![]);
    let mut worst_diff: Option<(f32, String, String)> = None;
    let mut worst_same: Option<(f32, String, String)> = None;
    for i in 0..samples.len() {
        for j in i + 1..samples.len() {
            let s = similarity(&samples[i].2, &samples[j].2);
            let pair = (s, format!("{}/{}", samples[i].0, samples[i].1), format!("{}/{}", samples[j].0, samples[j].1));
            if samples[i].0 == samples[j].0 {
                same.push(s);
                if worst_same.as_ref().is_none_or(|w| s < w.0) {
                    worst_same = Some(pair);
                }
            } else {
                diff.push(s);
                if worst_diff.as_ref().is_none_or(|w| s > w.0) {
                    worst_diff = Some(pair);
                }
            }
        }
    }
    let stats = |v: &[f32]| {
        let mean = v.iter().sum::<f32>() / v.len().max(1) as f32;
        let min = v.iter().cloned().fold(f32::INFINITY, f32::min);
        let max = v.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        (mean, min, max)
    };
    let (sm, smin, smax) = stats(&same);
    let (dm, dmin, dmax) = stats(&diff);
    let missed = same.iter().filter(|&&s| s < SAME_PERSON).count();
    let false_match = diff.iter().filter(|&&s| s >= SAME_PERSON).count();
    println!(
        "{} labelled faces ({untrusted} single-face photos not trusted: title lacks the surname or marked wrong_person)",
        samples.len()
    );
    println!("same person : {} pairs, mean {sm:.3}, min {smin:.3}, max {smax:.3}", same.len());
    println!("different   : {} pairs, mean {dm:.3}, min {dmin:.3}, max {dmax:.3}", diff.len());
    println!(
        "at threshold {SAME_PERSON}: same-person pairs missed {missed}/{} ({:.1}%), different-person pairs matched {false_match}/{} ({:.2}%)",
        same.len(),
        100.0 * missed as f32 / same.len().max(1) as f32,
        diff.len(),
        100.0 * false_match as f32 / diff.len().max(1) as f32
    );
    if let Some((s, a, b)) = worst_same {
        println!("least similar same-person pair: {s:.3} {a} vs {b}");
    }
    if let Some((s, a, b)) = worst_diff {
        println!("most similar different pair  : {s:.3} {a} vs {b}");
    }

    // Clustering over every face (labelled + faces from group photos).
    let embs: Vec<Vec<f32>> = all.iter().map(|x| x.1.clone()).collect();
    let groups = cluster(&embs, CLUSTER);
    let (mut pure, mut labelled_groups) = (0, 0);
    println!(
        "
clustering {} faces at {CLUSTER}: {} groups",
        embs.len(),
        groups.len()
    );
    for g in &groups {
        let mut counts = std::collections::BTreeMap::<&str, usize>::new();
        for &i in g {
            if !all[i].0.starts_with('?') {
                *counts.entry(all[i].0.as_str()).or_default() += 1;
            }
        }
        if !counts.is_empty() {
            labelled_groups += 1;
            pure += usize::from(counts.len() == 1);
        }
        if g.len() > 1 {
            println!(
                "  group of {:2}: {:?} + {} from group photos",
                g.len(),
                counts,
                g.iter().filter(|&&i| all[i].0.starts_with('?')).count()
            );
        }
    }
    println!(
        "{pure}/{labelled_groups} groups with labelled faces hold only one person; {} single-face groups",
        groups.iter().filter(|g| g.len() == 1).count()
    );
    Ok(())
}
