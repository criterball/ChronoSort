//! Picture edits in the viewer: rotate / flip, crop, resize, Save As.
//!
//! The picture is edited in memory (photocraft codecs via `cs-image`); the file
//! on disk is only touched by [`edit_save`]:
//! * "copy" (default): a new file next to the original (never overwrites).
//! * "replace": the edited picture takes the original's place and the original
//!   is kept in `<folder>/.chronosort/edit-originals/<run>/` so Undo can put it back
//!   (works the same on every PC, also where the Recycle Bin is switched off).
//! * A JPEG that was only turned or flipped is saved **losslessly**: the pixels
//!   are copied unchanged and only the EXIF orientation tag is changed.
//!   The saved file keeps the original's file dates, so sorting by date still works.

use crate::fileops::{self, Op, Undo};
use crate::{AppState, Res, err};
use cs_image::{Edit, Format, Photo};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

struct Session {
    path: PathBuf,
    original: Photo,
    edits: Vec<Edit>,
    current: cs_image::Image,
    version: u64,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

#[derive(Serialize)]
pub struct EditState {
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub edits: usize,
    /// Bumps with every change; the preview URL includes it.
    pub version: u64,
    /// True when saving as JPEG would be lossless (turns/flips of a JPEG only).
    pub lossless: bool,
}

fn state_of(s: &Session) -> EditState {
    let (width, height) = s.current.dimensions();
    EditState {
        width,
        height,
        format: format!("{:?}", s.original.format),
        edits: s.edits.len(),
        version: s.version,
        lossless: s.original.format == Format::Jpeg
            && cs_image::orientation_after(s.original.orientation, &s.edits).is_some(),
    }
}

fn lock() -> std::sync::MutexGuard<'static, Option<Session>> {
    SESSION.lock().unwrap_or_else(|e| e.into_inner())
}

#[tauri::command]
pub async fn edit_open(state: State<'_, AppState>, id: u64) -> Res<EditState> {
    let path = state.browse.path(id).ok_or("This photo is no longer listed; press F5.")?;
    let p2 = path.clone();
    let original = tauri::async_runtime::spawn_blocking(move || cs_image::open(&p2)).await.map_err(err)??;
    let s = Session {
        current: original.image.clone(),
        path,
        original,
        edits: Vec::new(),
        version: crate::now_unix() as u64,
    };
    let st = state_of(&s);
    *lock() = Some(s);
    Ok(st)
}

#[tauri::command]
pub async fn edit_apply(edit: Edit) -> Res<EditState> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut g = lock();
        let s = g.as_mut().ok_or("No photo is open for editing.")?;
        let img = std::mem::replace(
            &mut s.current,
            cs_image::Image::from_raw(0, 0, cs_image::ChannelLayout::Rgb, cs_image::SampleType::U8, vec![])
                .map_err(err)?,
        );
        match cs_image::apply(img.clone(), edit) {
            Ok(n) => s.current = n,
            Err(e) => {
                s.current = img;
                return Err(e);
            }
        }
        s.edits.push(edit);
        s.version += 1;
        Ok(state_of(s))
    })
    .await
    .map_err(err)?
}

/// Take back the last edit step (recomputed from the original).
#[tauri::command]
pub async fn edit_step_back() -> Res<EditState> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut g = lock();
        let s = g.as_mut().ok_or("No photo is open for editing.")?;
        s.edits.pop();
        let mut img = s.original.image.clone();
        for e in &s.edits {
            img = cs_image::apply(img, *e)?;
        }
        s.current = img;
        s.version += 1;
        Ok(state_of(s))
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn edit_close() {
    *lock() = None;
}

/// `edit://<version>`: the current edited picture as JPEG (longest side 1600 px).
pub fn edit_response() -> tauri::http::Response<Vec<u8>> {
    let r = (|| -> Res<Vec<u8>> {
        let g = lock();
        let s = g.as_ref().ok_or("nothing open")?;
        let rgb = cs_image::preview_rgb8(&s.current, 1600)?;
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&rgb).map_err(err)?;
        Ok(out)
    })();
    let b = tauri::http::Response::builder().header("Cache-Control", "no-store");
    match r {
        Ok(bytes) => b.header("Content-Type", "image/jpeg").status(200).body(bytes),
        Err(e) => b.header("Content-Type", "text/plain").status(404).body(e.into_bytes()),
    }
    .expect("static response")
}

#[derive(Serialize)]
pub struct Saved {
    pub path: String,
    pub lossless: bool,
    pub warnings: Vec<String>,
}

fn format_of(name: &str) -> Res<(Format, &'static str)> {
    cs_image::save_formats()
        .into_iter()
        .find(|(_, n, _)| n.eq_ignore_ascii_case(name))
        .map(|(f, _, e)| (f, e))
        .ok_or_else(|| format!("Unknown format {name}"))
}

/// Write the edit. `mode`: "copy" | "replace". `format`: "JPEG" | "PNG" | "TIFF" | "WebP".
#[tauri::command]
pub async fn edit_save(mode: String, format: String, quality: u8, name: Option<String>) -> Res<Saved> {
    let (fmt, ext) = format_of(&format)?;
    tauri::async_runtime::spawn_blocking(move || {
        let g = lock();
        let s = g.as_ref().ok_or("No photo is open for editing.")?;
        let src = s.path.clone();
        fileops::guard(&src)?;
        let dir = src.parent().ok_or("no folder")?.to_path_buf();
        let stem = src.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "photo".into());
        let times = cs_platform::read_times(&src).ok();
        let lossless_tag = (fmt == Format::Jpeg && s.original.format == Format::Jpeg)
            .then(|| cs_image::orientation_after(s.original.orientation, &s.edits))
            .flatten();

        // 1. Write the new picture to a temporary name in the same folder.
        let tmp = fileops::free_name(&dir, &format!(".{stem}.chronosort-edit.{ext}"));
        let mut warnings = Vec::new();
        let written = (|| -> Res<()> {
            if let Some(tag) = lossless_tag {
                std::fs::copy(&src, &tmp).map_err(err)?;
                let mut et = cs_geo::exiftool::ExifTool::start(&crate::places::exiftool()?).map_err(err)?;
                let out = et
                    .run(
                        &[
                            "-charset",
                            "filename=utf8",
                            "-overwrite_original",
                            "-n",
                            &format!("-Orientation={tag}"),
                            &tmp.to_string_lossy(),
                        ]
                        .map(String::from),
                    )
                    .map_err(err)?;
                if !out.iter().any(|l| l.contains("1 image files updated")) {
                    return Err(format!("Could not change the orientation tag: {}", out.join(" ")));
                }
            } else {
                warnings = cs_image::warnings(&s.current, fmt);
                cs_image::save(&s.current, fmt, cs_image::SaveOptions { quality }, &tmp)?;
            }
            if let (Some(t), Ok(f)) = (times, std::fs::File::options().write(true).open(&tmp)) {
                let _ = cs_platform::apply_times(&f, &t);
            }
            Ok(())
        })();
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }

        // 2. Put it in place.
        let final_path = if mode == "replace" {
            let run = format!("{}", crate::now_unix());
            let keep = dir.join(".chronosort").join("edit-originals").join(&run);
            std::fs::create_dir_all(&keep).map_err(err)?;
            let backup = keep.join(src.file_name().ok_or("no name")?);
            cs_platform::rename_no_replace(&src, &backup).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                format!("Could not set the original aside: {e}")
            })?;
            let same_ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()) == Some(ext.to_string())
                || (ext == "jpg" && src.extension().is_some_and(|e| e.eq_ignore_ascii_case("jpeg")));
            let target = if same_ext { src.clone() } else { fileops::free_name(&dir, &format!("{stem}.{ext}")) };
            if let Err(e) = cs_platform::rename_no_replace(&tmp, &target) {
                let _ = cs_platform::rename_no_replace(&backup, &src); // put the original back
                let _ = std::fs::remove_file(&tmp);
                return Err(e.to_string());
            }
            fileops::push(Op {
                label: format!(
                    "Replace “{}” with the edited picture",
                    src.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
                ),
                undo: Undo::Replaced { edited: target.clone(), original: src.clone(), backup },
            });
            target
        } else {
            let want =
                name.map(|n| fileops::check_name(&n)).transpose()?.unwrap_or_else(|| format!("{stem}_edited.{ext}"));
            let target = fileops::free_name(&dir, &want);
            cs_platform::rename_no_replace(&tmp, &target).map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                e.to_string()
            })?;
            let m = std::fs::metadata(&target).map_err(err)?;
            fileops::push(Op {
                label: format!(
                    "Save edited copy “{}”",
                    target.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
                ),
                undo: Undo::Copied { files: vec![(target.clone(), m.len(), m.modified().ok())], dirs: vec![] },
            });
            target
        };
        Ok(Saved { path: final_path.to_string_lossy().into_owned(), lossless: lossless_tag.is_some(), warnings })
    })
    .await
    .map_err(err)?
}

/// The formats offered in "Save as".
#[tauri::command]
pub fn edit_formats() -> Vec<&'static str> {
    cs_image::save_formats().iter().map(|f| f.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_session(p: &std::path::Path) {
        let original = cs_image::open(p).unwrap();
        *lock() = Some(Session {
            current: original.image.clone(),
            path: p.to_path_buf(),
            original,
            edits: vec![],
            version: 1,
        });
    }
    fn apply_now(e: Edit) {
        let mut g = lock();
        let s = g.as_mut().unwrap();
        s.current = cs_image::apply(s.current.clone(), e).unwrap();
        s.edits.push(e);
    }

    /// One test (the edit session is global): lossless turn as a copy, then
    /// crop + replace the original, then Undo brings the original back.
    #[test]
    fn save_copy_lossless_then_replace_and_undo() {
        let _l = crate::fileops::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if crate::places::exiftool().is_err() {
            eprintln!("skip: no ExifTool");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("pic.jpg");
        image::RgbImage::from_fn(40, 30, |x, y| image::Rgb([(x * 6) as u8, (y * 8) as u8, 90])).save(&f).unwrap();
        let original_bytes = std::fs::read(&f).unwrap();

        // 1. Turn right, save as copy: lossless (same pixels, Orientation tag 6).
        open_session(&f);
        apply_now(Edit::RotateRight);
        let r = tauri::async_runtime::block_on(edit_save("copy".into(), "JPEG".into(), 92, None)).unwrap();
        assert!(r.lossless);
        let copy = std::path::PathBuf::from(&r.path);
        assert_eq!(copy.file_name().unwrap(), "pic_edited.jpg");
        let info = crate::meta::read(&copy).unwrap();
        assert!(info.tags.iter().any(|t| t.name == "Orientation" && t.value.contains("90")), "orientation tag set");
        // Shown upright it is now 30 × 40.
        assert_eq!(cs_image::open(&copy).unwrap().image.dimensions(), (30, 40));
        assert_eq!(std::fs::read(&f).unwrap(), original_bytes, "original untouched");

        // 2. Crop and replace the original.
        open_session(&f);
        apply_now(Edit::Crop { x: 0, y: 0, w: 20, h: 10 });
        let r = tauri::async_runtime::block_on(edit_save("replace".into(), "JPEG".into(), 90, None)).unwrap();
        assert!(!r.lossless);
        assert_eq!(cs_image::open(&f).unwrap().image.dimensions(), (20, 10));
        // 3. Undo: original back, byte for byte.
        let op = crate::fileops::take_last_for_test();
        crate::fileops::undo_for_test(op).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), original_bytes);
        *lock() = None;
    }
}
