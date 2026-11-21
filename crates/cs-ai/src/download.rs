//! Model downloads: the only network code in ChronoSort.
//!
//! Rules (see docs/THIRD_PARTY_POLICY.md and the privacy rule in README):
//! * Only HTTPS, only allow-listed hosts (Hugging Face and its CDN), for every
//!   redirect hop too. URLs in the catalogue are pinned to a commit.
//! * Nothing about the user or their photos is sent: a plain GET with a
//!   generic User-Agent, plus a Range header when resuming.
//! * Bytes go to `<file>.part`; the exact size and SHA-256 must match before
//!   the file is renamed into place. A mismatch deletes the part file.
//! * Cancelling keeps the part file so the next attempt resumes.

use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

const USER_AGENT: &str = concat!("ChronoSort/", env!("CARGO_PKG_VERSION"), " (model download)");
const MAX_HOPS: usize = 5;
const CHUNK: usize = 1 << 20;

#[derive(Debug)]
pub enum DlError {
    NotAllowed(String),
    Http(String),
    Io(io::Error),
    Size {
        expected: u64,
        got: u64,
    },
    Checksum,
    Cancelled,
    /// No data arrived for [`IDLE`].
    Stalled,
}

impl std::fmt::Display for DlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAllowed(u) => write!(f, "Refused to download from an unknown address: {u}"),
            Self::Http(e) => write!(f, "Download failed: {e}. Check the internet connection and try again."),
            Self::Io(e) => write!(f, "Could not save the file: {e}"),
            Self::Size { expected, got } => {
                write!(f, "The download has the wrong size ({got} instead of {expected} bytes).")
            }
            Self::Stalled => write!(
                f,
                "The download stopped: no data arrived for a minute. Check the internet connection and click Download again; it continues where it stopped."
            ),
            Self::Checksum => {
                write!(f, "The downloaded file did not match its checksum, so it was deleted. Please try again.")
            }
            Self::Cancelled => write!(f, "Download cancelled. It will continue where it stopped next time."),
        }
    }
}
impl std::error::Error for DlError {}

impl From<io::Error> for DlError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    // No user-info or explicit ports: keep it simple and strict.
    (!host.is_empty() && !host.contains(['@', ':'])).then_some(host)
}

/// Catalogue URLs: Hugging Face (models) and llama.cpp's official GitHub
/// release downloads (the local AI engine). Nothing else.
pub fn allowed_url(url: &str) -> bool {
    match host_of(url) {
        Some("huggingface.co") => true,
        // Release downloads of the two programs we use, nothing else on GitHub.
        Some("github.com") => [
            "https://github.com/ggml-org/llama.cpp/releases/download/",
            "https://github.com/GyanD/codexffmpeg/releases/download/",
        ]
        .iter()
        .any(|p| url.starts_with(p)),
        _ => false,
    }
}

/// Redirect targets: Hugging Face and its CDN hosts.
fn allowed_redirect(url: &str) -> bool {
    host_of(url).is_some_and(|h| {
        let h = h.to_ascii_lowercase();
        h == "huggingface.co"
            || h.ends_with(".huggingface.co")
            || h.ends_with(".hf.co")
            // GitHub release assets (llama.cpp)
            || h == "release-assets.githubusercontent.com"
            || h == "objects.githubusercontent.com"
    })
}

/// SHA-256 of a file as lowercase hex.
pub fn sha256_file(path: &Path, progress: &dyn Fn(u64, u64)) -> io::Result<String> {
    let mut f = File::open(path)?;
    let total = f.metadata()?.len();
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut done = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        done += n as u64;
        progress(done, total);
    }
    Ok(hex(&h.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .max_redirects(0) // followed by hand so every hop is checked
        .http_status_as_error(false)
        .user_agent(USER_AGENT)
        .timeout_connect(Some(Duration::from_secs(20)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .https_only(true)
        // The operating system's own certificate store and checks (updates,
        // company / school networks with their own certificates).
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .build()
        .into()
}

/// Download `url` to `dest`, verifying `bytes` and `sha256`.
/// `progress(done, total)` is called about once per MiB.
pub fn fetch(
    url: &str,
    dest: &Path,
    bytes: u64,
    sha256: &str,
    progress: &dyn Fn(u64, u64),
    cancel: &AtomicBool,
) -> Result<(), DlError> {
    if !allowed_url(url) {
        return Err(DlError::NotAllowed(url.into()));
    }
    // Already there and correct: nothing to do.
    if std::fs::metadata(dest).is_ok_and(|m| m.len() == bytes)
        && sha256_file(dest, &|_, _| {})?.eq_ignore_ascii_case(sha256)
    {
        progress(bytes, bytes);
        return Ok(());
    }
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d)?;
    }
    let part = part_path(dest);
    let mut file = OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&part)?;
    let mut have = file.metadata()?.len();
    if have > bytes {
        file.set_len(0)?;
        have = 0;
    }

    if have < bytes {
        download_into(&agent(), url, &mut file, have, bytes, progress, cancel)?;
    }
    file.sync_all()?;
    let got = file.metadata()?.len();
    drop(file);
    if got != bytes {
        let _ = std::fs::remove_file(&part);
        return Err(DlError::Size { expected: bytes, got });
    }
    if !sha256_file(&part, &|d, t| progress(d.min(t), t))?.eq_ignore_ascii_case(sha256) {
        let _ = std::fs::remove_file(&part);
        return Err(DlError::Checksum);
    }
    std::fs::rename(&part, dest)?;
    Ok(())
}

fn download_into(
    agent: &ureq::Agent,
    url: &str,
    file: &mut File,
    mut have: u64,
    bytes: u64,
    progress: &dyn Fn(u64, u64),
    cancel: &AtomicBool,
) -> Result<(), DlError> {
    let mut url = url.to_string();
    let resp = 'hops: {
        for _ in 0..MAX_HOPS {
            let mut req = agent.get(&url);
            if have > 0 {
                req = req.header("Range", &format!("bytes={have}-"));
            }
            let resp = req.call().map_err(|e| DlError::Http(e.to_string()))?;
            let status = resp.status().as_u16();
            if (300..400).contains(&status) {
                let loc = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| DlError::Http("redirect without a target".into()))?;
                if !allowed_redirect(loc) {
                    return Err(DlError::NotAllowed(host_of(loc).unwrap_or("?").to_string()));
                }
                url = loc.to_string();
                continue;
            }
            break 'hops resp;
        }
        return Err(DlError::Http("too many redirects".into()));
    };

    match resp.status().as_u16() {
        206 => {
            // Resume must start exactly where the part file ends.
            let start = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("bytes "))
                .and_then(|v| v.split('-').next())
                .and_then(|v| v.parse::<u64>().ok());
            if start != Some(have) {
                return Err(DlError::Http("the server resumed at the wrong place".into()));
            }
        }
        200 => {
            // Server ignored the Range: start over.
            file.set_len(0)?;
            have = 0;
        }
        416 if have == bytes => return Ok(()),
        s => return Err(DlError::Http(format!("server answered {s}"))),
    }

    file.seek(SeekFrom::Start(have))?;
    let reader = resp.into_body().into_reader();
    let mut since = 0usize;
    progress(have, bytes);
    let r = pump(reader, IDLE, cancel, |b| {
        if have + b.len() as u64 > bytes {
            return Err(DlError::Size { expected: bytes, got: have + b.len() as u64 });
        }
        file.write_all(b)?;
        have += b.len() as u64;
        since += b.len();
        if since >= CHUNK {
            since = 0;
            progress(have, bytes);
        }
        Ok(())
    });
    file.flush()?; // keep what arrived: the next try resumes from here
    r?;
    progress(have, bytes);
    Ok(())
}

/// A download that receives nothing for this long is stopped (resumable).
const IDLE: Duration = Duration::from_secs(60);

/// Read `r` on a helper thread and hand each piece to `sink`. A blocked
/// network read can neither be cancelled nor timed out from the outside, so the
/// reading happens elsewhere and this side waits in short steps: Cancel works
/// at once, and `idle` without data ends the download with [`DlError::Stalled`].
/// (A thread stuck in a dead connection ends when the OS gives up on it.)
fn pump<R: Read + Send + 'static>(
    mut r: R,
    idle: Duration,
    cancel: &AtomicBool,
    mut sink: impl FnMut(&[u8]) -> Result<(), DlError>,
) -> Result<(), DlError> {
    use std::sync::mpsc::{RecvTimeoutError, sync_channel};
    let (tx, rx) = sync_channel::<io::Result<Vec<u8>>>(4);
    std::thread::spawn(move || {
        loop {
            let mut buf = vec![0u8; CHUNK];
            let msg = match r.read(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    Ok(buf)
                }
                Err(e) => Err(e),
            };
            let end = !matches!(&msg, Ok(b) if !b.is_empty());
            if tx.send(msg).is_err() || end {
                return;
            }
        }
    });
    let mut last = std::time::Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(DlError::Cancelled);
        }
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(b)) if b.is_empty() => return Ok(()),
            Ok(Ok(b)) => {
                last = std::time::Instant::now();
                sink(&b)?;
            }
            Ok(Err(e)) => return Err(DlError::Http(e.to_string())),
            Err(RecvTimeoutError::Timeout) if last.elapsed() > idle => return Err(DlError::Stalled),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err(DlError::Http("the connection was lost".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gives some bytes, then hangs like a dead connection.
    struct Hang(usize);
    impl Read for Hang {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0 == 0 {
                std::thread::sleep(Duration::from_secs(30));
                return Ok(0);
            }
            let n = self.0.min(buf.len()).min(10);
            self.0 -= n;
            Ok(n)
        }
    }

    #[test]
    fn stalled_and_cancelled_downloads_do_not_hang() {
        let t = std::time::Instant::now();
        let mut got = 0;
        let r = pump(Hang(25), Duration::from_millis(700), &AtomicBool::new(false), |b| {
            got += b.len();
            Ok(())
        });
        assert!(matches!(r, Err(DlError::Stalled)) && got == 25, "{r:?}");
        assert!(t.elapsed() < Duration::from_secs(5));
        let t = std::time::Instant::now();
        let r = pump(Hang(0), Duration::from_secs(60), &AtomicBool::new(true), |_| Ok(()));
        assert!(matches!(r, Err(DlError::Cancelled)) && t.elapsed() < Duration::from_secs(2));
        let r = pump(&b"complete"[..], Duration::from_secs(5), &AtomicBool::new(false), |_| Ok(()));
        assert!(r.is_ok());
    }

    #[test]
    fn host_rules() {
        assert!(allowed_url("https://huggingface.co/opencv/x/resolve/abc/f.onnx"));
        assert!(!allowed_url("http://huggingface.co/x"));
        assert!(!allowed_url("https://huggingface.co.evil.com/x"));
        assert!(!allowed_url("https://evil.com/huggingface.co/x"));
        assert!(!allowed_url("https://user@huggingface.co/x"));
        assert!(!allowed_url("https://huggingface.co:8443/x"));
        assert!(!allowed_url("https://us.aws.cdn.hf.co/x")); // CDN only via redirect
        assert!(allowed_url(
            "https://github.com/GyanD/codexffmpeg/releases/download/9.1/ffmpeg-9.1-essentials_build.zip"
        ));
        assert!(!allowed_url("https://github.com/GyanD/other/releases/download/1/x.zip"), "only the two release feeds");
        assert!(!allowed_url("https://github.com/someone/codexffmpeg/releases/download/9.1/x.zip"));
        assert!(allowed_redirect("https://us.aws.cdn.hf.co/xet-bridge-us/abc?sig=1"));
        assert!(allowed_redirect("https://cas-bridge.xethub.hf.co/x"));
        assert!(allowed_redirect("https://cdn-lfs.huggingface.co/x"));
        assert!(!allowed_redirect("https://hf.co.evil.com/x"));
        assert!(!allowed_redirect("https://evilhf.co/x"));
        assert!(!allowed_redirect("http://us.aws.cdn.hf.co/x"));
        assert!(allowed_url("https://github.com/ggml-org/llama.cpp/releases/download/b10276/x.zip"));
        assert!(!allowed_url("https://github.com/someone/else/releases/download/b1/x.zip"));
        assert!(!allowed_url("https://github.com/ggml-org/llama.cpp/archive/main.zip"));
        assert!(allowed_redirect("https://release-assets.githubusercontent.com/github-production-release-asset/1/2"));
        assert!(!allowed_redirect("https://evil.githubusercontent.com/x"));
    }

    #[test]
    fn sha256_known_value() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("abc");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            sha256_file(&p, &|_, _| {}).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn refuses_unknown_hosts_before_any_network() {
        let d = tempfile::tempdir().unwrap();
        let r = fetch("https://example.com/x.gguf", &d.path().join("x"), 1, "00", &|_, _| {}, &AtomicBool::new(false));
        assert!(matches!(r, Err(DlError::NotAllowed(_))));
        assert!(!d.path().join("x.part").exists());
    }

    #[test]
    fn existing_correct_file_is_not_downloaded_again() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("abc");
        std::fs::write(&p, b"abc").unwrap();
        // Would fail on network (fake commit) if it tried to download.
        fetch(
            "https://huggingface.co/none/none/resolve/0000000000000000000000000000000000000000/abc",
            &p,
            3,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            &|_, _| {},
            &AtomicBool::new(false),
        )
        .unwrap();
    }

    /// Real download of the 227 KB YuNet model, then size + SHA-256 check.
    /// Needs internet, so it only runs when asked: CS_NET_TESTS=1 cargo test -p cs-ai
    #[test]
    fn real_download_small_model() {
        if std::env::var("CS_NET_TESTS").is_err() {
            return;
        }
        let cat = crate::catalogue();
        let f = &cat.packs.iter().find(|p| p.id == "faces-opencv").unwrap().files[0];
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join(&f.name);
        // Simulate an interrupted earlier attempt: first 1000 bytes of garbage-free prefix are unknown,
        // so start with an empty part file and a cancelled run, then resume.
        let cancel = AtomicBool::new(false);
        fetch(f.url.as_deref().unwrap(), &dest, f.bytes.unwrap(), f.sha256.as_deref().unwrap(), &|_, _| {}, &cancel)
            .unwrap();
        assert_eq!(std::fs::metadata(&dest).unwrap().len(), f.bytes.unwrap());
        assert!(!part_path(&dest).exists());

        // Resume path: keep the first half as a part file and fetch again.
        let all = std::fs::read(&dest).unwrap();
        std::fs::remove_file(&dest).unwrap();
        std::fs::write(part_path(&dest), &all[..all.len() / 2]).unwrap();
        fetch(f.url.as_deref().unwrap(), &dest, f.bytes.unwrap(), f.sha256.as_deref().unwrap(), &|_, _| {}, &cancel)
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), all);

        // A corrupted part file is caught by the checksum and deleted.
        std::fs::remove_file(&dest).unwrap();
        let mut bad = all.clone();
        bad[10] ^= 0xff;
        std::fs::write(part_path(&dest), &bad[..bad.len() - 100]).unwrap();
        let r = fetch(
            f.url.as_deref().unwrap(),
            &dest,
            f.bytes.unwrap(),
            f.sha256.as_deref().unwrap(),
            &|_, _| {},
            &cancel,
        );
        assert!(matches!(r, Err(DlError::Checksum)), "{r:?}");
        assert!(!part_path(&dest).exists() && !dest.exists());
    }
}
