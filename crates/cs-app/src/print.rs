//! Printing photos: the page layouts of the Tauri app's print dialog — one photo per page (the page turns to
//! the photo), two or four per page, or exact photo sizes (9 × 13 … 20 × 30 cm, with dashed cut guides) —
//! fit or fill, optional captions, A4 or Letter, a margin. Each page is drawn at 300 dpi and written as a
//! small PDF (one JPEG per page), which can be saved as a file; printing goes through the Windows Print
//! dialog (`cs_platform::print`). Captions are drawn by the caller (the UI owns the font).

use crate::{Res, err};
use image::{Rgb, RgbImage, imageops};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Layout {
    One,
    Two,
    Four,
    /// A contact sheet: `cols` × `rows` small photos per page, each with its file name underneath.
    Grid {
        cols: u32,
        rows: u32,
    },
    /// Exact photo size in centimetres (the long side follows the photo's own shape).
    Size {
        long_cm: f32,
        short_cm: f32,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Paper {
    A4,
    Letter,
}

#[derive(Clone, Copy, Debug)]
pub struct PrintOpts {
    pub layout: Layout,
    pub paper: Paper,
    /// Fill the cell (edges cut) instead of fitting the whole photo.
    pub fill: bool,
    pub captions: bool,
    pub margin_mm: f32,
}

/// Draws `text` into the page, centred in the box (x, y, w) with letters `px` pixels high.
pub type CaptionFn<'a> = &'a dyn Fn(&mut RgbImage, f32, f32, f32, &str, f32);

/// Photo sizes offered, as (long, short) centimetres.
pub const SIZES: [(f32, f32); 6] = [(13.0, 9.0), (15.0, 10.0), (18.0, 13.0), (20.0, 15.0), (24.0, 18.0), (30.0, 20.0)];

fn paper_mm(p: Paper) -> (f32, f32) {
    match p {
        Paper::A4 => (210.0, 297.0),
        Paper::Letter => (215.9, 279.4),
    }
}

struct Cell {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    /// A dashed cut line around it (exact sizes).
    guide: bool,
}

struct Page {
    landscape: bool,
    cells: Vec<(usize, Cell)>,
}

/// Lay out `sizes` (each photo's upright width and height) into pages; units are millimetres.
fn layout(sizes: &[(u32, u32)], o: &PrintOpts) -> Vec<Page> {
    let (pw, ph) = paper_mm(o.paper);
    let m = o.margin_mm.clamp(0.0, 40.0);
    let cap = if captions_on(o) { 7.0 } else { 0.0 };
    let gap = 5.0;
    let wide = |i: usize| sizes[i].0 >= sizes[i].1;
    let mut pages = Vec::new();
    match o.layout {
        Layout::One => {
            for i in 0..sizes.len() {
                let landscape = wide(i);
                let (w, h) = if landscape { (ph, pw) } else { (pw, ph) };
                pages.push(Page {
                    landscape,
                    cells: vec![(i, Cell { x: m, y: m, w: w - 2.0 * m, h: h - 2.0 * m - cap, guide: false })],
                });
            }
        }
        Layout::Two | Layout::Four | Layout::Grid { .. } => {
            let (cols, rows): (usize, usize) = match o.layout {
                Layout::Two => (1, 2),
                Layout::Grid { cols, rows } => (cols.clamp(1, 10) as usize, rows.clamp(1, 12) as usize),
                _ => (2, 2),
            };
            let per = cols * rows;
            let cw = (pw - 2.0 * m - gap * (cols as f32 - 1.0)) / cols as f32;
            let chh = (ph - 2.0 * m - gap * (rows as f32 - 1.0)) / rows as f32;
            for chunk in (0..sizes.len()).collect::<Vec<_>>().chunks(per) {
                let cells = chunk
                    .iter()
                    .enumerate()
                    .map(|(k, &i)| {
                        let (c, r) = (k % cols, k / cols);
                        (
                            i,
                            Cell {
                                x: m + c as f32 * (cw + gap),
                                y: m + r as f32 * (chh + gap),
                                w: cw,
                                h: chh - cap,
                                guide: false,
                            },
                        )
                    })
                    .collect();
                pages.push(Page { landscape: false, cells });
            }
        }
        Layout::Size { long_cm, short_cm } => {
            // Rows of exact-size cells; a photo never splits over two pages.
            let (lw, sw) = (long_cm * 10.0, short_cm * 10.0);
            let mut page = Page { landscape: false, cells: Vec::new() };
            let (mut x, mut y, mut row_h) = (m, m, 0.0f32);
            for i in 0..sizes.len() {
                let (w, h) = if wide(i) { (lw, sw) } else { (sw, lw) };
                if x + w > pw - m + 0.01 {
                    x = m;
                    y += row_h + gap + cap;
                    row_h = 0.0;
                }
                if y + h + cap > ph - m + 0.01 && !page.cells.is_empty() {
                    pages.push(std::mem::replace(&mut page, Page { landscape: false, cells: Vec::new() }));
                    x = m;
                    y = m;
                    row_h = 0.0;
                }
                page.cells.push((i, Cell { x, y, w, h, guide: true }));
                x += w + gap;
                row_h = row_h.max(h);
            }
            if !page.cells.is_empty() {
                pages.push(page);
            }
        }
    }
    pages
}

/// Contact sheets always print the file names (that is what they are for).
fn captions_on(o: &PrintOpts) -> bool {
    o.captions || matches!(o.layout, Layout::Grid { .. })
}

/// How many pages these photos make.
pub fn page_count(sizes: &[(u32, u32)], o: &PrintOpts) -> usize {
    layout(sizes, o).len()
}

/// Draw the pages at `dpi` (300 for printing, ~40 for a preview).
pub fn draw_pages(files: &[PathBuf], o: &PrintOpts, dpi: f32, caption: CaptionFn<'_>) -> Res<Vec<RgbImage>> {
    let photos: Vec<RgbImage> = files
        .iter()
        .map(|p| {
            let photo = crate::open_any(p)?;
            cs_image::preview_rgb8(&photo.image, 3600)
        })
        .collect::<Res<_>>()?;
    let sizes: Vec<(u32, u32)> = photos.iter().map(|p| p.dimensions()).collect();
    let mm = |v: f32| v / 25.4 * dpi;
    let (pw, ph) = paper_mm(o.paper);
    let mut out = Vec::new();
    for page in layout(&sizes, o) {
        let (w, h) = if page.landscape { (ph, pw) } else { (pw, ph) };
        let mut img = RgbImage::from_pixel(mm(w).round() as u32, mm(h).round() as u32, Rgb([255, 255, 255]));
        for (i, c) in &page.cells {
            let (cx, cy, cw, ch) = (mm(c.x), mm(c.y), mm(c.w), mm(c.h));
            place(&mut img, &photos[*i], cx, cy, cw, ch, o.fill);
            if c.guide {
                dashed_box(&mut img, cx, cy, cw, ch);
            }
            if captions_on(o) {
                let name = files[*i].file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                caption(&mut img, cx, cy + ch + mm(1.5), cw, &name, mm(3.5));
            }
        }
        out.push(img);
    }
    Ok(out)
}

/// The photo into the cell: scaled to fit (centred) or to fill (centre cut out).
fn place(page: &mut RgbImage, pic: &RgbImage, x: f32, y: f32, w: f32, h: f32, fill: bool) {
    let (pw, ph) = (pic.width() as f32, pic.height() as f32);
    let s = if fill { (w / pw).max(h / ph) } else { (w / pw).min(h / ph) };
    let (sw, sh) = ((pw * s).round().max(1.0) as u32, (ph * s).round().max(1.0) as u32);
    let scaled = imageops::resize(pic, sw, sh, imageops::FilterType::CatmullRom);
    let (cw, ch) = (w.round() as u32, h.round() as u32);
    let ox = (sw.saturating_sub(cw)) / 2;
    let oy = (sh.saturating_sub(ch)) / 2;
    let view = imageops::crop_imm(&scaled, ox, oy, sw.min(cw), sh.min(ch)).to_image();
    let dx = x.round() as i64 + i64::from((cw.saturating_sub(view.width())) / 2);
    let dy = y.round() as i64 + i64::from((ch.saturating_sub(view.height())) / 2);
    imageops::overlay(page, &view, dx, dy);
}

fn dashed_box(img: &mut RgbImage, x: f32, y: f32, w: f32, h: f32) {
    let grey = Rgb([150, 150, 150]);
    let (x0, y0, x1, y1) = (x.round() as i64 - 1, y.round() as i64 - 1, (x + w).round() as i64, (y + h).round() as i64);
    let mut put = |px: i64, py: i64| {
        if px >= 0 && py >= 0 && px < i64::from(img.width()) && py < i64::from(img.height()) {
            img.put_pixel(px as u32, py as u32, grey);
        }
    };
    for t in x0..=x1 {
        if (t / 12) % 2 == 0 {
            put(t, y0);
            put(t, y1);
        }
    }
    for t in y0..=y1 {
        if (t / 12) % 2 == 0 {
            put(x0, t);
            put(x1, t);
        }
    }
}

/// The pages as a PDF: one full-page JPEG per page, page size from the paper (turned for landscape pages).
pub fn pdf(pages: &[RgbImage], paper: Paper) -> Res<Vec<u8>> {
    let (pw, ph) = paper_mm(paper);
    let pt = |mm: f32| mm / 25.4 * 72.0;
    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let n = pages.len();
    // Objects: 1 catalog, 2 pages, then per page: page, contents, image (3 + 3k ..).
    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        let id = offsets.len();
        let _ = writeln!(out, "{id} 0 obj");
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    obj(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..n).map(|k| format!("{} 0 R", 3 + 3 * k)).collect();
    obj(&mut out, &mut offsets, format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).as_bytes());
    for (k, img) in pages.iter().enumerate() {
        let landscape = img.width() > img.height();
        let (w, h) = if landscape { (pt(ph), pt(pw)) } else { (pt(pw), pt(ph)) };
        let (page_id, content_id, image_id) = (3 + 3 * k, 4 + 3 * k, 5 + 3 * k);
        let _ = page_id;
        obj(
            &mut out,
            &mut offsets,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] /Resources << /XObject << /Im{k} {image_id} 0 R >> >> /Contents {content_id} 0 R >>"
            )
            .as_bytes(),
        );
        let draw = format!("q {w:.2} 0 0 {h:.2} 0 0 cm /Im{k} Do Q");
        obj(&mut out, &mut offsets, format!("<< /Length {} >>\nstream\n{draw}\nendstream", draw.len()).as_bytes());
        let mut jpg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 90).encode_image(img).map_err(err)?;
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            img.width(),
            img.height(),
            jpg.len()
        )
        .into_bytes();
        body.extend_from_slice(&jpg);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &mut offsets, &body);
    }
    let xref = out.len();
    let _ = write!(out, "xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1);
    for o in &offsets {
        let _ = writeln!(out, "{o:010} 00000 n ");
    }
    let _ = write!(out, "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1);
    Ok(out)
}

/// Show the Windows Print dialog and print the pages (owner = the app window, 0 if unknown). `Ok(false)` =
/// cancelled. Blocking.
pub fn print(pages: &[RgbImage], title: &str, owner: isize) -> Res<bool> {
    let px: Vec<cs_platform::print::PagePixels<'_>> = pages
        .iter()
        .map(|p| cs_platform::print::PagePixels { width: p.width(), height: p.height(), rgb: p.as_raw() })
        .collect();
    cs_platform::print::print_pages(&px, title, owner)
}

/// Save the PDF as a new file (never overwrites; Ctrl+Z removes it). Returns its path.
pub fn save_pdf(pdf: &[u8], dir: &Path, name: &str) -> Res<PathBuf> {
    let target = crate::fileops::free_name(dir, name);
    std::fs::write(&target, pdf).map_err(err)?;
    let m = std::fs::metadata(&target).map_err(err)?;
    crate::fileops::push(crate::fileops::Op {
        label: format!("Save “{}”", target.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()),
        undo: crate::fileops::Undo::Copied { files: vec![(target.clone(), m.len(), m.modified().ok())], dirs: vec![] },
    });
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(layout: Layout) -> PrintOpts {
        PrintOpts { layout, paper: Paper::A4, fill: false, captions: true, margin_mm: 10.0 }
    }

    #[test]
    fn layouts_make_the_right_number_of_pages() {
        let sizes = [(4000, 3000), (3000, 4000), (4000, 3000), (4000, 3000), (4000, 3000)];
        assert_eq!(page_count(&sizes, &opts(Layout::One)), 5);
        assert_eq!(page_count(&sizes, &opts(Layout::Two)), 3);
        assert_eq!(page_count(&sizes, &opts(Layout::Four)), 2);
        // Contact sheets: 12 and 24 per page.
        let many = vec![(4000, 3000); 30];
        assert_eq!(page_count(&many, &opts(Layout::Grid { cols: 3, rows: 4 })), 3);
        assert_eq!(page_count(&many, &opts(Layout::Grid { cols: 4, rows: 6 })), 2);
        // 15 × 10 cm on A4 (210 × 297, 10 mm margins): one per row, two rows per page.
        assert_eq!(page_count(&sizes, &opts(Layout::Size { long_cm: 15.0, short_cm: 10.0 })), 3);
        // One per page turns the page for a wide photo.
        let p = layout(&sizes, &opts(Layout::One));
        assert!(p[0].landscape && !p[1].landscape);
    }

    #[test]
    fn pdf_has_one_page_per_picture_and_a_valid_xref() {
        let pages =
            vec![RgbImage::from_pixel(80, 113, Rgb([200, 10, 10])), RgbImage::from_pixel(113, 80, Rgb([10, 10, 200]))];
        let pdf = pdf(&pages, Paper::A4).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Count 2"));
        assert!(text.contains("/MediaBox [0 0 595.28 841.89]") && text.contains("/MediaBox [0 0 841.89 595.28]"));
        // startxref points at the xref table.
        let sx: usize = text.rsplit("startxref\n").next().unwrap().lines().next().unwrap().parse().unwrap();
        assert!(pdf[sx..].starts_with(b"xref"));
    }
}
