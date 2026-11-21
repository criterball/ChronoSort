//! Structural integrity checks. A damaged file is still copied (preservation
//! comes first), but it is flagged and always sent to review.

use cs_scan::Format;
use std::io::{self, Read, Seek, SeekFrom};

const MAX_SEGMENTS: usize = 1024;

pub fn check<R: Read + Seek>(r: &mut R, format: Format, len: u64) -> io::Result<Option<String>> {
    Ok(match format {
        Format::Jpeg => jpeg(r, len)?,
        Format::Png => png(r, len)?,
        _ => None,
    })
}

/// Walk marker segments to Start-Of-Scan, then require an End-Of-Image
/// (FF D9) somewhere after it. Truncated downloads/copies fail this.
fn jpeg<R: Read + Seek>(r: &mut R, len: u64) -> io::Result<Option<String>> {
    let mut pos = 2u64;
    let mut sos = None;
    for _ in 0..MAX_SEGMENTS {
        if pos + 4 > len {
            break;
        }
        r.seek(SeekFrom::Start(pos))?;
        let mut m = [0u8; 4];
        r.read_exact(&mut m)?;
        if m[0] != 0xFF {
            return Ok(Some("damaged: invalid JPEG marker structure".into()));
        }
        if m[1] == 0xDA {
            sos = Some(pos);
            break;
        }
        let seg = u16::from_be_bytes([m[2], m[3]]) as u64;
        if seg < 2 {
            return Ok(Some("damaged: invalid JPEG segment length".into()));
        }
        pos += 2 + seg;
    }
    let Some(sos) = sos else { return Ok(Some("damaged: JPEG has no image data".into())) };
    // Search the tail (up to 4 MiB) backwards for FF D9 after SOS.
    let tail_start = sos.max(len.saturating_sub(4 << 20));
    r.seek(SeekFrom::Start(tail_start))?;
    let mut tail = Vec::new();
    r.take(len - tail_start).read_to_end(&mut tail)?;
    let has_eoi = tail.windows(2).any(|w| w == [0xFF, 0xD9]);
    Ok((!has_eoi).then(|| "damaged: JPEG is truncated (no end-of-image marker)".into()))
}

fn png<R: Read + Seek>(r: &mut R, len: u64) -> io::Result<Option<String>> {
    if len < 20 {
        return Ok(Some("damaged: PNG too short".into()));
    }
    let tail_len = len.min(4096);
    r.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = Vec::new();
    r.take(tail_len).read_to_end(&mut tail)?;
    let ok = tail.windows(4).any(|w| w == b"IEND");
    Ok((!ok).then(|| "damaged: PNG is truncated (no IEND chunk)".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn jpeg_bytes(with_eoi: bool) -> Vec<u8> {
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0, 0, 0xFF, 0xDA, 0x00, 0x02, 1, 2, 3];
        if with_eoi {
            j.extend([0xFF, 0xD9]);
        }
        j
    }

    #[test]
    fn jpeg_checks() {
        let ok = jpeg_bytes(true);
        assert_eq!(check(&mut Cursor::new(&ok), Format::Jpeg, ok.len() as u64).unwrap(), None);
        let bad = jpeg_bytes(false);
        assert!(check(&mut Cursor::new(&bad), Format::Jpeg, bad.len() as u64).unwrap().is_some());
        let zero = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0];
        assert!(check(&mut Cursor::new(&zero), Format::Jpeg, zero.len() as u64).unwrap().is_some());
    }
}
