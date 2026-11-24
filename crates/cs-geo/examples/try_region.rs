//! Real ExifTool run on COPIES: town first, then replaced by a general location,
//! then both undone. cargo run -p cs-geo --example try_region -- <exiftool.exe> <work folder> <town> <region query> [more region queries to list]
use std::path::Path;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (et, folder) = (Path::new(&a[0]), Path::new(&a[1]));
    let out = std::process::Command::new(et).args(["-api", "geolocation", "-listgeo"]).output().unwrap();
    let places = cs_geo::Places::new(cs_geo::parse_listgeo(&String::from_utf8_lossy(&out.stdout)));
    for q in &a[3..] {
        let t = std::time::Instant::now();
        let r = places.regions(q, 5);
        let names: Vec<String> = r.iter().map(|r| format!("{} / {} ({})", r.name, r.country, r.population)).collect();
        println!("regions {q:?} in {:.1?}: {names:?}", t.elapsed());
    }
    let jobs: Vec<cs_geo::Job> = std::fs::read_dir(folder)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "jpg"))
        .map(|e| cs_geo::Job { rel: e.file_name().to_string_lossy().into(), video: false })
        .collect();
    let before: Vec<String> = jobs.iter().map(|j| cs_core::hash_file(&folder.join(&j.rel)).unwrap()).collect();
    let read = |label: &str| {
        let o = std::process::Command::new(et)
            .args(["-n", "-s", "-GPSLatitude", "-GPSLongitude", "-XMP-photoshop:City", "-XMP-photoshop:State"])
            .args(["-XMP-photoshop:Country", "-XMP-iptcCore:CountryCode"])
            .arg(folder.join(&jobs[0].rel))
            .output()
            .unwrap();
        println!("{label}: {}", String::from_utf8_lossy(&o.stdout).replace("\r\n", " | "));
    };
    let none = Default::default();
    let town = cs_geo::Target::City { place: places.search(&a[2], 1)[0].clone() };
    let r = cs_geo::apply(et, folder, &jobs, &town, "run-1", &|_, _| {}, &none).unwrap();
    println!("town: {} changed, failed {:?}", r.changed, r.failed);
    read("after town");
    let g = places.regions(&a[3], 1).remove(0);
    let region = cs_geo::Target::Region { name: g.name, country: g.country, code: g.code };
    let r = cs_geo::apply(et, folder, &jobs, &region, "run-2", &|_, _| {}, &none).unwrap();
    println!("region {:?}: {} changed, failed {:?}", region.label(), r.changed, r.failed);
    read("after region");
    let c = cs_geo::Target::Country { name: "Italy".into(), code: "IT".into() };
    let r = cs_geo::apply(et, folder, &jobs, &c, "run-3", &|_, _| {}, &none).unwrap();
    println!("country: {} changed, failed {:?}", r.changed, r.failed);
    read("after country only");
    for run in ["run-3", "run-2", "run-1"] {
        let u = cs_geo::undo(folder, run).unwrap();
        println!("undo {run}: restored {}, kept {:?}, failed {:?}", u.restored, u.kept, u.failed);
    }
    let same = jobs.iter().zip(&before).all(|(j, h)| &cs_core::hash_file(&folder.join(&j.rel)).unwrap() == h);
    println!("all files identical to before: {same}");
}
