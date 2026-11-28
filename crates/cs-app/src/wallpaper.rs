//! "Set as desktop wallpaper": the photo (upright, as JPEG) is copied into
//! ChronoSort's data folder and handed to Windows, so the wallpaper stays when
//! the photo is moved or renamed. The previous wallpaper is remembered:
//! Ctrl+Z in the browser puts it back.

use crate::fileops::{self, Op, Undo};
use crate::{Res, err};
use std::path::{Path, PathBuf};

/// Longest side of the wallpaper copy (8K screens).
const MAX_SIDE: u32 = 7680;

/// Make `src` the desktop wallpaper (`style`: fill, fit, stretch, center, tile, span). Blocking.
/// Returns the photo's name. The copy lives in `<data>/wallpaper/`.
pub fn set_wallpaper(src: &Path, style: &str, data: &Path) -> Res<String> {
    let style = cs_platform::WallStyle::parse(style).ok_or("Unknown wallpaper style.")?;
    let dir = data.join("wallpaper");
    {
        let photo = crate::open_any(src)?;
        let rgb = cs_image::preview_rgb8(&photo.image, MAX_SIDE)?;
        std::fs::create_dir_all(&dir).map_err(err)?;
        let file = fileops::free_name(&dir, &format!("wallpaper-{}.jpg", crate::now_unix()));
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95).encode_image(&rgb).map_err(err)?;
        std::fs::write(&file, &bytes).map_err(err)?;
        let previous = cs_platform::current_wallpaper().ok();
        cs_platform::set_wallpaper(&file, style).map_err(|e| format!("Windows did not accept the wallpaper: {e}"))?;
        tidy(&dir, &file, previous.as_ref().map(|p| p.0.as_path()));
        if let Some((prev, prev_style)) = previous {
            fileops::push(Op {
                label: "Set the desktop wallpaper".into(),
                undo: Undo::Wallpaper { file: prev, style: prev_style.name().to_string() },
            });
        }
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Ok(name)
    }
}

/// Keep only the new wallpaper copy and the one it replaced (Undo may need it).
fn tidy(dir: &Path, keep: &Path, previous: Option<&Path>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let same = |a: &Path, b: &Path| a.file_name() == b.file_name();
    let olds: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("wallpaper-")))
        .filter(|p| !same(p, keep) && previous.is_none_or(|q| !(same(p, q) && q.starts_with(dir))))
        .collect();
    for p in olds {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_keeps_the_new_and_the_previous_copy() {
        let d = tempfile::tempdir().unwrap();
        for n in ["wallpaper-1.jpg", "wallpaper-2.jpg", "wallpaper-3.jpg", "other.txt"] {
            std::fs::write(d.path().join(n), b"x").unwrap();
        }
        tidy(d.path(), &d.path().join("wallpaper-3.jpg"), Some(&d.path().join("wallpaper-2.jpg")));
        let mut left: Vec<String> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["other.txt", "wallpaper-2.jpg", "wallpaper-3.jpg"]);
        // A previous wallpaper elsewhere (Windows' own) does not protect old copies.
        tidy(d.path(), &d.path().join("wallpaper-3.jpg"), Some(Path::new("C:/Windows/Web/img0.jpg")));
        assert!(!d.path().join("wallpaper-2.jpg").exists());
    }
}
