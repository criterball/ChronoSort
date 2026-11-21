//! Show what preview ChronoSort gets from a file: cargo run -p cs-thumb --example preview -- FILE [OUT.jpg]
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let p = std::path::Path::new(&a[0]);
    let head = std::fs::read(p).map(|b| b[..b.len().min(64)].to_vec()).unwrap_or_default();
    let fmt = cs_scan::sniff(&head, p.extension().and_then(|e| e.to_str()).unwrap_or(""));
    let (_, small) = cs_meta::orientation_and_thumbnail(p, fmt);
    let big = cs_meta::raw_preview(p);
    println!(
        "format {fmt:?}; EXIF thumbnail {} bytes; largest embedded preview {} bytes",
        small.map_or(0, |s| s.len()),
        big.map_or(0, |b| b.len())
    );
    let t = std::time::Instant::now();
    match cs_thumb::load(p, fmt, 1280) {
        Ok(img) => {
            println!("preview {}x{} in {:.0?}", img.width(), img.height(), t.elapsed());
            if let Some(out) = a.get(1) {
                img.to_rgb8().save(out).unwrap();
            }
        }
        Err(e) => println!("no preview: {e}"),
    }
}
