//! Minimal little-endian TIFF/EXIF writer, just enough to produce test files.
//! Written by hand so the bytes are fully under our control (and so the same
//! code can produce deliberately broken structures).

pub const TAG_MAKE: u16 = 0x010F;
pub const TAG_MODEL: u16 = 0x0110;
pub const TAG_DATETIME: u16 = 0x0132;
pub const TAG_EXIF_IFD: u16 = 0x8769;
pub const TAG_DATETIME_ORIGINAL: u16 = 0x9003;
pub const TAG_DATETIME_DIGITIZED: u16 = 0x9004;
pub const TAG_OFFSET_TIME_ORIGINAL: u16 = 0x9011;

#[derive(Debug, Clone)]
pub enum Val {
    Ascii(String),
    Long(u32),
}

impl Val {
    fn type_count(&self) -> (u16, u32) {
        match self {
            Val::Ascii(s) => (2, s.len() as u32 + 1),
            Val::Long(_) => (4, 1),
        }
    }
    fn bytes(&self) -> Vec<u8> {
        match self {
            Val::Ascii(s) => {
                let mut b = s.as_bytes().to_vec();
                b.push(0);
                b
            }
            Val::Long(v) => v.to_le_bytes().to_vec(),
        }
    }
}

/// Serialise one IFD (+ its out-of-line data) to be placed at file offset `at`.
fn ifd_bytes(entries: &[(u16, Val)], at: u32, next_ifd: u32) -> Vec<u8> {
    let mut entries = entries.to_vec();
    entries.sort_by_key(|e| e.0);
    let n = entries.len();
    let table_len = 2 + 12 * n + 4;
    let mut table = Vec::with_capacity(table_len);
    let mut data = Vec::new();
    table.extend((n as u16).to_le_bytes());
    for (tag, val) in &entries {
        let (ty, count) = val.type_count();
        let b = val.bytes();
        table.extend(tag.to_le_bytes());
        table.extend(ty.to_le_bytes());
        table.extend(count.to_le_bytes());
        if b.len() <= 4 {
            let mut inline = [0u8; 4];
            inline[..b.len()].copy_from_slice(&b);
            table.extend(inline);
        } else {
            let off = at + table_len as u32 + data.len() as u32;
            table.extend(off.to_le_bytes());
            data.extend(&b);
            if data.len() % 2 == 1 {
                data.push(0); // word alignment
            }
        }
    }
    table.extend(next_ifd.to_le_bytes());
    table.extend(data);
    table
}

/// Build a TIFF stream: IFD0 (+ an Exif sub-IFD if `exif` is not empty).
pub fn build(ifd0: &[(u16, Val)], exif: &[(u16, Val)]) -> Vec<u8> {
    let mut out = b"II*\0".to_vec();
    out.extend(8u32.to_le_bytes());
    let mut ifd0 = ifd0.to_vec();
    if exif.is_empty() {
        out.extend(ifd_bytes(&ifd0, 8, 0));
        return out;
    }
    // Length does not depend on the pointer value, so measure with a placeholder.
    ifd0.push((TAG_EXIF_IFD, Val::Long(0)));
    let len0 = ifd_bytes(&ifd0, 8, 0).len() as u32;
    let mut exif_at = 8 + len0;
    exif_at += exif_at % 2;
    ifd0.last_mut().unwrap().1 = Val::Long(exif_at);
    out.extend(ifd_bytes(&ifd0, 8, 0));
    out.resize(exif_at as usize, 0);
    out.extend(ifd_bytes(exif, exif_at, 0));
    out
}

/// Hostile TIFF: a string with an absurd length pointing outside the file, an
/// Exif pointer to itself, and a next-IFD loop. A parser must reject this
/// quickly without hanging or allocating gigabytes.
pub fn build_malicious() -> Vec<u8> {
    let mut out = b"II*\0".to_vec();
    out.extend(8u32.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    // Make: ASCII, count 0xFFFF_FFF0, offset far out of bounds
    out.extend(TAG_MAKE.to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(0xFFFF_FFF0u32.to_le_bytes());
    out.extend(0x7FFF_FFF0u32.to_le_bytes());
    // ExifIFD -> 8 (itself)
    out.extend(TAG_EXIF_IFD.to_le_bytes());
    out.extend(4u16.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(8u32.to_le_bytes());
    // next IFD -> 8 (loop)
    out.extend(8u32.to_le_bytes());
    out
}

/// Wrap a TIFF stream as a JPEG APP1 "Exif" segment and insert it right after SOI.
pub fn insert_into_jpeg(jpeg: &[u8], tiff: &[u8]) -> Vec<u8> {
    assert!(jpeg.starts_with(&[0xFF, 0xD8]), "not a JPEG");
    let seg_len = 2 + 6 + tiff.len();
    assert!(seg_len <= 0xFFFF, "EXIF too large for one APP1");
    let mut out = Vec::with_capacity(jpeg.len() + seg_len + 2);
    out.extend(&jpeg[..2]);
    out.extend([0xFF, 0xE1]);
    out.extend((seg_len as u16).to_be_bytes());
    out.extend(b"Exif\0\0");
    out.extend(tiff);
    out.extend(&jpeg[2..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16le(b: &[u8], o: usize) -> u16 {
        u16::from_le_bytes([b[o], b[o + 1]])
    }
    fn u32le(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }

    #[test]
    fn layout_is_readable() {
        let t = build(
            &[(TAG_MAKE, Val::Ascii("Canon".into())), (TAG_MODEL, Val::Ascii("EOS 300D".into()))],
            &[(TAG_DATETIME_ORIGINAL, Val::Ascii("2005:03:12 14:22:07".into()))],
        );
        assert_eq!(&t[..4], b"II*\0");
        let ifd0 = u32le(&t, 4) as usize;
        let n = u16le(&t, ifd0) as usize;
        assert_eq!(n, 3);
        // entries sorted; the last one is the Exif pointer
        let e = ifd0 + 2 + 12 * 2;
        assert_eq!(u16le(&t, e), TAG_EXIF_IFD);
        let exif = u32le(&t, e + 8) as usize;
        assert_eq!(exif % 2, 0);
        assert_eq!(u16le(&t, exif + 2), TAG_DATETIME_ORIGINAL);
        let off = u32le(&t, exif + 2 + 8) as usize;
        assert_eq!(&t[off..off + 19], b"2005:03:12 14:22:07");
        // "Canon\0" is 6 bytes -> out of line
        let make_off = u32le(&t, ifd0 + 2 + 8) as usize;
        assert_eq!(&t[make_off..make_off + 6], b"Canon\0");
    }
}
