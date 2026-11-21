//! Unpack a verified program archive (e.g. llama.cpp) into ChronoSort's own
//! tools folder.
//!
//! The archive's SHA-256 has already been checked against the catalogue, but
//! extraction is still defensive: every entry must stay inside the target
//! (no `..`, no absolute paths, no symlinks), with limits on entry count and
//! total size, and the result only appears under its final name once complete.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

pub const MAX_ENTRIES: usize = 5_000;
pub const MAX_TOTAL: u64 = 4 << 30;

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Extract `zip` into `dest` (which must not exist yet). On any error nothing
/// is left at `dest` (a `dest.tmp` folder may remain and is replaced next time).
pub fn extract_zip(zip: &Path, dest: &Path) -> io::Result<()> {
    if dest.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists", dest.display())));
    }
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir_all(&tmp)?;

    let mut ar = zip::ZipArchive::new(File::open(zip)?).map_err(|e| bad(e.to_string()))?;
    if ar.len() > MAX_ENTRIES {
        return Err(bad("the archive has too many entries"));
    }
    let mut total = 0u64;
    let mut buf = vec![0u8; 1 << 16];
    for i in 0..ar.len() {
        let mut e = ar.by_index(i).map_err(|e| bad(e.to_string()))?;
        if e.is_symlink() {
            return Err(bad(format!("refusing symlink in archive: {}", e.name().unwrap_or_default())));
        }
        // enclosed_name() rejects absolute paths and anything that escapes with "..".
        let rel = e
            .enclosed_name()
            .ok_or_else(|| bad(format!("unsafe path in archive: {}", e.name().unwrap_or_default())))?;
        let out = tmp.join(&rel);
        if e.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = File::create_new(&out)?;
        let mut written = 0u64;
        loop {
            let n = e.read(&mut buf)?;
            if n == 0 {
                break;
            }
            written += n as u64;
            total += n as u64;
            // Never trust the header's size: count real bytes.
            if total > MAX_TOTAL || written > e.size().saturating_add(1) {
                drop(f);
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(bad("the archive unpacks to more data than it claims"));
            }
            f.write_all(&buf[..n])?;
        }
        f.sync_all()?;
    }
    std::fs::rename(&tmp, dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, data) in entries {
            w.start_file(*name, SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn extracts_normal_archive() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("a.zip");
        make_zip(&z, &[("llama-server.exe", b"MZ.."), ("sub/ggml.dll", b"dll")]);
        let dest = d.path().join("out");
        extract_zip(&z, &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("llama-server.exe")).unwrap(), b"MZ..");
        assert_eq!(std::fs::read(dest.join("sub/ggml.dll")).unwrap(), b"dll");
        assert!(!d.path().join("out.tmp").exists());
    }

    #[test]
    fn refuses_path_traversal() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("evil.zip");
        make_zip(&z, &[("ok.txt", b"1"), ("../escaped.txt", b"2")]);
        let dest = d.path().join("inner").join("out");
        assert!(extract_zip(&z, &dest).is_err());
        assert!(!dest.exists());
        assert!(!d.path().join("inner").join("escaped.txt").exists());
    }

    #[test]
    fn refuses_existing_destination() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("a.zip");
        make_zip(&z, &[("x", b"1")]);
        std::fs::create_dir(d.path().join("out")).unwrap();
        assert!(extract_zip(&z, &d.path().join("out")).is_err());
    }
}
