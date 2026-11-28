//! ExifTool comes with ChronoSort: `exiftool\exiftool.exe` in the ChronoSort folder (no network, nothing installed).

use crate::{Res, err};
use std::path::{Path, PathBuf};

pub fn path() -> Res<PathBuf> {
    let name = if cfg!(windows) { "exiftool.exe" } else { "exiftool" };
    let exe = std::env::current_exe().map_err(err)?;
    let dir = exe.parent().ok_or("no program folder")?;
    let p = dir.join("exiftool").join(name);
    if p.is_file() {
        return Ok(p);
    }
    // Development builds and tests (target\debug, target\…\deps): the project's own copy.
    if cfg!(debug_assertions) {
        let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tools/exiftool").join(name);
        if dev.is_file() {
            return Ok(dev);
        }
    }
    Err("ExifTool is missing from ChronoSort's folder (exiftool\\exiftool.exe). Please copy ChronoSort again.".into())
}

/// Run ExifTool once with these arguments (a short-lived process; `-stay_open` lives in cs-geo).
pub fn run(args: &[String]) -> Res<Vec<String>> {
    let mut et = cs_geo::exiftool::ExifTool::start(&path()?).map_err(err)?;
    et.run(args).map_err(err)
}
