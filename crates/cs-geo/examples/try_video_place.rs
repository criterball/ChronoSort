//! Real ExifTool run on COPIES of videos: a town, then a general area, then undo.
//! cargo run -p cs-geo --example try_video_place -- <exiftool.exe> <work folder with .mp4/.mov> <town>
use std::path::Path;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (et, folder) = (Path::new(&a[0]), Path::new(&a[1]));
    let out = std::process::Command::new(et).args(["-api", "geolocation", "-listgeo"]).output().unwrap();
    let places = cs_geo::Places::new(cs_geo::parse_listgeo(&String::from_utf8_lossy(&out.stdout)));
    let jobs: Vec<cs_geo::Job> = std::fs::read_dir(folder)
        .unwrap()
        .flatten()
        .filter(|e| {
            e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("mp4") || x.eq_ignore_ascii_case("mov"))
        })
        .map(|e| cs_geo::Job { rel: e.file_name().to_string_lossy().into(), video: true })
        .collect();
    let before: Vec<String> = jobs.iter().map(|j| cs_core::hash_file(&folder.join(&j.rel)).unwrap()).collect();
    let read = |label: &str| {
        for j in &jobs {
            let o = std::process::Command::new(et)
                .args(["-n", "-s3", "-f", "-GPSCoordinates", "-XMP-photoshop:City", "-XMP-photoshop:State"])
                .args(["-XMP-photoshop:Country", "-Duration"])
                .arg(folder.join(&j.rel))
                .output()
                .unwrap();
            println!("{label} {}: {}", j.rel, String::from_utf8_lossy(&o.stdout).trim().replace("\r\n", " | "));
        }
    };
    let none = Default::default();
    let town = cs_geo::Target::City { place: places.search(&a[2], 1)[0].clone() };
    let r = cs_geo::apply(et, folder, &jobs, &town, "v-1", &|_, _| {}, &none).unwrap();
    println!("town {:?}: {} changed, failed {:?}", town.label(), r.changed, r.failed);
    read("after town");
    let g = places.regions("tuscany", 1).remove(0);
    let region = cs_geo::Target::Region { name: g.name, country: g.country, code: g.code };
    let r = cs_geo::apply(et, folder, &jobs, &region, "v-2", &|_, _| {}, &none).unwrap();
    println!("area: {} changed, failed {:?}", r.changed, r.failed);
    read("after area");
    println!("(check the tagged copies still play/decode now, then press Enter to undo)");
    let mut s = String::new();
    if std::env::var("CS_NO_WAIT").is_err() {
        std::io::stdin().read_line(&mut s).unwrap();
    }
    for run in ["v-2", "v-1"] {
        let u = cs_geo::undo(folder, run).unwrap();
        println!("undo {run}: restored {}, kept {:?}, failed {:?}", u.restored, u.kept, u.failed);
    }
    let same = jobs.iter().zip(&before).all(|(j, h)| &cs_core::hash_file(&folder.join(&j.rel)).unwrap() == h);
    println!("all {} videos identical to before: {same}", jobs.len());
}
