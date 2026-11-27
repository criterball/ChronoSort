//! Check the lossless crop on real files: crop the middle half of each JPEG and
//! compare the decoded pixels with the same area of the decoded original.
//! `cargo run -p cs-image --example lossless_crop -- a.jpg b.jpg …`
fn main() {
    for f in std::env::args().skip(1) {
        let b = std::fs::read(&f).unwrap();
        let name = std::path::Path::new(&f).file_name().unwrap().to_string_lossy().into_owned();
        let (w, h, mw, mh) = match cs_image::jpegcrop::grid(&b) {
            Ok(g) => g,
            Err(e) => {
                println!("{name}: {e}");
                continue;
            }
        };
        let t = std::time::Instant::now();
        let (out, (x0, y0, cw, ch)) = match cs_image::jpegcrop::crop(&b, w / 4 + 5, h / 4 + 3, w / 2, h / 2) {
            Ok(r) => r,
            Err(e) => {
                println!("{name}: {e}");
                continue;
            }
        };
        let ms = t.elapsed().as_millis();
        let whole = image::load_from_memory(&b).unwrap().to_rgb8();
        let part = image::load_from_memory(&out).unwrap().to_rgb8();
        let mut diff = 0;
        for y in 2..ch - 2 {
            for x in 2..cw - 2 {
                let a = part.get_pixel(x, y).0;
                let o = whole.get_pixel(x0 + x, y0 + y).0;
                for c in 0..3 {
                    diff = diff.max((i32::from(a[c]) - i32::from(o[c])).abs());
                }
            }
        }
        println!(
            "{name}: {w}x{h} grid {mw}x{mh} -> crop {cw}x{ch} at {x0},{y0}; {} KB -> {} KB; {ms} ms; max pixel difference {diff}",
            b.len() / 1024,
            out.len() / 1024
        );
    }
}
