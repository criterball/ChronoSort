//! Small knowledge base about capture devices.

use cs_meta::ExifInfo;

/// (model, first-sale year). Model matched case-insensitively, exact.
/// A photo dated before its camera existed has a wrong clock.
const RELEASES: &[(&str, i16)] = &[
    ("iphone", 2007),
    ("iphone 3g", 2008),
    ("iphone 3gs", 2009),
    ("iphone 4", 2010),
    ("iphone 4s", 2011),
    ("iphone 5", 2012),
    ("iphone 5c", 2013),
    ("iphone 5s", 2013),
    ("iphone 6", 2014),
    ("iphone 6 plus", 2014),
    ("iphone 6s", 2015),
    ("iphone 6s plus", 2015),
    ("iphone se", 2016),
    ("iphone 7", 2016),
    ("iphone 7 plus", 2016),
    ("iphone 8", 2017),
    ("iphone 8 plus", 2017),
    ("iphone x", 2017),
    ("iphone xr", 2018),
    ("iphone xs", 2018),
    ("iphone xs max", 2018),
    ("iphone 11", 2019),
    ("iphone 11 pro", 2019),
    ("iphone 11 pro max", 2019),
    ("iphone se (2nd generation)", 2020),
    ("iphone 12", 2020),
    ("iphone 12 mini", 2020),
    ("iphone 12 pro", 2020),
    ("iphone 12 pro max", 2020),
    ("iphone 13", 2021),
    ("iphone 13 mini", 2021),
    ("iphone 13 pro", 2021),
    ("iphone 13 pro max", 2021),
    ("iphone 14", 2022),
    ("iphone 14 plus", 2022),
    ("iphone 14 pro", 2022),
    ("iphone 14 pro max", 2022),
    ("iphone 15", 2023),
    ("iphone 15 plus", 2023),
    ("iphone 15 pro", 2023),
    ("iphone 15 pro max", 2023),
    ("iphone 16", 2024),
    ("iphone 16 plus", 2024),
    ("iphone 16 pro", 2024),
    ("iphone 16 pro max", 2024),
    ("iphone 16e", 2025),
    ("iphone 17", 2025),
    ("iphone 17 pro", 2025),
    ("iphone 17 pro max", 2025),
    ("iphone air", 2025),
    ("canon eos 300d digital", 2003),
    ("canon eos 350d digital", 2005),
    ("canon eos 5d", 2005),
    ("canon eos 5d mark ii", 2008),
    ("nikon d70", 2004),
    ("nikon d90", 2008),
    ("nikon d750", 2014),
];

pub fn release_year(e: &ExifInfo) -> Option<i16> {
    let m = e.model.as_deref()?.trim().to_lowercase();
    RELEASES.iter().find(|(name, _)| *name == m).map(|(_, y)| *y)
}

/// Flatbed and film scanners: their EXIF date is the scan date.
pub fn is_scanner(e: &ExifInfo) -> bool {
    let l = |s: &Option<String>| s.as_deref().unwrap_or("").to_lowercase();
    let (make, model, sw) = (l(&e.make), l(&e.model), l(&e.software));
    const MODEL: &[&str] = &["scan", "perfection", "coolscan", "opticfilm", "filmscan", "canoscan", "scanjet"];
    const MAKE: &[&str] =
        &["epson", "plustek", "microtek", "reflecta", "avision", "visioneer", "mustek", "pacific image"];
    const SOFTWARE: &[&str] = &["vuescan", "silverfast", "epson scan", "scan"];
    MODEL.iter().any(|k| model.contains(k))
        || MAKE.iter().any(|k| make.starts_with(k))
        || SOFTWARE.iter().any(|k| sw.contains(k))
}
