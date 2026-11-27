//! Star rating (0–5) stored in a photo: XMP `xmp:Rating` first (Lightroom,
//! digiKam, Windows), then EXIF Rating (0x4746, Windows and some cameras).
//! Fast and read-only: JPEG reads only its header segments, other formats
//! a bounded start of the file; RAW files also look at a `.xmp` sidecar.

use cs_scan::Format;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// How much of a non-JPEG file is searched for an XMP packet.
const XMP_SCAN: u64 = 512 << 10;
/// TIFF-based files: EXIF IFD0 (where Rating lives) is at the start.
const TIFF_LIMIT: u64 = 2 << 20;
const XMP_ID: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

/// The photo's star rating: `Some(0)` = explicitly none, `None` = not found.
/// A "rejected" rating (-1) counts as 0.
pub fn rating(path: &Path, format: Format) -> Option<u8> {
    if let Some(r) = xmp_rating(path, format) {
        return Some(r);
    }
    if matches!(format, Format::Tiff | Format::Cr3)
        && let Some(r) = sidecar_rating(path)
    {
        return Some(r);
    }
    exif_rating(path, format)
}

fn clamp(r: f64) -> u8 {
    r.round().clamp(0.0, 5.0) as u8
}

/// `xmp:Rating="3"` or `<xmp:Rating>3</xmp:Rating>` (also the old `xap:` prefix).
/// Other ratings (e.g. MicrosoftPhoto:Rating, a percentage) are not this one.
pub fn find_xmp_rating(buf: &[u8]) -> Option<u8> {
    for key in [&b"xmp:Rating"[..], b"xap:Rating"] {
        let mut from = 0;
        while let Some(p) = find(&buf[from..], key) {
            let at = from + p;
            from = at + key.len();
            // Not part of a longer name (xmp:RatingX) or a closing tag (</xmp:Rating>).
            if at > 0 && buf[at - 1] == b'/' {
                continue;
            }
            let rest = &buf[from..];
            let rest = trim_start(rest);
            let (val, end) = match rest.first() {
                Some(b'=') => {
                    let r = trim_start(&rest[1..]);
                    let q = *r.first()?;
                    if q != b'"' && q != b'\'' {
                        continue;
                    }
                    (&r[1..], q)
                }
                Some(b'>') => (&rest[1..], b'<'),
                _ => continue,
            };
            let n = val.iter().take(16).position(|&c| c == end)?;
            let s = std::str::from_utf8(&val[..n]).ok()?.trim();
            if let Ok(v) = s.parse::<f64>() {
                return Some(clamp(v));
            }
        }
    }
    None
}

fn trim_start(b: &[u8]) -> &[u8] {
    let n = b.iter().take_while(|c| c.is_ascii_whitespace()).count();
    &b[n..]
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn xmp_rating(path: &Path, format: Format) -> Option<u8> {
    let mut f = File::open(path).ok()?;
    if format == Format::Jpeg {
        return jpeg_xmp(&mut f).as_deref().and_then(find_xmp_rating);
    }
    let mut buf = Vec::new();
    f.take(XMP_SCAN).read_to_end(&mut buf).ok()?;
    find_xmp_rating(&buf)
}

/// The XMP APP1 segment of a JPEG (header segments only, never the picture data).
fn jpeg_xmp(f: &mut File) -> Option<Vec<u8>> {
    let mut r = BufReader::with_capacity(16 << 10, f);
    let mut b2 = [0u8; 2];
    r.read_exact(&mut b2).ok()?;
    if b2 != [0xFF, 0xD8] {
        return None;
    }
    for _ in 0..64 {
        r.read_exact(&mut b2).ok()?;
        if b2[0] != 0xFF {
            return None;
        }
        let mut marker = b2[1];
        while marker == 0xFF {
            let mut b = [0u8; 1];
            r.read_exact(&mut b).ok()?;
            marker = b[0];
        }
        if marker == 0xDA || marker == 0xD9 {
            return None; // picture data starts: no XMP in the header
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            continue; // no length
        }
        r.read_exact(&mut b2).ok()?;
        let len = u16::from_be_bytes(b2) as usize;
        if len < 2 {
            return None;
        }
        let body = len - 2;
        if marker == 0xE1 && body > XMP_ID.len() {
            let mut data = vec![0u8; body];
            r.read_exact(&mut data).ok()?;
            if data.starts_with(XMP_ID) {
                return Some(data.split_off(XMP_ID.len()));
            }
        } else {
            r.seek(SeekFrom::Current(body as i64)).ok()?;
        }
    }
    None
}

/// `photo.xmp` or `photo.NEF.xmp` next to a RAW file.
fn sidecar_rating(path: &Path) -> Option<u8> {
    let mut with_ext = path.as_os_str().to_owned();
    with_ext.push(".xmp");
    for p in [path.with_extension("xmp"), with_ext.into()] {
        if let Ok(f) = File::open(&p) {
            let mut buf = Vec::new();
            if f.take(XMP_SCAN).read_to_end(&mut buf).is_ok()
                && let Some(r) = find_xmp_rating(&buf)
            {
                return Some(r);
            }
        }
    }
    None
}

fn exif_rating(path: &Path, format: Format) -> Option<u8> {
    use exif::{Context, In, Reader, Tag};
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    let mut f = File::open(path).ok()?;
    let parsed = if format == Format::Tiff {
        let mut buf = Vec::new();
        (&mut f).take(TIFF_LIMIT).read_to_end(&mut buf).ok()?;
        reader.read_raw(buf)
    } else {
        reader.read_from_container(&mut BufReader::new(f))
    };
    let e = match parsed {
        Ok(e) => e,
        Err(exif::Error::PartialResult(p)) => p.into_inner().0,
        Err(_) => return None,
    };
    let v = e.get_field(Tag(Context::Tiff, 0x4746), In::PRIMARY)?.value.get_uint(0)?;
    // Stored as SHORT, so "-1" (rejected) arrives as 65535.
    Some(if v > 5 { 0 } else { v as u8 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xmp_forms() {
        assert_eq!(find_xmp_rating(br#"<rdf:Description xmp:Rating="4" xmp:Label="x"/>"#), Some(4));
        assert_eq!(find_xmp_rating(b"<x xmp:Rating = '2' />"), Some(2));
        assert_eq!(find_xmp_rating(b"<xmp:Rating>5</xmp:Rating>"), Some(5));
        assert_eq!(find_xmp_rating(b"<xap:Rating>3.0</xap:Rating>"), Some(3));
        assert_eq!(find_xmp_rating(br#"xmp:Rating="-1""#), Some(0));
        assert_eq!(find_xmp_rating(br#"xmp:Rating="9""#), Some(5));
        // Windows' percentage is another tag.
        assert_eq!(find_xmp_rating(br#"MicrosoftPhoto:Rating="75""#), None);
        assert_eq!(find_xmp_rating(br#"xmp:RatingX="3" xmp:Rating="1""#), Some(1));
        assert_eq!(find_xmp_rating(b"xmp:Rating"), None);
        assert_eq!(find_xmp_rating(br#"xmp:Rating="abc""#), None);
        assert_eq!(find_xmp_rating(b""), None);
    }

    fn jpeg_with(segments: &[(u8, &[u8])]) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8];
        for (m, body) in segments {
            v.extend([0xFF, *m]);
            v.extend(((body.len() + 2) as u16).to_be_bytes());
            v.extend(*body);
        }
        v.extend([0xFF, 0xDA, 0, 2, 0xFF, 0xD9]);
        v
    }

    #[test]
    fn jpeg_header_segments() {
        let d = tempfile::tempdir().unwrap();
        let mut xmp = XMP_ID.to_vec();
        xmp.extend(br#"<x:xmpmeta><rdf:Description xmp:Rating="3"/></x:xmpmeta>"#);
        let p = d.path().join("a.jpg");
        std::fs::write(&p, jpeg_with(&[(0xE0, b"JFIF\0\x01\x01"), (0xE1, &xmp)])).unwrap();
        assert_eq!(rating(&p, Format::Jpeg), Some(3));
        // No rating anywhere.
        std::fs::write(&p, jpeg_with(&[(0xE0, b"JFIF\0\x01\x01")])).unwrap();
        assert_eq!(rating(&p, Format::Jpeg), None);
        // A rating text after the picture data starts is not read.
        let mut v = jpeg_with(&[]);
        v.extend(br#"xmp:Rating="5""#);
        std::fs::write(&p, v).unwrap();
        assert_eq!(rating(&p, Format::Jpeg), None);
        // Broken files are fine.
        std::fs::write(&p, [0xFF, 0xD8, 0xFF, 0xE1, 0x00]).unwrap();
        assert_eq!(rating(&p, Format::Jpeg), None);
    }

    #[test]
    fn raw_sidecar() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("b.nef");
        std::fs::write(&p, b"II*\0not really").unwrap();
        assert_eq!(rating(&p, Format::Tiff), None);
        std::fs::write(d.path().join("b.xmp"), br#"<rdf:Description xmp:Rating="2"/>"#).unwrap();
        assert_eq!(rating(&p, Format::Tiff), Some(2));
    }
}
