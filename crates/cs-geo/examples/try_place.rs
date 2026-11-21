//! Real ExifTool run on COPIES: cargo run -p cs-geo --example try_place -- <exiftool.exe> <work folder with photos> <place query>
use std::path::Path;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (et, folder) = (Path::new(&a[0]), Path::new(&a[1]));
    let t = std::time::Instant::now();
    let out = std::process::Command::new(et).args(["-api", "geolocation", "-listgeo"]).output().unwrap();
    let places = cs_geo::Places::new(cs_geo::parse_listgeo(&String::from_utf8_lossy(&out.stdout)));
    println!("{} places loaded in {:.1?}", places.len(), t.elapsed());
    for p in places.search(&a[2], 3) {
        println!("  match: {} ({}, pop {})", p.label(), p.time_zone, p.population);
    }
    let place = places.search(&a[2], 1)[0].clone();
    let jobs: Vec<cs_geo::Job> = std::fs::read_dir(folder)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jpg"))
        .map(|e| cs_geo::Job { rel: e.file_name().to_string_lossy().into(), video: false })
        .collect();
    let before: Vec<String> = jobs.iter().map(|j| cs_core::hash_file(&folder.join(&j.rel)).unwrap()).collect();
    let t = std::time::Instant::now();
    let rep =
        cs_geo::apply(et, folder, &jobs, &cs_geo::Target::City { place }, "test-run", &|_, _| {}, &Default::default())
            .unwrap();
    println!("apply: {} changed, failed {:?} in {:.1?}", rep.changed, rep.failed, t.elapsed());
    let check = std::process::Command::new(et)
        .args(["-n", "-GPSLatitude", "-GPSLongitude", "-XMP-photoshop:City", "-XMP-photoshop:Country", "-s3"])
        .arg(folder.join(&jobs[0].rel))
        .output()
        .unwrap();
    println!("read back: {}", String::from_utf8_lossy(&check.stdout).replace(['\r', '\n'], " | "));
    let u = cs_geo::undo(folder, "test-run").unwrap();
    let same = jobs.iter().zip(&before).all(|(j, h)| &cs_core::hash_file(&folder.join(&j.rel)).unwrap() == h);
    println!(
        "undo: restored {}, kept {:?}, failed {:?}; all files identical to before: {same}",
        u.restored, u.kept, u.failed
    );
}
