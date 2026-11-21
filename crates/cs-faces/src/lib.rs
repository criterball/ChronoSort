//! `cs-faces`: find faces in a photo and turn each into a 128-number
//! fingerprint, entirely on this PC.
//!
//! * Detector: OpenCV Zoo **YuNet** 2023mar (MIT), 640×640 BGR input,
//!   anchor-free outputs at strides 8/16/32 (cls, obj, bbox, kps).
//! * Recogniser: OpenCV Zoo **SFace** 2021dec (Apache-2.0), 112×112 RGB
//!   input aligned on 5 landmarks, 128-d output.
//! * Inference: `tract` (pure Rust ONNX), so there is no native runtime and no
//!   telemetry. Post-processing follows OpenCV's `FaceDetectorYN` /
//!   `FaceRecognizerSF` so results match the reference implementation.

pub mod folder;

use image::RgbImage;
use serde::Serialize;
use std::path::Path;
use tract_onnx::prelude::*;

/// Cosine similarity at or above which two faces are the same person
/// (OpenCV's published SFace threshold).
pub const SAME_PERSON: f32 = 0.363;
/// Default minimum detector confidence. OpenCV's demo uses 0.9; a little lower
/// keeps old, soft scans.
pub const MIN_SCORE: f32 = 0.8;

const DET_SIZE: u32 = 640;
const STRIDES: [usize; 3] = [8, 16, 32];
const NMS_IOU: f32 = 0.3;

/// ArcFace 112×112 landmark template (eyes, nose, mouth corners) used by SFace.
const TEMPLATE: [[f32; 2]; 5] =
    [[38.2946, 51.6963], [73.5318, 51.5014], [56.0252, 71.7366], [41.5493, 92.3655], [70.7299, 92.2041]];

type Plan = std::sync::Arc<TypedRunnableModel>;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Face {
    /// x, y, width, height in pixels of the original image.
    pub bbox: [f32; 4],
    /// Eye (image-left), eye (image-right), nose tip, mouth left, mouth right.
    pub landmarks: [[f32; 2]; 5],
    pub score: f32,
}

#[derive(Debug)]
pub struct Error(String);
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Error {}
impl From<TractError> for Error {
    fn from(e: TractError) -> Self {
        Error(format!("face model: {e}"))
    }
}

pub struct FaceModels {
    det: Plan,
    rec: Plan,
}

impl FaceModels {
    pub fn load(detector: &Path, recogniser: &Path) -> Result<Self, Error> {
        let det = tract_onnx::onnx()
            .model_for_path(detector)?
            .with_input_fact(0, f32::fact([1, 3, DET_SIZE as usize, DET_SIZE as usize]).into())?
            .into_optimized()?
            .into_runnable()?;
        let rec = tract_onnx::onnx()
            .model_for_path(recogniser)?
            .with_input_fact(0, f32::fact([1, 3, 112, 112]).into())?
            .into_optimized()?
            .into_runnable()?;
        let m = FaceModels { det, rec };
        if m.det.model().outputs.len() != 12 {
            return Err(Error("this is not the YuNet 2023mar face detector".into()));
        }
        Ok(m)
    }

    /// All faces with confidence ≥ `min_score`, best first.
    pub fn detect(&self, img: &RgbImage, min_score: f32) -> Result<Vec<Face>, Error> {
        let (w, h) = img.dimensions();
        if w == 0 || h == 0 {
            return Ok(vec![]);
        }
        // Letterbox: scale to fit 640×640, pad right/bottom with black.
        let scale = DET_SIZE as f32 / w.max(h) as f32;
        let (sw, sh) = (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
        let small = image::imageops::resize(img, sw, sh, image::imageops::FilterType::Triangle);
        let n = DET_SIZE as usize;
        let mut input = tract_ndarray::Array4::<f32>::zeros((1, 3, n, n));
        for (x, y, p) in small.enumerate_pixels() {
            let (x, y) = (x as usize, y as usize);
            // YuNet expects BGR, 0..255, no normalisation.
            input[[0, 0, y, x]] = p[2] as f32;
            input[[0, 1, y, x]] = p[1] as f32;
            input[[0, 2, y, x]] = p[0] as f32;
        }
        let out = self.det.run(tvec!(Tensor::from(input).into()))?;
        let mut faces = Vec::new();
        for (i, &stride) in STRIDES.iter().enumerate() {
            let cls = &f32s(&out[i])?;
            let obj = &f32s(&out[i + 3])?;
            let bbox = &f32s(&out[i + 6])?;
            let kps = &f32s(&out[i + 9])?;
            let cols = n / stride;
            let cells = cols * cols;
            if cls.len() != cells || obj.len() != cells || bbox.len() != cells * 4 || kps.len() != cells * 10 {
                return Err(Error(format!("unexpected detector output layout at stride {stride}")));
            }
            let s = stride as f32;
            for (idx, (&c, &o)) in cls.iter().zip(obj).enumerate() {
                let score = (c.clamp(0.0, 1.0) * o.clamp(0.0, 1.0)).sqrt();
                if score < min_score {
                    continue;
                }
                let (r, col) = ((idx / cols) as f32, (idx % cols) as f32);
                let cx = (col + bbox[idx * 4]) * s;
                let cy = (r + bbox[idx * 4 + 1]) * s;
                let bw = bbox[idx * 4 + 2].exp() * s;
                let bh = bbox[idx * 4 + 3].exp() * s;
                let mut lm = [[0f32; 2]; 5];
                for (k, p) in lm.iter_mut().enumerate() {
                    *p = [(kps[idx * 10 + 2 * k] + col) * s / scale, (kps[idx * 10 + 2 * k + 1] + r) * s / scale];
                }
                faces.push(Face {
                    bbox: [(cx - bw / 2.0) / scale, (cy - bh / 2.0) / scale, bw / scale, bh / scale],
                    landmarks: lm,
                    score,
                });
            }
        }
        Ok(nms(faces, NMS_IOU))
    }

    /// Unit-length 128-d fingerprint of one detected face.
    pub fn embed(&self, img: &RgbImage, face: &Face) -> Result<Vec<f32>, Error> {
        let aligned = align(img, &face.landmarks);
        let mut input = tract_ndarray::Array4::<f32>::zeros((1, 3, 112, 112));
        for (x, y, p) in aligned.enumerate_pixels() {
            // SFace: RGB (OpenCV swaps BGR->RGB in blobFromImage), 0..255.
            for ch in 0..3 {
                input[[0, ch, y as usize, x as usize]] = p[ch] as f32;
            }
        }
        let out = self.rec.run(tvec!(Tensor::from(input).into()))?;
        let mut v = f32s(&out[0])?;
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
        v.iter_mut().for_each(|x| *x /= norm);
        Ok(v)
    }
}

fn f32s(t: &TValue) -> Result<Vec<f32>, Error> {
    Ok(t.try_as_plain_ram()?.as_slice::<f32>()?.to_vec())
}

/// Cosine similarity of two unit vectors.
pub fn similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Default similarity for putting two faces in the same group: a bit stricter
/// than [`SAME_PERSON`], because one wrong merge mixes two people.
pub const CLUSTER: f32 = 0.42;

fn normalised_mean(vs: &[&[f32]]) -> Vec<f32> {
    let mut m = vec![0f32; vs.first().map_or(0, |v| v.len())];
    for v in vs {
        m.iter_mut().zip(v.iter()).for_each(|(a, b)| *a += b);
    }
    let n = m.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    m.iter_mut().for_each(|x| *x /= n);
    m
}

/// Average fingerprint of a group of faces (unit length), e.g. to compare a
/// whole group with the people the user has named.
pub fn centroid(embs: &[&[f32]]) -> Vec<f32> {
    normalised_mean(embs)
}

/// Square crop around a face with some room for hair and chin, scaled to
/// `side` pixels, for showing to the user (not for recognition: see [`align`]).
pub fn face_crop(img: &RgbImage, face: &Face, side: u32) -> RgbImage {
    let [x, y, w, h] = face.bbox;
    let size = w.max(h) * 1.5;
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let size = size.min(iw).min(ih).max(1.0);
    let left = (cx - size / 2.0).clamp(0.0, iw - size);
    let top = (cy - size / 2.0).clamp(0.0, ih - size);
    let sub = image::imageops::crop_imm(img, left as u32, top as u32, size as u32, size as u32).to_image();
    image::imageops::resize(&sub, side, side, image::imageops::FilterType::Triangle)
}

/// Group fingerprints into people. Returns groups of indices, largest first.
///
/// Pass 1: each face joins the group whose average it is most similar to (if
/// ≥ `threshold`), else starts a new group. Pass 2: groups whose averages are
/// ≥ `threshold` apart are merged. Deterministic for a given input order.
pub fn cluster(embs: &[Vec<f32>], threshold: f32) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut centres: Vec<Vec<f32>> = Vec::new();
    let centre = |g: &[usize]| normalised_mean(&g.iter().map(|&i| embs[i].as_slice()).collect::<Vec<_>>());
    for (i, e) in embs.iter().enumerate() {
        let best = centres.iter().enumerate().map(|(k, c)| (k, similarity(e, c))).max_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            Some((k, s)) if s >= threshold => {
                groups[k].push(i);
                centres[k] = centre(&groups[k]);
            }
            _ => {
                groups.push(vec![i]);
                centres.push(e.clone());
            }
        }
    }
    loop {
        let mut best: Option<(usize, usize, f32)> = None;
        for a in 0..centres.len() {
            for b in a + 1..centres.len() {
                let s = similarity(&centres[a], &centres[b]);
                if s >= threshold && best.is_none_or(|x| s > x.2) {
                    best = Some((a, b, s));
                }
            }
        }
        let Some((a, b, _)) = best else { break };
        let moved = groups.remove(b);
        centres.remove(b);
        groups[a].extend(moved);
        groups[a].sort_unstable();
        centres[a] = centre(&groups[a]);
    }
    groups.sort_by(|x, y| y.len().cmp(&x.len()).then(x[0].cmp(&y[0])));
    groups
}

/// Best matching known person for a fingerprint: (person index, similarity),
/// using the closest of each person's sample fingerprints. None below `SAME_PERSON`.
pub fn identify(emb: &[f32], people: &[Vec<Vec<f32>>]) -> Option<(usize, f32)> {
    people
        .iter()
        .enumerate()
        .filter_map(|(i, samples)| samples.iter().map(|s| similarity(emb, s)).max_by(f32::total_cmp).map(|s| (i, s)))
        .filter(|&(_, s)| s >= SAME_PERSON)
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let (x1, y1) = (a[0].max(b[0]), a[1].max(b[1]));
    let (x2, y2) = ((a[0] + a[2]).min(b[0] + b[2]), (a[1] + a[3]).min(b[1] + b[3]));
    let inter = (x2 - x1).max(0.0) * (y2 - y1).max(0.0);
    let union = a[2] * a[3] + b[2] * b[3] - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

fn nms(mut faces: Vec<Face>, thr: f32) -> Vec<Face> {
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut keep: Vec<Face> = Vec::new();
    for f in faces {
        if keep.iter().all(|k| iou(&k.bbox, &f.bbox) <= thr) {
            keep.push(f);
        }
    }
    keep
}

/// Least-squares similarity transform (rotation + uniform scale + shift)
/// mapping `src` points onto `dst`: returns (a, b, tx, ty) with
/// u = a·x − b·y + tx, v = b·x + a·y + ty.
fn similarity_transform(src: &[[f32; 2]; 5], dst: &[[f32; 2]; 5]) -> (f32, f32, f32, f32) {
    let mean = |p: &[[f32; 2]; 5]| {
        let (sx, sy) = p.iter().fold((0.0, 0.0), |(x, y), q| (x + q[0], y + q[1]));
        (sx / 5.0, sy / 5.0)
    };
    let ((mx, my), (mu, mv)) = (mean(src), mean(dst));
    let (mut num_a, mut num_b, mut den) = (0f32, 0f32, 0f32);
    for (s, d) in src.iter().zip(dst) {
        let (x, y, u, v) = (s[0] - mx, s[1] - my, d[0] - mu, d[1] - mv);
        num_a += x * u + y * v;
        num_b += x * v - y * u;
        den += x * x + y * y;
    }
    let den = den.max(1e-6);
    let (a, b) = (num_a / den, num_b / den);
    (a, b, mu - a * mx + b * my, mv - b * mx - a * my)
}

/// 112×112 face crop aligned to the ArcFace template (bilinear sampling).
pub fn align(img: &RgbImage, lm: &[[f32; 2]; 5]) -> RgbImage {
    let (a, b, tx, ty) = similarity_transform(lm, &TEMPLATE);
    let det = (a * a + b * b).max(1e-12);
    let (w, h) = img.dimensions();
    let mut out = RgbImage::new(112, 112);
    for (u, v, px) in out.enumerate_pixels_mut() {
        let (du, dv) = (u as f32 - tx, v as f32 - ty);
        // Inverse of the similarity transform.
        let x = (a * du + b * dv) / det;
        let y = (-b * du + a * dv) / det;
        *px = bilinear(img, x, y, w, h);
    }
    out
}

fn bilinear(img: &RgbImage, x: f32, y: f32, w: u32, h: u32) -> image::Rgb<u8> {
    if x < 0.0 || y < 0.0 || x > (w - 1) as f32 || y > (h - 1) as f32 {
        return image::Rgb([0, 0, 0]);
    }
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let (p00, p10, p01, p11) =
        (img.get_pixel(x0, y0), img.get_pixel(x1, y0), img.get_pixel(x0, y1), img.get_pixel(x1, y1));
    let mut c = [0u8; 3];
    for (i, ch) in c.iter_mut().enumerate() {
        let top = p00[i] as f32 * (1.0 - fx) + p10[i] as f32 * fx;
        let bot = p01[i] as f32 * (1.0 - fx) + p11[i] as f32 * fx;
        *ch = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
    }
    image::Rgb(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_crop_stays_inside_the_image() {
        let img = RgbImage::from_pixel(200, 100, image::Rgb([9, 9, 9]));
        // A face at the very edge, bigger than the image is tall.
        let f = Face { bbox: [180.0, -10.0, 60.0, 130.0], landmarks: [[0.0; 2]; 5], score: 1.0 };
        let c = face_crop(&img, &f, 96);
        assert_eq!(c.dimensions(), (96, 96));
        let c = centroid(&[&[1.0, 0.0], &[0.0, 1.0]]);
        assert!((c[0] - c[1]).abs() < 1e-6 && (c[0] * c[0] + c[1] * c[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn similarity_transform_recovers_known_motion() {
        // Rotate 30°, scale 1.7, shift (5, -3): the fit must undo it exactly.
        let (ang, sc) = (30f32.to_radians(), 1.7f32);
        let (a, b) = (sc * ang.cos(), sc * ang.sin());
        let mut src = [[0f32; 2]; 5];
        for (i, s) in src.iter_mut().enumerate() {
            let [u, v] = TEMPLATE[i];
            // src = inverse(T)(template)
            let (du, dv) = (u - 5.0, v + 3.0);
            let d = a * a + b * b;
            *s = [(a * du + b * dv) / d, (-b * du + a * dv) / d];
        }
        let (ra, rb, rtx, rty) = similarity_transform(&src, &TEMPLATE);
        assert!((ra - a).abs() < 1e-3 && (rb - b).abs() < 1e-3, "{ra} {rb}");
        assert!((rtx - 5.0).abs() < 1e-2 && (rty + 3.0).abs() < 1e-2, "{rtx} {rty}");
    }

    #[test]
    fn nms_keeps_best_of_overlaps() {
        let f = |x: f32, s: f32| Face { bbox: [x, 0.0, 10.0, 10.0], landmarks: [[0.0; 2]; 5], score: s };
        let kept = nms(vec![f(0.0, 0.8), f(1.0, 0.9), f(50.0, 0.85)], 0.3);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].score, 0.9);
        assert_eq!(kept[1].bbox[0], 50.0);
    }

    fn unit(v: &[f32]) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter().map(|x| x / n).collect()
    }

    #[test]
    fn clusters_two_people_and_identifies() {
        // Two "people" around different directions, with small variations.
        let a = |d: f32| unit(&[1.0, d, 0.0, 0.0]);
        let b = |d: f32| unit(&[0.0, 0.0, 1.0, d]);
        let embs = vec![a(0.1), b(0.0), a(-0.2), b(0.3), a(0.0), unit(&[0.6, 0.0, 0.6, 0.0])];
        let g = cluster(&embs, 0.8);
        assert_eq!(g[0], vec![0, 2, 4]);
        assert_eq!(g[1], vec![1, 3]);
        assert_eq!(g.len(), 3); // the in-between face stays alone at this strictness
        let people = vec![vec![a(0.0)], vec![b(0.0)]];
        assert_eq!(identify(&a(0.15), &people).map(|x| x.0), Some(0));
        assert_eq!(identify(&b(-0.1), &people).map(|x| x.0), Some(1));
        assert_eq!(identify(&unit(&[0.0, 1.0, 0.0, 0.0]), &[vec![unit(&[0.0, 0.0, 0.0, 1.0])]]), None);
    }

    #[test]
    fn similarity_of_unit_vectors() {
        let a = [1.0, 0.0];
        assert!((similarity(&a, &a) - 1.0).abs() < 1e-6);
        assert!(similarity(&a, &[0.0, 1.0]).abs() < 1e-6);
    }
}
