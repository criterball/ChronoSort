//! Remembered choices (slideshow options, print layout …) as small JSON files in `ChronoSortData\settings` —
//! inside the ChronoSort folder like everything else, never in AppData or browser storage. A missing or
//! damaged file just means the defaults.

use serde::{Serialize, de::DeserializeOwned};
use std::path::Path;

pub fn load<T: DeserializeOwned + Default>(data_root: &Path, name: &str) -> T {
    std::fs::read(data_root.join("settings").join(format!("{name}.json")))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Write the settings (a temporary file first, so a crash never leaves half a file).
pub fn save<T: Serialize>(data_root: &Path, name: &str, value: &T) {
    let dir = data_root.join("settings");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec_pretty(value) else { return };
    let tmp = dir.join(format!("{name}.json.tmp"));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(format!("{name}.json")));
    }
}
