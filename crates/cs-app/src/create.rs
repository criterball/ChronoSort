//! "Create" tools in the file browser: a contact sheet or an image strip (one
//! picture made of many), a PDF with one photo per page, and a ZIP of files.
//! The new file goes next to the first selected file and never replaces
//! anything; Ctrl+Z removes it again.
//!
//! Sheets and strips are laid out by the app (cell rectangles) and their text
//! (title, captions) arrives as a transparent PNG drawn by the page, so no
//! fonts have to be shipped. Progress: `create-progress` `{ done, total, name }`.

use crate::fileops::{self, Op, Undo};
use crate::{Res, err};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Largest sheet: 120 megapixels.
const MAX_SHEET_PIXELS: u64 = 120_000_000;

pub use crate::batch::Progress;

#[derive(Serialize)]
pub struct Created {
    pub path: String,
    pub name: String,
    pub skipped: Vec<(String, String)>,
}

fn files_of(files: &[PathBuf]) -> Res<Vec<PathBuf>> {
    let v: Vec<PathBuf> = files.iter().filter(|p| p.is_file()).cloned().collect();
    if v.is_empty() {
        return Err("None of these files can be found any more. Press F5 to refresh.".into());
    }
    Ok(v)
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A new file name in `dir`: the user's name (checked) with `ext`, never replacing a file.
fn target(dir: &Path, name: &str, ext: &str) -> Res<PathBuf> {
    let stem = name.trim().trim_end_matches(&format!(".{ext}")).trim();
    let n = fileops::check_name(&format!("{stem}.{ext}"))?;
    let p = fileops::free_name(dir, &n);
    fileops::guard(&p)?;
    Ok(p)
}

fn remember(p: &Path, label: String) -> Res<()> {
    let m = std::fs::metadata(p).map_err(err)?;
    fileops::push(Op {
        label,
        undo: Undo::Copied { files: vec![(p.to_path_buf(), m.len(), m.modified().ok())], dirs: vec![] },
    });
    Ok(())
}

/// The picture of a file, upright, as RGB. `small`: a preview is enough (contact sheets).
fn picture(p: &Path, small: bool) -> Res<image::RgbImage> {
    let fmt = crate::real_format(p);
    if small || fmt.kind() == cs_model::MediaKind::Video {
        return Ok(cs_thumb::load(p, fmt, cs_thumb::PREVIEW_SIZE).map_err(err)?.to_rgb8());
    }
    let photo = crate::open_any(p)?;
    cs_image::preview_rgb8(&photo.image, u32::MAX)
}

// ======================================================================= sizes

/// Upright width × height of each file (EXIF turn applied), for laying out strips.
/// `None` when the size cannot be read.
pub fn sizes(files: &[PathBuf]) -> Vec<Option<(u32, u32)>> {
    {
        files
            .iter()
            .map(|p| {
                let p = p.clone();
                let fmt = crate::real_format(&p);
                if fmt.kind() == cs_model::MediaKind::Video {
                    return cs_video::play::info(&p).ok().map(|i| (i.width, i.height));
                }
                let (w, h) = image::ImageReader::open(&p)
                    .ok()
                    .and_then(|r| r.with_guessed_format().ok()?.into_dimensions().ok())
                    .or_else(|| {
                        cs_meta::raw_preview(&p)
                            .and_then(|j| image::load_from_memory(&j).ok())
                            .map(|i| (i.width(), i.height()))
                    })?;
                let turned = matches!(cs_meta::orientation_and_thumbnail(&p, fmt).0, Some(5..=8));
                Some(if turned { (h, w) } else { (w, h) })
            })
            .collect::<Vec<_>>()
    }
}

// ======================================================================= sheet

#[derive(Deserialize, Clone, Copy, Debug)]
pub struct Cell {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SheetOpts {
    pub width: u32,
    pub height: u32,
    pub background: [u8; 3],
    /// One cell per id, in the same order.
    pub cells: Vec<Cell>,
    /// "contain" (whole picture) or "cover" (fill the cell, trim the edges).
    pub fit: String,
    /// Transparent PNG of the sheet's size with the title and captions.
    pub overlay: Option<Vec<u8>>,
    /// "JPEG" or "PNG".
    pub format: String,
    pub quality: u8,
    pub name: String,
}

fn place(canvas: &mut image::RgbImage, pic: &image::RgbImage, c: Cell, cover: bool) {
    use image::imageops::FilterType;
    let (pw, ph) = pic.dimensions();
    if pw == 0 || ph == 0 || c.w == 0 || c.h == 0 {
        return;
    }
    let sx = f64::from(c.w) / f64::from(pw);
    let sy = f64::from(c.h) / f64::from(ph);
    let s = if cover { sx.max(sy) } else { sx.min(sy) };
    let (nw, nh) = (((f64::from(pw) * s).round() as u32).max(1), ((f64::from(ph) * s).round() as u32).max(1));
    let small = image::imageops::resize(pic, nw, nh, if s < 0.5 { FilterType::Triangle } else { FilterType::Lanczos3 });
    // Centre in the cell; with "cover" the overhang is cut off.
    let ox = i64::from(c.x) + (i64::from(c.w) - i64::from(nw)) / 2;
    let oy = i64::from(c.y) + (i64::from(c.h) - i64::from(nh)) / 2;
    let (cw, ch) = canvas.dimensions();
    for y in 0..nh {
        let ty = oy + i64::from(y);
        if ty < i64::from(c.y) || ty >= i64::from(c.y + c.h) || ty < 0 || ty >= i64::from(ch) {
            continue;
        }
        for x in 0..nw {
            let tx = ox + i64::from(x);
            if tx < i64::from(c.x) || tx >= i64::from(c.x + c.w) || tx < 0 || tx >= i64::from(cw) {
                continue;
            }
            canvas.put_pixel(tx as u32, ty as u32, *small.get_pixel(x, y));
        }
    }
}

fn blend_overlay(canvas: &mut image::RgbImage, png: &[u8]) -> Res<()> {
    let layer = image::load_from_memory_with_format(png, image::ImageFormat::Png).map_err(err)?.to_rgba8();
    if layer.dimensions() != canvas.dimensions() {
        return Err("The text layer does not match the sheet's size.".into());
    }
    for (p, l) in canvas.pixels_mut().zip(layer.pixels()) {
        let a = u16::from(l.0[3]);
        if a == 0 {
            continue;
        }
        for c in 0..3 {
            p.0[c] = ((u16::from(l.0[c]) * a + u16::from(p.0[c]) * (255 - a) + 127) / 255) as u8;
        }
    }
    Ok(())
}

/// Compose many pictures into one (contact sheet or strip) and save it next to the first.
/// Blocking. `progress` is called as pictures are loaded (from several threads).
pub fn sheet(files: &[PathBuf], opts: SheetOpts, progress: impl Fn(Progress) + Sync) -> Res<Created> {
    if opts.cells.len() != files.len() {
        return Err("The layout does not match the pictures.".into());
    }
    if opts.width == 0 || opts.height == 0 || u64::from(opts.width) * u64::from(opts.height) > MAX_SHEET_PIXELS {
        return Err("That picture would be too big (at most 120 megapixels).".into());
    }
    let files: Vec<(u64, PathBuf)> = files.iter().cloned().enumerate().map(|(i, p)| (i as u64, p)).collect();
    if files.iter().any(|(_, p)| !p.is_file()) {
        return Err("Some of these files are no longer there. Press F5 to refresh.".into());
    }
    {
        let (fmt, ext) = if opts.format.eq_ignore_ascii_case("PNG") {
            (image::ImageFormat::Png, "png")
        } else {
            (image::ImageFormat::Jpeg, "jpg")
        };
        let dir = files[0].1.parent().ok_or("no folder")?.to_path_buf();
        let out = target(&dir, &opts.name, ext)?;
        let mut canvas = image::RgbImage::from_pixel(opts.width, opts.height, image::Rgb(opts.background));
        let cover = opts.fit == "cover";
        let total = files.len();
        let done = AtomicUsize::new(0);
        // Load in parallel (a few at a time), place one by one.
        let pics: Vec<Res<image::RgbImage>> = {
            use rayon::prelude::*;
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 4))
                .build()
                .map_err(err)?;
            pool.install(|| {
                files
                    .par_iter()
                    .zip(opts.cells.par_iter())
                    .map(|((_, p), c)| {
                        let small = c.w.max(c.h) <= cs_thumb::PREVIEW_SIZE;
                        let r = picture(p, small);
                        let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                        progress(Progress { done: n, total, name: file_name(p) });
                        r
                    })
                    .collect()
            })
        };
        let mut skipped = Vec::new();
        for (((_, p), c), pic) in files.iter().zip(&opts.cells).zip(pics) {
            match pic {
                Ok(pic) => place(&mut canvas, &pic, *c, cover),
                Err(e) => skipped.push((file_name(p), e)),
            }
        }
        if let Some(o) = &opts.overlay {
            blend_overlay(&mut canvas, o)?;
        }
        let mut bytes = Vec::new();
        if fmt == image::ImageFormat::Jpeg {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, opts.quality.clamp(50, 100))
                .encode_image(&canvas)
                .map_err(err)?;
        } else {
            canvas.write_to(&mut std::io::Cursor::new(&mut bytes), fmt).map_err(err)?;
        }
        write_new(&out, &bytes)?;
        remember(&out, format!("Create “{}”", file_name(&out)))?;
        Ok(Created { path: out.to_string_lossy().into_owned(), name: file_name(&out), skipped })
    }
}

/// Write a new file (never replacing one), flushed to disk.
fn write_new(p: &Path, bytes: &[u8]) -> Res<()> {
    let mut f = std::fs::File::options().write(true).create_new(true).open(p).map_err(err)?;
    f.write_all(bytes).map_err(err)?;
    f.sync_all().map_err(err)
}

// ========================================================================= PDF

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfOpts {
    /// "a4" or "letter"
    pub page: String,
    /// Margin in millimetres.
    pub margin_mm: f32,
    pub name: String,
}

/// One page's picture: JPEG bytes, size in pixels, colour components (1 or 3).
struct PdfImage {
    jpeg: Vec<u8>,
    w: u32,
    h: u32,
    comps: u8,
}

/// Longest side of a re-encoded page picture (300 dpi on A4).
const PDF_MAX_SIDE: u32 = 3508;

fn pdf_image(p: &Path) -> Res<PdfImage> {
    let fmt = crate::real_format(p);
    if fmt.kind() == cs_model::MediaKind::Video {
        let img = cs_thumb::load(p, fmt, cs_thumb::PREVIEW_SIZE).map_err(err)?.to_rgb8();
        return encode_pdf_jpeg(&img);
    }
    let photo = crate::open_any(p)?;
    // An upright colour JPEG goes in exactly as it is (no quality loss, fast).
    if photo.format == cs_image::Format::Jpeg
        && photo.orientation == 1
        && photo.image.layout() == cs_image::ChannelLayout::Rgb
        && std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX) <= 40 * 1024 * 1024
    {
        let (w, h) = photo.image.dimensions();
        return Ok(PdfImage { jpeg: std::fs::read(p).map_err(err)?, w, h, comps: 3 });
    }
    let rgb = cs_image::preview_rgb8(&photo.image, PDF_MAX_SIDE)?;
    encode_pdf_jpeg(&rgb)
}

fn encode_pdf_jpeg(img: &image::RgbImage) -> Res<PdfImage> {
    let img = if img.width().max(img.height()) > PDF_MAX_SIDE {
        let s = f64::from(PDF_MAX_SIDE) / f64::from(img.width().max(img.height()));
        image::imageops::resize(
            img,
            ((f64::from(img.width()) * s).round() as u32).max(1),
            ((f64::from(img.height()) * s).round() as u32).max(1),
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img.clone()
    };
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90).encode_image(&img).map_err(err)?;
    Ok(PdfImage { jpeg, w: img.width(), h: img.height(), comps: 3 })
}

/// A minimal PDF: one page per picture, the picture centred and as large as the margins allow.
/// Portrait pictures get portrait pages, landscape ones landscape pages.
fn write_pdf(out: &mut Vec<u8>, pages: &[PdfImage], page_pt: (f32, f32), margin_pt: f32) {
    let mut offsets: Vec<usize> = Vec::new();
    let n = pages.len();
    // Objects: 1 catalog, 2 pages, then per page: page, content, image (3 each).
    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    obj(out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + i * 3)).collect();
    obj(out, &mut offsets, format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), n).as_bytes());
    for (i, p) in pages.iter().enumerate() {
        let (pw, ph) = if p.w > p.h {
            (page_pt.0.max(page_pt.1), page_pt.0.min(page_pt.1))
        } else {
            (page_pt.0.min(page_pt.1), page_pt.0.max(page_pt.1))
        };
        let (aw, ah) = ((pw - 2.0 * margin_pt).max(10.0), (ph - 2.0 * margin_pt).max(10.0));
        let s = (aw / p.w as f32).min(ah / p.h as f32);
        let (iw, ih) = (p.w as f32 * s, p.h as f32 * s);
        let (x, y) = ((pw - iw) / 2.0, (ph - ih) / 2.0);
        let (page_id, content_id, image_id) = (3 + i * 3, 4 + i * 3, 5 + i * 3);
        obj(
            out,
            &mut offsets,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im{i} {image_id} 0 R >> >> /Contents {content_id} 0 R >>"
            )
            .as_bytes(),
        );
        debug_assert_eq!(offsets.len(), page_id);
        let stream = format!("q {iw:.3} 0 0 {ih:.3} {x:.3} {y:.3} cm /Im{i} Do Q");
        obj(out, &mut offsets, format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()).as_bytes());
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /{} /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            p.w,
            p.h,
            if p.comps == 1 { "DeviceGray" } else { "DeviceRGB" },
            p.jpeg.len()
        )
        .into_bytes();
        body.extend_from_slice(&p.jpeg);
        body.extend_from_slice(b"\nendstream");
        obj(out, &mut offsets, &body);
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes(),
    );
}

/// Make a PDF with one photo per page.
/// Blocking.
pub fn pdf(files: &[PathBuf], opts: PdfOpts, progress: impl Fn(Progress)) -> Res<Created> {
    let files: Vec<(u64, PathBuf)> = files_of(files)?.into_iter().enumerate().map(|(i, p)| (i as u64, p)).collect();
    {
        let dir = files[0].1.parent().ok_or("no folder")?.to_path_buf();
        let out = target(&dir, &opts.name, "pdf")?;
        let page = if opts.page == "letter" { (612.0, 792.0) } else { (595.28, 841.89) };
        let margin = opts.margin_mm.clamp(0.0, 50.0) / 25.4 * 72.0;
        let total = files.len();
        let mut pages = Vec::new();
        let mut skipped = Vec::new();
        for (i, (_, p)) in files.iter().enumerate() {
            match pdf_image(p) {
                Ok(img) => pages.push(img),
                Err(e) => skipped.push((file_name(p), e)),
            }
            progress(Progress { done: i + 1, total, name: file_name(p) });
        }
        if pages.is_empty() {
            return Err("None of these files could be put into a PDF.".into());
        }
        let mut bytes = Vec::new();
        write_pdf(&mut bytes, &pages, page, margin);
        write_new(&out, &bytes)?;
        remember(&out, format!("Create “{}”", file_name(&out)))?;
        Ok(Created { path: out.to_string_lossy().into_owned(), name: file_name(&out), skipped })
    }
}

// ========================================================================= ZIP

/// Already-compressed kinds are stored as they are (zipping them again only costs time).
fn stored(p: &Path) -> bool {
    let e = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    matches!(
        e.as_str(),
        "jpg"
            | "jpeg"
            | "png"
            | "gif"
            | "webp"
            | "heic"
            | "heif"
            | "avif"
            | "mp4"
            | "mov"
            | "m4v"
            | "mkv"
            | "webm"
            | "avi"
            | "mp3"
            | "m4a"
            | "zip"
            | "7z"
            | "rar"
            | "gz"
            | "cr2"
            | "cr3"
            | "nef"
            | "arw"
            | "dng"
            | "orf"
            | "rw2"
            | "raf"
    )
}

/// Pack files into a new ZIP next to the first one (names made unique inside the ZIP).
/// Blocking.
pub fn zip(files: &[PathBuf], name: &str, progress: impl Fn(Progress)) -> Res<Created> {
    let files = files_of(files)?;
    {
        let name = name.to_string();
        let dir = files[0].parent().ok_or("no folder")?.to_path_buf();
        let out = target(&dir, &name, "zip")?;
        let tmp = out.with_extension("zip.part");
        let r = (|| -> Res<Vec<(String, String)>> {
            let f = std::fs::File::options().write(true).create_new(true).open(&tmp).map_err(err)?;
            let mut z = zip::ZipWriter::new(std::io::BufWriter::new(f));
            let mut used = std::collections::HashSet::new();
            let total = files.len();
            let skipped = Vec::new();
            for (i, p) in files.iter().enumerate() {
                let base = file_name(p);
                let mut entry = base.clone();
                let mut n = 1;
                while !used.insert(entry.to_lowercase()) {
                    let (s, e) = base.rsplit_once('.').map_or((base.as_str(), ""), |(s, e)| (s, e));
                    entry = if e.is_empty() { format!("{s}_{n}") } else { format!("{s}_{n}.{e}") };
                    n += 1;
                }
                let size = std::fs::metadata(p).map_err(err)?.len();
                let method = if stored(p) { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated };
                let opts = zip::write::SimpleFileOptions::default()
                    .compression_method(method)
                    .large_file(size >= u64::from(u32::MAX));
                z.start_file(entry.as_str(), opts).map_err(err)?;
                let mut src = std::fs::File::open(p).map_err(err)?;
                std::io::copy(&mut src, &mut z).map_err(err)?;
                progress(Progress { done: i + 1, total, name: base });
            }
            let mut w = z.finish().map_err(err)?;
            w.flush().map_err(err)?;
            w.get_ref().sync_all().map_err(err)?;
            Ok(skipped)
        })();
        let skipped = match r {
            Ok(s) => s,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
        };
        cs_platform::rename_no_replace(&tmp, &out).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })?;
        remember(&out, format!("Create “{}”", file_name(&out)))?;
        Ok(Created { path: out.to_string_lossy().into_owned(), name: file_name(&out), skipped })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_contains_and_covers_inside_the_cell() {
        let mut c = image::RgbImage::from_pixel(100, 50, image::Rgb([255, 255, 255]));
        let pic = image::RgbImage::from_pixel(40, 20, image::Rgb([200, 0, 0]));
        place(&mut c, &pic, Cell { x: 10, y: 10, w: 20, h: 20 }, false);
        assert_eq!(c.get_pixel(20, 20).0, [200, 0, 0]); // middle
        assert_eq!(c.get_pixel(20, 11).0, [255, 255, 255]); // letterbox above (picture is 20×10)
        assert_eq!(c.get_pixel(31, 20).0, [255, 255, 255]); // outside the cell
        let mut c = image::RgbImage::from_pixel(100, 50, image::Rgb([255, 255, 255]));
        place(&mut c, &pic, Cell { x: 10, y: 10, w: 20, h: 20 }, true);
        assert_eq!(c.get_pixel(20, 11).0, [200, 0, 0]); // filled
        assert_eq!(c.get_pixel(30, 20).0, [255, 255, 255]); // never past the cell
        assert_eq!(c.get_pixel(9, 20).0, [255, 255, 255]);
    }

    #[test]
    fn pdf_is_well_formed() {
        let mut j = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut j, 80)
            .encode_image(&image::RgbImage::from_pixel(30, 20, image::Rgb([1, 2, 3])))
            .unwrap();
        let pages =
            vec![PdfImage { jpeg: j.clone(), w: 30, h: 20, comps: 3 }, PdfImage { jpeg: j, w: 20, h: 30, comps: 3 }];
        let mut out = Vec::new();
        write_pdf(&mut out, &pages, (595.28, 841.89), 28.35);
        let s = String::from_utf8_lossy(&out);
        assert!(s.starts_with("%PDF-1.4"));
        assert!(s.contains("/Count 2"));
        assert!(s.contains("/MediaBox [0 0 841.89 595.28]")); // landscape picture → landscape page
        assert!(s.contains("/MediaBox [0 0 595.28 841.89]"));
        // Every xref offset points at "N 0 obj".
        let xref = s.rfind("\nxref\n").unwrap() + 1; // not the "startxref" at the end
        let entries: Vec<usize> =
            s[xref..].lines().skip(3).take_while(|l| l.ends_with(" n ")).map(|l| l[..10].parse().unwrap()).collect();
        assert_eq!(entries.len(), 8);
        for (i, off) in entries.iter().enumerate() {
            assert!(out[*off..].starts_with(format!("{} 0 obj", i + 1).as_bytes()), "object {}", i + 1);
        }
        assert!(s.trim_end().ends_with("%%EOF"));
    }

    #[test]
    fn stored_kinds() {
        assert!(stored(Path::new("a.JPG")));
        assert!(stored(Path::new("a.mp4")));
        assert!(!stored(Path::new("a.txt")));
        assert!(!stored(Path::new("a.tif")));
    }
}
