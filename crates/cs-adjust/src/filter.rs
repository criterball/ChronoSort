//! Spatial building blocks on single-channel `f32` planes: box blur (O(1) per
//! pixel, any radius), a near-Gaussian blur from three boxes, and the
//! edge-aware guided filter (He et al.), which smooths inside regions but keeps
//! edges, so local tone changes do not make halos.

use rayon::prelude::*;

/// A single-channel picture, row-major.
#[derive(Clone)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub v: Vec<f32>,
}

impl Plane {
    pub fn new(w: usize, h: usize, v: Vec<f32>) -> Self {
        debug_assert_eq!(v.len(), w * h);
        Plane { w, h, v }
    }
}

/// Box blur of radius `r` along rows, edges clamped.
fn box_rows(src: &[f32], w: usize, r: usize, out: &mut [f32]) {
    if w == 0 {
        return;
    }
    out.par_chunks_mut(w).zip(src.par_chunks(w)).for_each(|(o, s)| {
        let r = r.min(w.saturating_sub(1));
        let n = (2 * r + 1) as f32;
        let at = |i: isize| s[i.clamp(0, w as isize - 1) as usize];
        let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
        for (x, ov) in o.iter_mut().enumerate() {
            *ov = sum / n;
            let x = x as isize;
            sum += at(x + r as isize + 1) - at(x - r as isize);
        }
    });
}

/// Box blur of radius `r` down the columns, edges clamped. Runs row by row
/// with one running sum per column (cache friendly, no transpose); bands of
/// columns go to different threads.
fn box_cols(src: &[f32], w: usize, h: usize, r: usize, out: &mut [f32]) {
    const BAND: usize = 256;
    let r = r.min(h.saturating_sub(1));
    let n = (2 * r + 1) as f32;
    let row = |y: isize| y.clamp(0, h as isize - 1) as usize * w;
    let bands: Vec<(usize, Vec<f32>)> = (0..w.div_ceil(BAND))
        .into_par_iter()
        .map(|b| {
            let (x0, x1) = (b * BAND, ((b + 1) * BAND).min(w));
            let bw = x1 - x0;
            let mut sum = vec![0.0f32; bw];
            for y in -(r as isize)..=r as isize {
                let s = &src[row(y) + x0..row(y) + x1];
                sum.iter_mut().zip(s).for_each(|(a, v)| *a += v);
            }
            let mut o = vec![0.0f32; bw * h];
            for y in 0..h {
                o[y * bw..(y + 1) * bw].iter_mut().zip(&sum).for_each(|(o, s)| *o = s / n);
                let (add, sub) = (row(y as isize + r as isize + 1), row(y as isize - r as isize));
                for (i, s) in sum.iter_mut().enumerate() {
                    *s += src[add + x0 + i] - src[sub + x0 + i];
                }
            }
            (x0, o)
        })
        .collect();
    for (x0, o) in bands {
        let bw = o.len() / h.max(1);
        for y in 0..h {
            out[y * w + x0..y * w + x0 + bw].copy_from_slice(&o[y * bw..(y + 1) * bw]);
        }
    }
}

/// Box blur with radius `r` in both directions.
pub fn box_blur(p: &Plane, r: usize) -> Plane {
    if r == 0 || p.v.is_empty() {
        return p.clone();
    }
    let mut a = vec![0.0; p.v.len()];
    box_rows(&p.v, p.w, r, &mut a);
    let mut b = vec![0.0; a.len()];
    box_cols(&a, p.w, p.h, r, &mut b);
    Plane::new(p.w, p.h, b)
}

/// Near-Gaussian blur with standard deviation `sigma` (three box passes).
pub fn gauss(p: &Plane, sigma: f32) -> Plane {
    if sigma < 0.3 {
        return p.clone();
    }
    // Box radius whose three passes give about this sigma: var(box r) = r(r+1)/3.
    let r = (((sigma * sigma + 0.25).sqrt() - 0.5).round() as usize).max(1);
    let a = box_blur(p, r);
    let b = box_blur(&a, r);
    box_blur(&b, r)
}

/// Self-guided filter: an edge-preserving smoothing of `p` with window radius
/// `r` and regularisation `eps` (bigger = smoother across edges).
pub fn guided(p: &Plane, r: usize, eps: f32) -> Plane {
    if r == 0 || p.v.is_empty() {
        return p.clone();
    }
    let mean = box_blur(p, r);
    let sq = Plane::new(p.w, p.h, p.v.par_iter().map(|x| x * x).collect());
    let corr = box_blur(&sq, r);
    let (a, b): (Vec<f32>, Vec<f32>) = mean
        .v
        .par_iter()
        .zip(corr.v.par_iter())
        .map(|(&m, &c)| {
            let var = (c - m * m).max(0.0);
            let a = var / (var + eps);
            (a, m - a * m)
        })
        .unzip();
    let ma = box_blur(&Plane::new(p.w, p.h, a), r);
    let mb = box_blur(&Plane::new(p.w, p.h, b), r);
    Plane::new(
        p.w,
        p.h,
        p.v.par_iter().zip(ma.v.par_iter().zip(mb.v.par_iter())).map(|(i, (a, b))| a * i + b).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_blur_keeps_flat_areas_and_averages() {
        let p = Plane::new(5, 1, vec![0.0, 0.0, 3.0, 0.0, 0.0]);
        let b = box_blur(&p, 1);
        assert!((b.v[2] - 1.0).abs() < 1e-6 && (b.v[1] - 1.0).abs() < 1e-6 && b.v[0] == 0.0);
        let flat = Plane::new(4, 3, vec![0.5; 12]);
        assert!(gauss(&flat, 3.0).v.iter().all(|v| (v - 0.5).abs() < 1e-6));
    }

    #[test]
    fn guided_filter_keeps_a_strong_edge() {
        // Step edge plus small noise: the noise is smoothed, the step stays.
        let w = 40;
        let v: Vec<f32> = (0..w * w)
            .map(|i| {
                let x = i % w;
                (if x < w / 2 { 0.0 } else { 4.0 }) + if i % 2 == 0 { 0.05 } else { -0.05 }
            })
            .collect();
        let g = guided(&Plane::new(w, w, v), 4, 0.01);
        let row = &g.v[20 * w..21 * w];
        assert!(row[5].abs() < 0.03 && (row[35] - 4.0).abs() < 0.03, "flat parts smoothed");
        assert!(row[19] < 0.5 && row[20] > 3.5, "edge kept: {} {}", row[19], row[20]);
    }
}
