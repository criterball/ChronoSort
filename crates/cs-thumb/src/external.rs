//! Previews of videos and HEIC/AVIF photos made in a separate process, so a
//! crash, a hang or a hostile file cannot take ChronoSort down.
//!
//! * Videos (MP4/MOV): ChronoSort's own pure-Rust decoder (`cs-video`), run as
//!   a copy of the program itself with a hidden argument (`video_helper_main`).
//! * Anything it cannot read, and HEIC/AVIF photos: FFmpeg, when it is set up.
//!
//! Safety: arguments are passed as a list (no shell); FFmpeg may only read
//! local files (`-protocol_whitelist file,pipe`, input given as `file:`); no
//! console window; a time limit; the output is capped and decoded with the
//! usual pixel limits. Without either, these files get a placeholder.

use crate::ThumbError;
use image::DynamicImage;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::RwLock;
use std::time::{Duration, Instant};

static FFMPEG: RwLock<Option<PathBuf>> = RwLock::new(None);
/// The program that makes video posters (`<exe> --cs-video-poster <max side> <file>`).
static VIDEO_HELPER: RwLock<Option<PathBuf>> = RwLock::new(None);
const HELPER_ARG: &str = "--cs-video-poster";

/// How long one preview may take before FFmpeg is stopped.
const TIME_LIMIT: Duration = Duration::from_secs(20);
/// Largest PNG accepted from FFmpeg.
const MAX_OUTPUT: u64 = 64 << 20;

/// Tell the previewer where `ffmpeg(.exe)` is (None = not available).
pub fn set_ffmpeg(path: Option<PathBuf>) {
    if let Ok(mut f) = FFMPEG.write() {
        *f = path.filter(|p| p.is_file());
    }
}

pub fn ffmpeg_available() -> bool {
    FFMPEG.read().is_ok_and(|f| f.is_some())
}

/// Call first thing in `main()`. Started as the video helper, this makes the
/// poster, writes it to stdout as PNG and returns the exit code to end with;
/// otherwise it registers this program as the video helper and returns None.
pub fn video_helper_main() -> Option<i32> {
    let a: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if a.first().is_some_and(|x| x == HELPER_ARG) {
        let max_side = a.get(1).and_then(|x| x.to_str()?.parse().ok()).unwrap_or(crate::DEFAULT_SIZE);
        let Some(file) = a.get(2) else { return Some(2) };
        return Some(match cs_video::poster(Path::new(file), max_side.clamp(16, 4096)) {
            Ok(img) => {
                let mut png = std::io::Cursor::new(Vec::new());
                if img.write_to(&mut png, image::ImageFormat::Png).is_err() {
                    return Some(1);
                }
                use std::io::Write;
                let mut out = std::io::stdout().lock();
                if out.write_all(png.get_ref()).and_then(|_| out.flush()).is_ok() { 0 } else { 1 }
            }
            Err(e) => {
                eprintln!("{e}");
                1
            }
        });
    }
    if let (Ok(exe), Ok(mut h)) = (std::env::current_exe(), VIDEO_HELPER.write()) {
        *h = Some(exe);
    }
    None
}

/// The arguments for one preview frame (kept separate so they can be tested).
pub(crate) fn args(input: &Path, max_side: u32, seek_secs: Option<u32>) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin", "-protocol_whitelist", "file,pipe"]
        .map(String::from)
        .to_vec();
    if let Some(s) = seek_secs {
        a.extend(["-ss".into(), s.to_string()]);
    }
    a.extend(["-i".into(), format!("file:{}", input.display())]);
    a.extend(
        [
            "-frames:v",
            "1",
            "-an",
            "-sn",
            "-dn",
            "-vf",
            &format!("scale='min({max_side},iw)':'min({max_side},ih)':force_original_aspect_ratio=decrease"),
            "-f",
            "image2pipe",
            "-c:v",
            "png",
            "pipe:1",
        ]
        .map(String::from),
    );
    a
}

fn run(ff: &Path, args: &[String]) -> Result<Vec<u8>, ThumbError> {
    let mut cmd = Command::new(ff);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| ThumbError::Decode(format!("the preview helper could not start: {e}")))?;
    let mut out = child.stdout.take().ok_or(ThumbError::Unsupported("no output from the preview helper"))?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = (&mut out).take(MAX_OUTPUT).read_to_end(&mut buf);
        buf
    });
    let t = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if t.elapsed() > TIME_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ThumbError::Unsupported("the preview took too long"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    reader.join().map_err(|_| ThumbError::Unsupported("the preview output was lost"))
}

/// A poster frame from the built-in decoder (in its own process).
fn video_poster(input: &Path, max_side: u32) -> Result<DynamicImage, ThumbError> {
    let helper = VIDEO_HELPER.read().ok().and_then(|h| h.clone()).ok_or(ThumbError::Unsupported("no video helper"))?;
    let png = run(&helper, &[HELPER_ARG.into(), max_side.to_string(), input.display().to_string()])?;
    if png.is_empty() {
        return Err(ThumbError::Unsupported("the built-in video reader could not read this file"));
    }
    crate::decode_limited(image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png))
}

/// A video's poster frame: the built-in decoder first, else FFmpeg.
pub fn video(input: &Path, max_side: u32) -> Result<DynamicImage, ThumbError> {
    match video_poster(input, max_side) {
        Ok(img) => Ok(img),
        Err(e) if !ffmpeg_available() => Err(e),
        Err(_) => preview(input, max_side, true),
    }
}

/// A preview frame of a video (about 1 s in) or a HEIC/AVIF photo, by FFmpeg.
pub fn preview(input: &Path, max_side: u32, video: bool) -> Result<DynamicImage, ThumbError> {
    let ff = FFMPEG.read().ok().and_then(|f| f.clone()).ok_or(ThumbError::Unsupported("needs FFmpeg"))?;
    let mut png = run(&ff, &args(input, max_side, video.then_some(1)))?;
    if png.is_empty() && video {
        png = run(&ff, &args(input, max_side, None))?; // clips shorter than a second
    }
    if png.is_empty() {
        return Err(ThumbError::Unsupported("FFmpeg could not read this file"));
    }
    crate::decode_limited(image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_safe() {
        let a = args(Path::new("C:/Photos/-trick:name.mov"), 320, Some(1));
        let i = a.iter().position(|x| x == "-i").unwrap();
        assert_eq!(a[i + 1], "file:C:/Photos/-trick:name.mov", "always read as a plain local file");
        assert!(a.windows(2).any(|w| w[0] == "-protocol_whitelist" && w[1] == "file,pipe"));
        assert!(a.contains(&"-nostdin".to_string()) && a.last().unwrap() == "pipe:1");
        assert!(!args(Path::new("x.heic"), 320, None).contains(&"-ss".to_string()));
    }

    #[test]
    fn missing_ffmpeg_is_a_plain_error() {
        set_ffmpeg(Some(PathBuf::from("Z:/does/not/exist/ffmpeg.exe")));
        assert!(!ffmpeg_available());
        assert!(preview(Path::new("x.mov"), 320, true).is_err());
    }
}
