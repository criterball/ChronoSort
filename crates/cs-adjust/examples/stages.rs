//! Time of each adjustment alone on a 1600 px picture.
//! cargo run --release -p cs-adjust --example stages -- <photo>
use cs_adjust::{Adjust, Look, render};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let photo = cs_image::open(std::path::Path::new(&a[0])).unwrap();
    let (w, h) = photo.image.dimensions();
    let s = 1600.0 / f64::from(w.max(h));
    let img = cs_image::apply(
        photo.image,
        cs_image::Edit::Resize { w: (f64::from(w) * s) as u32, h: (f64::from(h) * s) as u32 },
    )
    .unwrap();
    let d = Adjust::default;
    for (name, adj) in [
        ("exposure", Adjust { exposure: 0.5, ..d() }),
        ("tone (contrast)", Adjust { contrast: 30.0, ..d() }),
        ("local tone (shadows)", Adjust { shadows: 30.0, ..d() }),
        ("local + texture", Adjust { shadows: 30.0, texture: 30.0, ..d() }),
        ("dehaze", Adjust { dehaze: 30.0, ..d() }),
        ("curve", Adjust { curve_darks: 30.0, ..d() }),
        ("balance", Adjust { balance_midtones: [20.0, 0.0, 0.0], ..d() }),
        ("vibrance", Adjust { vibrance: 30.0, ..d() }),
        ("sharpen", Adjust { sharpen_amount: 50.0, ..d() }),
        ("nr luminance", Adjust { nr_luminance: 30.0, ..d() }),
        ("nr colour", Adjust { nr_color: 30.0, ..d() }),
        ("blur", Adjust { blur: 30.0, ..d() }),
        ("sepia", Adjust { look: Look::Sepia, ..d() }),
    ] {
        render(&img, &adj).unwrap();
        let t = std::time::Instant::now();
        for _ in 0..3 {
            render(&img, &adj).unwrap();
        }
        println!("{name:22} {:>6.1} ms", t.elapsed().as_secs_f64() * 1000.0 / 3.0);
    }
}
