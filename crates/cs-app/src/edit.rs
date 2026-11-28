//! Picture edits in the viewer: rotate / flip, crop, resize, straighten, canvas, retouching (red eye, heal,
//! clone, drawing), colour and light (`cs-adjust`), frame and watermark, print size, Save As.
//!
//! The picture is edited in memory (photocraft codecs via `cs-image`); the file on disk is only touched by
//! [`Session::save`]:
//! * "copy" (default): a new file next to the original (never overwrites).
//! * "replace": the edited picture takes the original's place and the original is kept in
//!   `<folder>/.chronosort/edit-originals/<run>/` so Undo can put it back.
//! * A JPEG that was only turned or flipped is saved **losslessly**: the pixels are copied unchanged and only
//!   the EXIF orientation tag is changed. The saved file keeps the original's file dates.
//!
//! Same logic as the Tauri app's `edit.rs`; here a [`Session`] is an object the UI owns. Its methods block:
//! call them from a worker thread.

use crate::fileops::{self, Op, Undo};
use crate::{Res, err};
pub use cs_image::Edit;
use cs_image::{Format, Photo};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One step of the history. Retouching keeps its result, so Undo can replay the history without redoing it.
#[derive(Clone)]
enum Step {
    Edit(Edit),
    Pixels(Arc<cs_image::Image>),
}

/// A retouching step (coordinates in pixels of the picture as it is now).
#[derive(Clone, Debug)]
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
    /// A drawn layer (lines, shapes, text) as PNG bytes, the picture's size.
    Draw {
        png: Vec<u8>,
    },
}

/// The watermark picture.
pub enum Watermark {
    /// A text drawn by the UI, as PNG bytes.
    Text(Vec<u8>),
    /// A logo file.
    Logo(PathBuf),
    None,
}

/// Longest side of the edit preview.
pub const PREVIEW_SIDE: u32 = 1600;

pub struct Session {
    path: PathBuf,
    original: Photo,
    steps: Vec<Step>,
    current: cs_image::Image,
    version: u64,
    adjust: cs_adjust::Adjust,
    preview_base: Option<cs_image::Image>,
    dpi: Option<u32>,
    effects: cs_adjust::effects::Effects,
    layer: Option<(Arc<cs_adjust::effects::Layer>, String)>,
}

#[derive(Clone, Debug)]
pub struct EditState {
    /// The picture being edited (crop, retouch etc. work in these pixels).
    pub width: u32,
    pub height: u32,
    /// The finished picture's size: bigger when a frame adds a border.
    pub out_width: u32,
    pub out_height: u32,
    pub format: String,
    pub edits: usize,
    /// Bumps with every change.
    pub version: u64,
    /// True when saving as JPEG would be lossless (turns / flips of a JPEG only).
    pub lossless: bool,
    pub adjusted: bool,
    pub adjust: cs_adjust::Adjust,
    pub dpi_original: Option<f32>,
    pub dpi: Option<u32>,
    pub effects: cs_adjust::effects::Effects,
    pub has_effects: bool,
    pub watermark: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Saved {
    pub path: PathBuf,
    pub lossless: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SaveMode {
    /// A new file next to the original.
    Copy,
    /// Replace the original (kept in a hidden backup folder for Undo).
    Replace,
}

impl Session {
    pub fn open(path: &Path) -> Res<Self> {
        let original = cs_image::open(path)?;
        Ok(Self {
            current: original.image.clone(),
            path: path.to_path_buf(),
            original,
            steps: Vec::new(),
            version: crate::now_unix() as u64,
            adjust: cs_adjust::Adjust::default(),
            preview_base: None,
            dpi: None,
            effects: Default::default(),
            layer: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

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

    /// Anything to save?
    pub fn dirty(&self) -> bool {
        !self.steps.is_empty() || !self.adjust.is_neutral() || self.has_effects() || self.dpi.is_some()
    }

    pub fn state(&self) -> EditState {
        let (width, height) = self.current.dimensions();
        let (out_width, out_height) = if self.has_effects() {
            cs_adjust::effects::output_size(width, height, &self.effects)
        } else {
            (width, height)
        };
        EditState {
            out_width,
            out_height,
            width,
            height,
            format: format!("{:?}", self.original.format),
            edits: self.steps.len(),
            version: self.version,
            lossless: self.original.format == Format::Jpeg
                && self.adjust.is_neutral()
                && !self.has_effects()
                && self
                    .plain_edits()
                    .is_some_and(|e| cs_image::orientation_after(self.original.orientation, &e).is_some()),
            adjusted: !self.adjust.is_neutral(),
            adjust: self.adjust.clone(),
            dpi_original: self.original.image.meta.dpi.map(|d| d.0).filter(|d| d.is_finite() && *d > 0.0),
            dpi: self.dpi,
            effects: self.effects,
            has_effects: self.has_effects(),
            watermark: self.layer.as_ref().map(|(_, n)| n.clone()),
        }
    }

    fn changed(&mut self) {
        self.preview_base = None;
        self.version += 1;
    }

    pub fn apply(&mut self, edit: Edit) -> Res<EditState> {
        let next = cs_image::apply(self.current.clone(), edit)?;
        self.current = next;
        self.steps.push(Step::Edit(edit));
        self.changed();
        Ok(self.state())
    }

    /// Take back the last step (recomputed from the original).
    pub fn step_back(&mut self) -> Res<EditState> {
        self.steps.pop();
        let mut img = self.original.image.clone();
        for st in &self.steps {
            img = match st {
                Step::Edit(e) => cs_image::apply(img, *e)?,
                Step::Pixels(p) => (**p).clone(),
            };
        }
        self.current = img;
        self.changed();
        Ok(self.state())
    }

    /// Throw away every change.
    pub fn reset(&mut self) -> EditState {
        self.steps.clear();
        self.current = self.original.image.clone();
        self.adjust = Default::default();
        self.effects = Default::default();
        self.layer = None;
        self.dpi = None;
        self.changed();
        self.state()
    }

    pub fn retouch(&mut self, op: Retouch) -> Res<EditState> {
        let out = match op {
            Retouch::RedEye { eyes } => cs_adjust::retouch::red_eye(&self.current, &eyes)?,
            Retouch::Heal { dabs } => cs_adjust::retouch::heal(&self.current, &dabs)?,
            Retouch::Clone { dabs, dx, dy, hardness } => {
                cs_adjust::retouch::clone_stamp(&self.current, &dabs, dx, dy, hardness)?
            }
            Retouch::Draw { png } => cs_adjust::retouch::overlay(&self.current, &png)?,
        };
        let out = Arc::new(out);
        self.current = (*out).clone();
        self.steps.push(Step::Pixels(out));
        self.changed();
        Ok(self.state())
    }

    /// Colour and light (all of them; clamped to their ranges). Non-destructive.
    pub fn set_adjust(&mut self, adjust: cs_adjust::Adjust) -> EditState {
        self.adjust = adjust.clamped();
        self.version += 1;
        self.state()
    }

    /// Frame and watermark placement (non-destructive, applied last).
    pub fn set_effects(&mut self, effects: cs_adjust::effects::Effects) -> EditState {
        self.effects = effects.clamped();
        self.version += 1;
        self.state()
    }

    pub fn set_watermark(&mut self, w: Watermark) -> Res<EditState> {
        self.layer = match w {
            Watermark::Text(png) => Some((Arc::new(cs_adjust::effects::Layer::from_png(&png)?), "text".to_string())),
            Watermark::Logo(p) => {
                let photo = cs_image::open(&p).map_err(|e| format!("The logo could not be opened: {e}"))?;
                let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                Some((Arc::new(cs_adjust::effects::Layer::from_image(&photo.image)?), name))
            }
            Watermark::None => None,
        };
        self.version += 1;
        Ok(self.state())
    }

    /// Set (or clear with `None`) the print resolution written on save.
    pub fn set_dpi(&mut self, dpi: Option<u32>) -> Res<EditState> {
        if dpi.is_some_and(|d| !(1..=10_000).contains(&d)) {
            return Err("The resolution must be between 1 and 10 000 dots per inch.".into());
        }
        self.dpi = dpi;
        self.version += 1;
        Ok(self.state())
    }

    /// Suggested colour and light for the picture as it is now (not applied).
    pub fn auto(&self) -> Res<cs_adjust::Adjust> {
        cs_adjust::auto(&self.current)
    }

    /// The edited picture as shown in the viewer (longest side [`PREVIEW_SIDE`]), RGB.
    /// `raw` leaves out the frame and watermark (while cropping, straightening or retouching).
    pub fn preview(&mut self, raw: bool) -> Res<image::RgbImage> {
        if self.preview_base.is_none() {
            let rgb = cs_image::preview_rgb8(&self.current, PREVIEW_SIDE)?;
            let (w, h) = rgb.dimensions();
            self.preview_base = Some(
                cs_image::Image::from_raw(w, h, cs_image::ChannelLayout::Rgb, cs_image::SampleType::U8, rgb.into_raw())
                    .map_err(err)?,
            );
        }
        let base = self.preview_base.as_ref().ok_or("no preview")?;
        let mut shown = cs_adjust::render(base, &self.adjust)?;
        if self.has_effects() && !raw {
            shown = cs_adjust::effects::apply(&shown, &self.effects, self.layer())?;
        }
        cs_image::preview_rgb8(&shown, PREVIEW_SIDE)
    }

    /// The picture to save: the steps, then the adjustments, then frame / watermark, at full size.
    fn finished(&self) -> Res<std::borrow::Cow<'_, cs_image::Image>> {
        let mut pic = if self.adjust.is_neutral() {
            std::borrow::Cow::Borrowed(&self.current)
        } else {
            std::borrow::Cow::Owned(cs_adjust::render(&self.current, &self.adjust)?)
        };
        if self.has_effects() {
            pic = std::borrow::Cow::Owned(cs_adjust::effects::apply(&pic, &self.effects, self.layer())?);
        }
        if let Some(d) = self.dpi {
            pic.to_mut().meta.dpi = Some((d as f32, d as f32));
        }
        Ok(pic)
    }

    /// Write the edit. `format`: "JPEG" | "PNG" | "TIFF" | "WebP". `name`: the copy's file name (checked).
    pub fn save(&self, mode: SaveMode, format: &str, quality: u8, name: Option<&str>) -> Res<Saved> {
        let (fmt, ext) = format_of(format)?;
        let src = self.path.clone();
        fileops::guard(&src)?;
        let dir = src.parent().ok_or("no folder")?.to_path_buf();
        let stem = src.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "photo".into());
        let times = cs_platform::read_times(&src).ok();
        let lossless_tag = (fmt == Format::Jpeg
            && self.original.format == Format::Jpeg
            && self.adjust.is_neutral()
            && !self.has_effects())
        .then(|| self.plain_edits().and_then(|e| cs_image::orientation_after(self.original.orientation, &e)))
        .flatten();

        // 1. Write the new picture to a temporary name in the same folder.
        let tmp = fileops::free_name(&dir, &format!(".{stem}.chronosort-edit.{ext}"));
        let mut warnings = Vec::new();
        let written = (|| -> Res<()> {
            let mut tags: Vec<String> = Vec::new();
            if let Some(tag) = lossless_tag {
                std::fs::copy(&src, &tmp).map_err(err)?;
                tags.push(format!("-Orientation={tag}"));
            } else {
                let pic = self.finished()?;
                warnings = cs_image::warnings(&pic, fmt);
                cs_image::save(&pic, fmt, cs_image::SaveOptions { quality }, &tmp)?;
            }
            if let Some(d) = self.dpi
                && matches!(fmt, Format::Jpeg | Format::Tiff)
            {
                tags.extend(dpi_args(d, fmt == Format::Jpeg));
            }
            if !tags.is_empty() {
                let mut args: Vec<String> =
                    ["-charset", "filename=utf8", "-overwrite_original", "-n"].map(String::from).to_vec();
                args.extend(tags);
                args.push(tmp.to_string_lossy().into_owned());
                let out = crate::exiftool::run(&args)?;
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
        let final_path = if mode == SaveMode::Replace {
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
            let want = name.map(fileops::check_name).transpose()?.unwrap_or_else(|| format!("{stem}_edited.{ext}"));
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
        Ok(Saved { path: final_path, lossless: lossless_tag.is_some(), warnings })
    }
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

fn format_of(name: &str) -> Res<(Format, &'static str)> {
    cs_image::save_formats()
        .into_iter()
        .find(|(_, n, _)| n.eq_ignore_ascii_case(name))
        .map(|(f, _, e)| (f, e))
        .ok_or_else(|| format!("Unknown format {name}"))
}

/// The formats offered in "Save as".
pub fn formats() -> Vec<&'static str> {
    cs_image::save_formats().iter().map(|f| f.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lossless turn as a copy, then crop + replace the original, then Undo brings the original back.
    #[test]
    fn save_copy_lossless_then_replace_and_undo() {
        let _l = crate::fileops::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if crate::exiftool::path().is_err() {
            eprintln!("skip: no ExifTool");
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("pic.jpg");
        image::RgbImage::from_fn(40, 30, |x, y| image::Rgb([(x * 6) as u8, (y * 8) as u8, 90])).save(&f).unwrap();
        let original_bytes = std::fs::read(&f).unwrap();

        let mut s = Session::open(&f).unwrap();
        s.apply(Edit::RotateRight).unwrap();
        assert!(s.state().lossless && s.dirty());
        let r = s.save(SaveMode::Copy, "JPEG", 92, None).unwrap();
        assert!(r.lossless);
        assert_eq!(r.path.file_name().unwrap(), "pic_edited.jpg");
        assert_eq!(cs_image::open(&r.path).unwrap().image.dimensions(), (30, 40));
        assert_eq!(std::fs::read(&f).unwrap(), original_bytes, "original untouched");

        let mut s = Session::open(&f).unwrap();
        s.apply(Edit::Crop { x: 0, y: 0, w: 20, h: 10 }).unwrap();
        let r = s.save(SaveMode::Replace, "JPEG", 90, None).unwrap();
        assert!(!r.lossless);
        assert_eq!(cs_image::open(&f).unwrap().image.dimensions(), (20, 10));
        let op = crate::fileops::take_last_for_test();
        crate::fileops::undo_for_test(op).unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), original_bytes);
    }

    #[test]
    fn step_back_reset_and_preview() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("r.jpg");
        image::RgbImage::from_fn(64, 48, |x, _| image::Rgb([if x < 32 { 250 } else { 10 }, 20, 20])).save(&f).unwrap();
        let mut s = Session::open(&f).unwrap();
        assert!(!s.dirty());
        s.apply(Edit::RotateRight).unwrap();
        let st = s.retouch(Retouch::Clone {
            dabs: vec![cs_adjust::retouch::Dab { x: 10.0, y: 10.0, r: 4.0 }],
            dx: 0.0,
            dy: 40.0,
            hardness: 100.0,
        });
        let st = st.unwrap();
        assert_eq!((st.edits, st.lossless, st.width, st.height), (2, false, 48, 64));
        let st = s.step_back().unwrap();
        assert_eq!((st.edits, st.lossless), (1, true));
        assert_eq!(s.preview(false).unwrap().dimensions(), (48, 64));
        let st = s.reset();
        assert_eq!((st.edits, st.width), (0, 64));
        assert!(!s.dirty());
    }
}
