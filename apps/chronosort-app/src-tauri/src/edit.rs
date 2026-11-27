//! Picture edits in the viewer: rotate / flip, crop, resize, straighten,
//! canvas, retouching (red eye, heal, clone, drawing), colour and light
//! adjustments (`cs-adjust`), print size, Save As.
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
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

/// One step of the edit history. Retouching keeps its result, so Undo can
/// replay the history without redoing slow repairs.
#[derive(Clone)]
enum Step {
    Edit(Edit),
    Pixels(std::sync::Arc<cs_image::Image>),
}

struct Session {
    path: PathBuf,
    original: Photo,
    steps: Vec<Step>,
    current: cs_image::Image,
    version: u64,
    /// Colour and light settings, applied after the steps above (non-destructive).
    adjust: cs_adjust::Adjust,
    /// `current` shrunk for the preview, kept while only adjustments change.
    preview_base: Option<cs_image::Image>,
    /// New print resolution (dots per inch), written on save.
    dpi: Option<u32>,
    /// Frame and watermark, applied after the adjustments (non-destructive).
    effects: cs_adjust::effects::Effects,
    /// The watermark picture and what it is ("text" or the logo's file name).
    layer: Option<(std::sync::Arc<cs_adjust::effects::Layer>, String)>,
}

impl Session {
    fn has_effects(&self) -> bool {
        !self.effects.is_none(self.layer.is_some())
    }
    fn layer(&self) -> Option<&cs_adjust::effects::Layer> {
        self.layer.as_ref().map(|(l, _)| l.as_ref())
    }
    /// The history as plain edits, if it has nothing else (for lossless saving).
    fn plain_edits(&self) -> Option<Vec<Edit>> {
        self.steps.iter().map(|s| if let Step::Edit(e) = s { Some(*e) } else { None }).collect()
    }
}

/// Longest side of the edit preview.
const PREVIEW_SIDE: u32 = 1600;

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

#[derive(Serialize)]
pub struct EditState {
    /// The picture being edited (crop, retouch etc. work in these pixels).
    pub width: u32,
    pub height: u32,
    /// The finished picture's size: bigger when a frame adds a border.
    pub out_width: u32,
    pub out_height: u32,
    pub format: String,
    pub edits: usize,
    /// Bumps with every change; the preview URL includes it.
    pub version: u64,
    /// True when saving as JPEG would be lossless (turns/flips of a JPEG only).
    pub lossless: bool,
    /// Colour / light adjustments are set.
    pub adjusted: bool,
    pub adjust: cs_adjust::Adjust,
    /// The photo's own print resolution (from the file), and the new one if set.
    pub dpi_original: Option<f32>,
    pub dpi: Option<u32>,
    /// Frame / watermark settings, whether they change anything, and the watermark's source.
    pub effects: cs_adjust::effects::Effects,
    pub has_effects: bool,
    pub watermark: Option<String>,
}

fn state_of(s: &Session) -> EditState {
    let (width, height) = s.current.dimensions();
    let (out_width, out_height) =
        if s.has_effects() { cs_adjust::effects::output_size(width, height, &s.effects) } else { (width, height) };
    EditState {
        out_width,
        out_height,
        width,
        height,
        format: format!("{:?}", s.original.format),
        edits: s.steps.len(),
        version: s.version,
        lossless: s.original.format == Format::Jpeg
            && s.adjust.is_neutral()
            && !s.has_effects()
            && s.plain_edits().is_some_and(|e| cs_image::orientation_after(s.original.orientation, &e).is_some()),
        adjusted: !s.adjust.is_neutral(),
        adjust: s.adjust.clone(),
        dpi_original: s.original.image.meta.dpi.map(|d| d.0).filter(|d| d.is_finite() && *d > 0.0),
        dpi: s.dpi,
        effects: s.effects,
        has_effects: s.has_effects(),
        watermark: s.layer.as_ref().map(|(_, n)| n.clone()),
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
        steps: Vec::new(),
        version: crate::now_unix() as u64,
        adjust: cs_adjust::Adjust::default(),
        preview_base: None,
        dpi: None,
        effects: Default::default(),
        layer: None,
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
        s.steps.push(Step::Edit(edit));
        s.preview_base = None;
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
        s.steps.pop();
        let mut img = s.original.image.clone();
        for st in &s.steps {
            img = match st {
                Step::Edit(e) => cs_image::apply(img, *e)?,
                Step::Pixels(p) => (**p).clone(),
            };
        }
        s.current = img;
        s.preview_base = None;
        s.version += 1;
        Ok(state_of(s))
    })
    .await
    .map_err(err)?
}

/// A retouching step (coordinates in pixels of the picture as it is now).
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Retouch {
    RedEye {
        eyes: Vec<cs_adjust::retouch::Eye>,
    },
    Heal {
        dabs: Vec<cs_adjust::retouch::Dab>,
    },
    Clone {
        dabs: Vec<cs_adjust::retouch::Dab>,
        dx: f64,
        dy: f64,
        hardness: f64,
    },
    /// A drawn layer (lines, shapes, text): PNG, the picture's size, base64.
    Draw {
        png: String,
    },
}

#[tauri::command]
pub async fn edit_retouch(op: Retouch) -> Res<EditState> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut g = lock();
        let s = g.as_mut().ok_or("No photo is open for editing.")?;
        let out = match op {
            Retouch::RedEye { eyes } => cs_adjust::retouch::red_eye(&s.current, &eyes)?,
            Retouch::Heal { dabs } => cs_adjust::retouch::heal(&s.current, &dabs)?,
            Retouch::Clone { dabs, dx, dy, hardness } => {
                cs_adjust::retouch::clone_stamp(&s.current, &dabs, dx, dy, hardness)?
            }
            Retouch::Draw { png } => {
                use base64::Engine;
                let b64 = png.split_once(',').map_or(png.as_str(), |(_, b)| b);
                let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(err)?;
                cs_adjust::retouch::overlay(&s.current, &bytes)?
            }
        };
        let out = std::sync::Arc::new(out);
        s.current = (*out).clone();
        s.steps.push(Step::Pixels(out));
        s.preview_base = None;
        s.version += 1;
        Ok(state_of(s))
    })
    .await
    .map_err(err)?
}

/// Set the colour and light adjustments (all of them; clamped to their ranges).
#[tauri::command]
pub fn edit_adjust(adjust: cs_adjust::Adjust) -> Res<EditState> {
    let mut g = lock();
    let s = g.as_mut().ok_or("No photo is open for editing.")?;
    s.adjust = adjust.clamped();
    s.version += 1;
    Ok(state_of(s))
}

/// Set the frame and watermark placement (non-destructive, applied last).
#[tauri::command]
pub fn edit_effects(effects: cs_adjust::effects::Effects) -> Res<EditState> {
    let mut g = lock();
    let s = g.as_mut().ok_or("No photo is open for editing.")?;
    s.effects = effects.clamped();
    s.version += 1;
    Ok(state_of(s))
}

/// The watermark picture: a text drawn by the app (`png`, base64 data URL),
/// a logo file (`path`), or neither to remove it.
#[tauri::command]
pub async fn edit_watermark(png: Option<String>, path: Option<String>) -> Res<EditState> {
    tauri::async_runtime::spawn_blocking(move || {
        let layer = match (png, path) {
            (Some(png), _) => {
                use base64::Engine;
                let b64 = png.split_once(',').map_or(png.as_str(), |(_, b)| b);
                let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(err)?;
                Some((cs_adjust::effects::Layer::from_png(&bytes)?, "text".to_string()))
            }
            (None, Some(path)) => {
                let p = PathBuf::from(&path);
                let photo = cs_image::open(&p).map_err(|e| format!("The logo could not be opened: {e}"))?;
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(path);
                Some((cs_adjust::effects::Layer::from_image(&photo.image)?, name))
            }
            (None, None) => None,
        };
        let mut g = lock();
        let s = g.as_mut().ok_or("No photo is open for editing.")?;
        s.layer = layer.map(|(l, n)| (std::sync::Arc::new(l), n));
        s.version += 1;
        Ok(state_of(s))
    })
    .await
    .map_err(err)?
}

/// Set (or clear with `None`) the print resolution written on save.
#[tauri::command]
pub fn edit_set_dpi(dpi: Option<u32>) -> Res<EditState> {
    if dpi.is_some_and(|d| !(1..=10_000).contains(&d)) {
        return Err("The resolution must be between 1 and 10 000 dots per inch.".into());
    }
    let mut g = lock();
    let s = g.as_mut().ok_or("No photo is open for editing.")?;
    s.dpi = dpi;
    s.version += 1;
    Ok(state_of(s))
}

/// Auto: suggested adjustments for the picture as it is now (not applied).
#[tauri::command]
pub async fn edit_auto() -> Res<cs_adjust::Adjust> {
    tauri::async_runtime::spawn_blocking(move || {
        let g = lock();
        let s = g.as_ref().ok_or("No photo is open for editing.")?;
        cs_adjust::auto(&s.current)
    })
    .await
    .map_err(err)?
}

/// The adjustment sliders (ranges and defaults from photocraft's develop model).
#[tauri::command]
pub fn edit_controls() -> Vec<cs_adjust::Control> {
    cs_adjust::controls()
}

/// The picture to save: the steps, then the adjustments, at full size.
fn finished(s: &Session) -> Res<std::borrow::Cow<'_, cs_image::Image>> {
    let mut pic = if s.adjust.is_neutral() {
        std::borrow::Cow::Borrowed(&s.current)
    } else {
        std::borrow::Cow::Owned(cs_adjust::render(&s.current, &s.adjust)?)
    };
    if s.has_effects() {
        pic = std::borrow::Cow::Owned(cs_adjust::effects::apply(&pic, &s.effects, s.layer())?);
    }
    if let Some(d) = s.dpi {
        pic.to_mut().meta.dpi = Some((d as f32, d as f32));
    }
    Ok(pic)
}

/// ExifTool arguments that write the print resolution into EXIF (and JFIF for JPEG).
fn dpi_args(dpi: u32, jpeg: bool) -> Vec<String> {
    let mut a = vec![
        format!("-EXIF:XResolution={dpi}"),
        format!("-EXIF:YResolution={dpi}"),
        "-EXIF:ResolutionUnit=inches".to_string(),
    ];
    if jpeg {
        a.extend([
            format!("-JFIF:XResolution={dpi}"),
            format!("-JFIF:YResolution={dpi}"),
            "-JFIF:ResolutionUnit=inches".into(),
        ]);
    }
    a
}

#[tauri::command]
pub fn edit_close() {
    *lock() = None;
}

/// `edit://<version>`: the current edited picture as JPEG (longest side 1600 px).
/// `raw` leaves out the frame and watermark (while cropping, straightening or
/// retouching, whose marks are placed on the picture itself).
pub fn edit_response(raw: bool) -> tauri::http::Response<Vec<u8>> {
    let r = (|| -> Res<Vec<u8>> {
        let rgb = edit_preview(raw)?;
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

/// The edited picture as shown in the viewer (longest side 1600 px), as RGB.
pub(crate) fn edit_preview(raw: bool) -> Res<image::RgbImage> {
    {
        let mut g = lock();
        let s = g.as_mut().ok_or("nothing open")?;
        if s.preview_base.is_none() {
            let rgb = cs_image::preview_rgb8(&s.current, PREVIEW_SIDE)?;
            let (w, h) = rgb.dimensions();
            s.preview_base = Some(
                cs_image::Image::from_raw(w, h, cs_image::ChannelLayout::Rgb, cs_image::SampleType::U8, rgb.into_raw())
                    .map_err(err)?,
            );
        }
        let base = s.preview_base.as_ref().ok_or("no preview")?;
        let mut shown = cs_adjust::render(base, &s.adjust)?;
        if s.has_effects() && !raw {
            shown = cs_adjust::effects::apply(&shown, &s.effects, s.layer())?;
        }
        cs_image::preview_rgb8(&shown, PREVIEW_SIDE)
    }
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
        let lossless_tag =
            (fmt == Format::Jpeg && s.original.format == Format::Jpeg && s.adjust.is_neutral() && !s.has_effects())
                .then(|| s.plain_edits().and_then(|e| cs_image::orientation_after(s.original.orientation, &e)))
                .flatten();

        // 1. Write the new picture to a temporary name in the same folder.
        let tmp = fileops::free_name(&dir, &format!(".{stem}.chronosort-edit.{ext}"));
        let mut warnings = Vec::new();
        let written = (|| -> Res<()> {
            // Tags ExifTool writes into the new file: the orientation (lossless JPEG
            // turn) and the print resolution (JPEG / TIFF keep it in EXIF too).
            let mut tags: Vec<String> = Vec::new();
            if let Some(tag) = lossless_tag {
                std::fs::copy(&src, &tmp).map_err(err)?;
                tags.push(format!("-Orientation={tag}"));
            } else {
                let pic = finished(s)?;
                warnings = cs_image::warnings(&pic, fmt);
                cs_image::save(&pic, fmt, cs_image::SaveOptions { quality }, &tmp)?;
            }
            if let Some(d) = s.dpi
                && matches!(fmt, Format::Jpeg | Format::Tiff)
            {
                tags.extend(dpi_args(d, fmt == Format::Jpeg));
            }
            if !tags.is_empty() {
                let mut et = cs_geo::exiftool::ExifTool::start(&crate::places::exiftool()?).map_err(err)?;
                let mut args: Vec<String> =
                    ["-charset", "filename=utf8", "-overwrite_original", "-n"].map(String::from).to_vec();
                args.extend(tags);
                args.push(tmp.to_string_lossy().into_owned());
                let out = et.run(&args).map_err(err)?;
                if !out.iter().any(|l| l.contains("1 image files updated")) {
                    return Err(format!("Could not write the picture's tags: {}", out.join(" ")));
                }
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
            steps: vec![],
            version: 1,
            adjust: Default::default(),
            preview_base: None,
            dpi: None,
            effects: Default::default(),
            layer: None,
        });
    }
    fn apply_now(e: Edit) {
        let mut g = lock();
        let s = g.as_mut().unwrap();
        s.current = cs_image::apply(s.current.clone(), e).unwrap();
        s.steps.push(Step::Edit(e));
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

    /// Retouching is a step: it can be undone, and it turns off lossless saving.
    #[test]
    fn retouch_steps_undo_and_are_not_lossless() {
        let _l = crate::fileops::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("r.jpg");
        image::RgbImage::from_fn(64, 48, |x, _| image::Rgb([if x < 32 { 250 } else { 10 }, 20, 20])).save(&f).unwrap();
        open_session(&f);
        let before = lock().as_ref().unwrap().current.data().to_vec();
        apply_now(Edit::RotateRight);
        let st = tauri::async_runtime::block_on(edit_retouch(Retouch::Clone {
            dabs: vec![cs_adjust::retouch::Dab { x: 10.0, y: 10.0, r: 4.0 }],
            dx: 0.0,
            dy: 40.0,
            hardness: 100.0,
        }))
        .unwrap();
        assert_eq!((st.edits, st.lossless), (2, false));
        let st = tauri::async_runtime::block_on(edit_step_back()).unwrap();
        assert_eq!((st.edits, st.lossless), (1, true), "back to a plain turn");
        tauri::async_runtime::block_on(edit_step_back()).unwrap();
        assert_eq!(lock().as_ref().unwrap().current.data(), &before[..]);
        // A drawn layer of the wrong size is refused and adds no step.
        let st = state_of(lock().as_ref().unwrap());
        assert!(
            tauri::async_runtime::block_on(edit_retouch(Retouch::Draw { png: "data:image/png;base64,AAAA".into() }))
                .is_err()
        );
        assert_eq!(state_of(lock().as_ref().unwrap()).edits, st.edits);
        *lock() = None;
    }

    /// Frame + watermark: not lossless any more, the reported and saved size
    /// include the border, the mark is on the picture, removing them restores lossless.
    #[test]
    fn frame_and_watermark_are_saved_last() {
        let _l = crate::fileops::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("fx.jpg");
        image::RgbImage::from_pixel(64, 48, image::Rgb([20, 20, 20])).save(&f).unwrap();
        open_session(&f);
        apply_now(Edit::RotateRight); // 48 × 64, lossless turn
        assert!(state_of(lock().as_ref().unwrap()).lossless);
        let mut fx = cs_adjust::effects::Effects {
            frame: cs_adjust::effects::Frame::Border,
            size: 10.0,
            color: [255, 255, 255],
            ..Default::default()
        };
        fx.watermark = cs_adjust::effects::Placement { on: true, anchor: 8, width: 50.0, margin: 0.0, opacity: 100.0 };
        let st = edit_effects(fx).unwrap();
        assert_eq!((st.width, st.height, st.out_width, st.out_height), (48, 64, 58, 74));
        assert!(!st.lossless && st.has_effects);
        // The watermark only counts once it has a picture.
        assert_eq!(st.watermark, None);
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 255, 0, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        use base64::Engine;
        let url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png));
        let st = tauri::async_runtime::block_on(edit_watermark(Some(url), None)).unwrap();
        assert_eq!(st.watermark.as_deref(), Some("text"));
        // Preview: with the frame, and without it for the placing tools.
        assert_eq!(edit_response(false).status(), 200);
        let framed = image::load_from_memory(edit_response(false).body()).unwrap();
        let raw = image::load_from_memory(edit_response(true).body()).unwrap();
        assert_eq!((framed.width(), framed.height(), raw.width(), raw.height()), (58, 74, 48, 64));
        let saved = tauri::async_runtime::block_on(edit_save("copy".into(), "PNG".into(), 90, None)).unwrap();
        let out = image::open(&saved.path).unwrap().to_rgb8();
        assert_eq!(out.dimensions(), (58, 74));
        assert_eq!(out.get_pixel(0, 0).0, [255, 255, 255]); // frame
        assert_eq!(out.get_pixel(20, 20).0, [20, 20, 20]); // picture
        assert_eq!(out.get_pixel(50, 66).0, [255, 255, 0]); // mark, bottom right of the picture
        // Remove both: lossless again.
        let st = edit_effects(cs_adjust::effects::Effects::default()).unwrap();
        assert!(st.lossless && !st.has_effects);
        assert_eq!((st.out_width, st.out_height), (48, 64));
        *lock() = None;
    }

    /// DPI only → lossless (same pixels, new resolution tags); straighten,
    /// canvas and colour adjustments → re-encoded at the expected size.
    #[test]
    fn dpi_straighten_canvas_and_adjust_are_saved() {
        let _l = crate::fileops::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if crate::places::exiftool().is_err() {
            eprintln!("skip: no ExifTool");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("scan.jpg");
        image::RgbImage::from_fn(120, 80, |x, y| image::Rgb([(x * 2) as u8, (y * 3) as u8, 140])).save(&f).unwrap();
        let tag = |p: &std::path::Path, n: &str| {
            crate::meta::read(p).unwrap().tags.into_iter().filter(|t| t.name == n).map(|t| t.value).collect::<Vec<_>>()
        };

        open_session(&f);
        edit_set_dpi(Some(300)).unwrap();
        let r = tauri::async_runtime::block_on(edit_save("copy".into(), "JPEG".into(), 92, None)).unwrap();
        assert!(r.lossless, "only the resolution changed: no re-encode");
        let copy = std::path::PathBuf::from(&r.path);
        assert!(tag(&copy, "XResolution").iter().all(|v| v == "300") && tag(&copy, "XResolution").len() >= 2);
        let (a, b) = (cs_image::open(&f).unwrap().image, cs_image::open(&copy).unwrap().image);
        assert_eq!(a.data(), b.data(), "same pixels");
        assert!(edit_set_dpi(Some(0)).is_err());

        open_session(&f);
        apply_now(Edit::Straighten { angle100: 350 });
        apply_now(Edit::Canvas { w: 200, h: 100, anchor: 4, color: [255, 255, 255] });
        edit_adjust(cs_adjust::Adjust { exposure: 0.5, ..Default::default() }).unwrap();
        let r = tauri::async_runtime::block_on(edit_save("copy".into(), "PNG".into(), 92, None)).unwrap();
        assert!(!r.lossless);
        let out = cs_image::open(std::path::Path::new(&r.path)).unwrap().image;
        assert_eq!(out.dimensions(), (200, 100));
        *lock() = None;
    }
}
