//! Faces in a whole folder: scan (read-only), detect, fingerprint, group.

use crate::{CLUSTER, Face, FaceModels, MIN_SCORE, cluster};
use cs_model::MediaKind;
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Longest side photos are decoded at: plenty for YuNet (which works at 640)
/// and keeps small faces sharp enough for the 112×112 fingerprint crop.
pub const LOAD_SIDE: u32 = 1600;
/// Faces smaller than this (pixels, shortest bbox side at LOAD_SIDE) give
/// unreliable fingerprints and are skipped.
pub const MIN_FACE_PX: f32 = 40.0;

#[derive(Debug, Clone, Serialize)]
pub struct FoundFace {
    /// Path relative to the scanned folder, `/`-separated.
    pub rel: String,
    /// Content-sniffed format name (cs_scan::Format::name), for reloading.
    pub format: String,
    pub face: Face,
    /// Faces in the same photo (1 = a portrait).
    pub faces_in_photo: usize,
    #[serde(skip)]
    pub embedding: Vec<f32>,
}

#[derive(Debug, Default, Serialize)]
pub struct FaceScan {
    pub root: PathBuf,
    pub photos: usize,
    pub photos_with_faces: usize,
    pub faces: Vec<FoundFace>,
    /// Groups of indices into `faces`, largest first.
    pub groups: Vec<Vec<usize>>,
    pub errors: Vec<(String, String)>,
}

/// Faces of one photo with their fingerprints, or why it could not be read.
type PhotoFaces = Result<Vec<(Face, Vec<f32>)>, String>;

/// `progress(done, total)` is called as photos are processed.
pub fn scan_folder(
    root: &Path,
    models: &FaceModels,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> std::io::Result<FaceScan> {
    let scan = cs_scan::scan(root)?;
    let photos: Vec<_> = scan.files.iter().filter(|f| f.kind == MediaKind::Image).collect();
    let total = photos.len();
    let done = AtomicUsize::new(0);
    let results: Vec<(String, &'static str, PhotoFaces)> = photos
        .par_iter()
        .map(|f| {
            let abs = root.join(f.rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            let r = (|| {
                let img = cs_thumb::load(&abs, f.format, LOAD_SIDE).map_err(|e| e.to_string())?.to_rgb8();
                let faces = models.detect(&img, MIN_SCORE).map_err(|e| e.to_string())?;
                faces
                    .into_iter()
                    .filter(|x| x.bbox[2].min(x.bbox[3]) >= MIN_FACE_PX)
                    .map(|x| models.embed(&img, &x).map(|e| (x, e)).map_err(|e| e.to_string()))
                    .collect::<Result<Vec<_>, _>>()
            })();
            progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
            (f.rel.clone(), f.format.name(), r)
        })
        .collect();

    let mut out = FaceScan { root: root.to_path_buf(), photos: total, ..Default::default() };
    out.errors = scan.errors.iter().map(|(p, e)| (p.display().to_string(), e.clone())).collect();
    for (rel, format, r) in results {
        match r {
            Ok(faces) => {
                out.photos_with_faces += usize::from(!faces.is_empty());
                let n = faces.len();
                for (face, embedding) in faces {
                    out.faces.push(FoundFace {
                        rel: rel.clone(),
                        format: format.to_string(),
                        face,
                        faces_in_photo: n,
                        embedding,
                    });
                }
            }
            Err(e) => out.errors.push((rel, e)),
        }
    }
    // Stable order (paths sorted) so grouping is repeatable.
    out.faces.sort_by(|a, b| a.rel.cmp(&b.rel).then(a.face.bbox[0].total_cmp(&b.face.bbox[0])));
    let embs: Vec<Vec<f32>> = out.faces.iter().map(|f| f.embedding.clone()).collect();
    out.groups = cluster(&embs, CLUSTER);
    Ok(out)
}
