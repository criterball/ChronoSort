//! cargo run --release -p cs-video --example poster -- <out dir> <videos…>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let out = std::path::Path::new(&a[0]);
    for v in &a[1..] {
        let t = std::time::Instant::now();
        let p = std::path::Path::new(v);
        let size = std::env::var("CS_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(640);
        match cs_video::poster(p, size) {
            Ok(img) => {
                let name = out.join(format!("{}.png", p.file_name().unwrap().to_string_lossy()));
                img.save(&name).unwrap();
                println!("{v}: {}x{} in {:.0?}", img.width(), img.height(), t.elapsed());
            }
            Err(e) => println!("{v}: FAILED {e} ({:.0?})", t.elapsed()),
        }
    }
}
