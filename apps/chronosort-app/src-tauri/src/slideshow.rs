//! Slideshow support: streaming videos and music to the webview, and keeping
//! the screen awake while the show runs.
//!
//! `media://localhost/<id>` serves a file in byte ranges (what `<video>` and
//! `<audio>` ask for), so a long video or song is never read whole. Only ids the
//! app handed out are served (files listed by the browser, or music the user
//! picked), and only kinds the webview can play. Nothing leaves the PC.

use crate::{AppState, Res};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use tauri::State;

/// Biggest piece sent for one request (the player asks for the next one).
const CHUNK: u64 = 4 * 1024 * 1024;

/// What the webview can play, by extension. `None` = not playable here.
pub fn playable_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    Some(match ext.as_str() {
        // QuickTime files are MP4's sibling; the webview plays their H.264/HEVC/AAC as MP4.
        "mp4" | "m4v" | "mov" | "qt" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        _ => return None,
    })
}

/// A byte range `start..=end` from a `Range: bytes=a-b` header (one range only).
fn parse_range(h: &str, len: u64) -> Option<(u64, u64)> {
    let spec = h.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || len == 0 {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (a, b) = (a.trim(), b.trim());
    let (start, end) = if a.is_empty() {
        // Last `b` bytes.
        let n: u64 = b.parse().ok()?;
        if n == 0 {
            return None;
        }
        (len.saturating_sub(n), len - 1)
    } else {
        let s: u64 = a.parse().ok()?;
        let e = if b.is_empty() { len - 1 } else { b.parse::<u64>().ok()?.min(len - 1) };
        (s, e)
    };
    (start <= end && start < len).then_some((start, end))
}

/// Answer one media request.
pub fn media_response(app: &tauri::AppHandle, path: &str, range: Option<&str>) -> tauri::http::Response<Vec<u8>> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let base = || tauri::http::Response::builder().header("Cache-Control", "no-store").header("Accept-Ranges", "bytes");
    let fail = |b: tauri::http::response::Builder, code: u16, msg: String| {
        b.header("Content-Type", "text/plain").status(code).body(msg.into_bytes())
    };
    let r = (|| -> Result<tauri::http::Response<Vec<u8>>, (u16, String)> {
        let id: u64 = path.trim_start_matches('/').parse().map_err(|_| (400, "bad request".to_string()))?;
        let file = state.browse.path(id).ok_or((404, "unknown file".to_string()))?;
        let ctype = playable_type(&file).ok_or((415, "cannot be played here".to_string()))?;
        let mut f = std::fs::File::open(&file).map_err(|e| (404, e.to_string()))?;
        let len = f.metadata().map_err(|e| (500, e.to_string()))?.len();
        let b = base().header("Content-Type", ctype);
        match range {
            Some(h) => {
                let Some((start, end)) = parse_range(h, len) else {
                    return Ok(fail(b.header("Content-Range", format!("bytes */{len}")), 416, "bad range".into())
                        .expect("static response"));
                };
                let end = end.min(start + CHUNK - 1);
                let mut buf = vec![0u8; (end - start + 1) as usize];
                f.seek(SeekFrom::Start(start)).map_err(|e| (500, e.to_string()))?;
                f.read_exact(&mut buf).map_err(|e| (500, e.to_string()))?;
                Ok(b.status(206)
                    .header("Content-Range", format!("bytes {start}-{end}/{len}"))
                    .header("Content-Length", buf.len().to_string())
                    .body(buf)
                    .expect("static response"))
            }
            None => {
                // No range: small files whole, big ones as the first piece (the player then asks for ranges).
                let n = len.min(CHUNK);
                let mut buf = vec![0u8; n as usize];
                f.read_exact(&mut buf).map_err(|e| (500, e.to_string()))?;
                let b = if n < len {
                    b.status(206).header("Content-Range", format!("bytes 0-{}/{len}", n - 1))
                } else {
                    b.status(200)
                };
                Ok(b.header("Content-Length", n.to_string()).body(buf).expect("static response"))
            }
        }
    })();
    r.unwrap_or_else(|(code, msg)| fail(base(), code, msg).expect("static response"))
}

/// Ids for music files the user picked for the slideshow (only playable audio).
#[tauri::command]
pub fn slideshow_music(state: State<'_, AppState>, paths: Vec<String>) -> Res<Vec<(u64, String)>> {
    let mut out = Vec::new();
    for p in paths {
        let path = std::path::PathBuf::from(&p);
        let ok = playable_type(&path).is_some_and(|t| t.starts_with("audio/")) && path.is_file();
        if !ok {
            return Err(format!(
                "“{}” cannot be played as music. Use MP3, M4A, AAC, WAV, OGG, Opus or FLAC.",
                path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
            ));
        }
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(p);
        out.push((state.browse.id_for(&path), name));
    }
    Ok(out)
}

/// Keep the screen on while the slideshow plays (and let it sleep again after).
#[tauri::command]
pub fn slideshow_awake(on: bool) {
    cs_platform::keep_awake(on);
}

/// Whether a browse id is a video the slideshow can play (others show their first frame).
#[tauri::command]
pub fn slideshow_playable(state: State<'_, AppState>, ids: Vec<u64>) -> Vec<u64> {
    ids.into_iter()
        .filter(|&id| state.browse.path(id).is_some_and(|p| playable_type(&p).is_some_and(|t| t.starts_with("video/"))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=90-500", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-500", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=20-10", 100), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 100), None);
        assert_eq!(parse_range("bytes=-0", 100), None);
        assert_eq!(parse_range("items=0-1", 100), None);
        assert_eq!(parse_range("bytes=0-", 0), None);
    }

    #[test]
    fn only_playable_kinds() {
        assert_eq!(playable_type(Path::new("a.MP4")), Some("video/mp4"));
        assert_eq!(playable_type(Path::new("a.mov")), Some("video/mp4"));
        assert_eq!(playable_type(Path::new("song.mp3")), Some("audio/mpeg"));
        assert_eq!(playable_type(Path::new("a.avi")), None);
        assert_eq!(playable_type(Path::new("a.jpg")), None);
        assert_eq!(playable_type(Path::new("a.exe")), None);
        assert_eq!(playable_type(Path::new("noext")), None);
    }
}
