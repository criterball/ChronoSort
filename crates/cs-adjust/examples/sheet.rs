//! Contact sheet: one photo through each adjustment, plus timings.
//! cargo run --release -p cs-adjust --example sheet -- <photo> <out.jpg>
use cs_adjust::{Adjust, Look, auto, render};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let photo = cs_image::open(std::path::Path::new(&a[0])).unwrap();
    let (w, h) = photo.image.dimensions();
    // Full-size timing for the heaviest combination.
    let heavy = Adjust {
        highlights: -60.0,
        shadows: 60.0,
        clarity: 40.0,
        texture: 30.0,
        dehaze: 30.0,
        contrast: 20.0,
        vibrance: 30.0,
        sharpen_amount: 60.0,
        nr_luminance: 30.0,
        nr_color: 30.0,
        ..Default::default()
    };
    let t = std::time::Instant::now();
    render(&photo.image, &heavy).unwrap();
    println!("full size {w}x{h}, everything on: {:.0?}", t.elapsed());
    // Work on a 400 px copy for the sheet.
    let small = cs_image::apply(
        photo.image.clone(),
        cs_image::Edit::Resize { w: (400 * w / w.max(h)).max(1), h: (400 * h / w.max(h)).max(1) },
    )
    .unwrap();
    let t = std::time::Instant::now();
    let pre = cs_image::apply(
        photo.image.clone(),
        cs_image::Edit::Resize { w: (1600 * w / w.max(h)).max(1), h: (1600 * h / w.max(h)).max(1) },
    )
    .unwrap();
    let t0 = std::time::Instant::now();
    render(&pre, &heavy).unwrap();
    println!("1600 px preview, everything on: {:.0?} (resize {:.0?})", t0.elapsed(), t0 - t);
    let au = auto(&small).unwrap();
    println!("auto: temp {} tint {} levels {}..{} gamma {}", au.temp, au.tint, au.in_black, au.in_white, au.gamma);
    let d = Adjust::default;
    let cases: Vec<(&str, Adjust)> = vec![
        ("original", d()),
        ("auto", au),
        ("exposure +1", Adjust { exposure: 1.0, ..d() }),
        ("contrast +60", Adjust { contrast: 60.0, ..d() }),
        ("highlights -100", Adjust { highlights: -100.0, ..d() }),
        ("shadows +100", Adjust { shadows: 100.0, ..d() }),
        ("whites +60 blacks -60", Adjust { whites: 60.0, blacks: -60.0, ..d() }),
        ("temp +50", Adjust { temp: 50.0, ..d() }),
        ("tint +50", Adjust { tint: 50.0, ..d() }),
        ("vibrance +80", Adjust { vibrance: 80.0, ..d() }),
        ("saturation -60", Adjust { saturation: -60.0, ..d() }),
        ("clarity +80", Adjust { clarity: 80.0, ..d() }),
        ("texture +80", Adjust { texture: 80.0, ..d() }),
        ("dehaze +60", Adjust { dehaze: 60.0, ..d() }),
        ("dehaze -60", Adjust { dehaze: -60.0, ..d() }),
        ("curve S", Adjust { curve_darks: -40.0, curve_lights: 40.0, ..d() }),
        ("levels", Adjust { in_black: 0.08, in_white: 0.9, gamma: 1.3, ..d() }),
        (
            "balance warm/teal",
            Adjust { balance_shadows: [-30.0, 0.0, 40.0], balance_highlights: [30.0, 0.0, -30.0], ..d() },
        ),
        ("sharpen 120", Adjust { sharpen_amount: 120.0, ..d() }),
        ("noise red. 80", Adjust { nr_luminance: 80.0, nr_color: 80.0, ..d() }),
        ("blur 40", Adjust { blur: 40.0, ..d() }),
        ("grayscale", Adjust { look: Look::Grayscale, ..d() }),
        ("sepia", Adjust { look: Look::Sepia, ..d() }),
        ("negative", Adjust { look: Look::Negative, ..d() }),
    ];
    let (tw, th) = small.dimensions();
    let cols = 4u32;
    let rows = (cases.len() as u32).div_ceil(cols);
    let mut sheet = image::RgbImage::from_pixel(cols * (tw + 8), rows * (th + 8), image::Rgb([40, 40, 40]));
    for (i, (name, adj)) in cases.iter().enumerate() {
        let out = render(&small, adj).unwrap();
        let rgb = cs_image::preview_rgb8(&out, 400).unwrap();
        let (cx, cy) = ((i as u32 % cols) * (tw + 8) + 4, (i as u32 / cols) * (th + 8) + 4);
        image::imageops::overlay(&mut sheet, &rgb, i64::from(cx), i64::from(cy));
        println!("{i:2}: {name}");
    }
    sheet.save(&a[1]).unwrap();
}
