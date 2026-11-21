//! EXIF via kamadak-exif (reviewed: third_party/kamadak-exif.md).

use crate::{ExifInfo, parse_exif_datetime, parse_offset};
use cs_scan::Format;
use exif::{In, Reader, Tag, Value};
use std::fs::File;
use std::io::{BufReader, Read};

fn ascii(e: &exif::Exif, tag: Tag) -> Option<String> {
    let f = e.get_field(tag, In::PRIMARY)?;
    match &f.value {
        Value::Ascii(v) => {
            let s = String::from_utf8_lossy(v.first()?).trim().trim_end_matches('\0').trim().to_string();
            (!s.is_empty()).then_some(s)
        }
        _ => None,
    }
}

pub fn read(file: &mut File, format: Format, tiff_limit: u64) -> Result<Option<ExifInfo>, exif::Error> {
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    let parsed = if format == Format::Tiff {
        // Bound memory: kamadak reads whole TIFF files; give it the first N MB only.
        let mut buf = Vec::new();
        file.take(tiff_limit).read_to_end(&mut buf)?;
        reader.read_raw(buf)
    } else {
        reader.read_from_container(&mut BufReader::new(file))
    };
    let e = match parsed {
        Ok(e) => e,
        Err(exif::Error::NotFound(_)) => return Ok(None),
        Err(exif::Error::PartialResult(p)) => p.into_inner().0,
        Err(e) => return Err(e),
    };
    let with_offset = |dt: Tag, off: Tag| {
        let mut d = parse_exif_datetime(&ascii(&e, dt)?)?;
        d.utc_offset_min = ascii(&e, off).as_deref().and_then(parse_offset);
        Some(d)
    };
    let info = ExifInfo {
        make: ascii(&e, Tag::Make),
        model: ascii(&e, Tag::Model),
        software: ascii(&e, Tag::Software),
        original: with_offset(Tag::DateTimeOriginal, Tag::OffsetTimeOriginal),
        digitized: with_offset(Tag::DateTimeDigitized, Tag::OffsetTimeDigitized),
        modify: with_offset(Tag::DateTime, Tag::OffsetTime),
    };
    Ok((info != ExifInfo::default()).then_some(info))
}

/// EXIF orientation (1–8) and the embedded JPEG thumbnail (IFD1), if any.
/// Used for thumbnails of formats we cannot decode ourselves (RAW, HEIC).
pub fn orientation_and_thumbnail(file: &mut File, format: Format, tiff_limit: u64) -> (Option<u16>, Option<Vec<u8>>) {
    let mut reader = Reader::new();
    reader.continue_on_error(true);
    let parsed = if format == Format::Tiff {
        let mut buf = Vec::new();
        if file.take(tiff_limit).read_to_end(&mut buf).is_err() {
            return (None, None);
        }
        reader.read_raw(buf)
    } else {
        reader.read_from_container(&mut BufReader::new(file))
    };
    let e = match parsed {
        Ok(e) => e,
        Err(exif::Error::PartialResult(p)) => p.into_inner().0,
        Err(_) => return (None, None),
    };
    let uint = |tag: Tag, ifd: In| e.get_field(tag, ifd).and_then(|f| f.value.get_uint(0));
    let orientation =
        uint(Tag::Orientation, In::PRIMARY).and_then(|v| u16::try_from(v).ok()).filter(|v| (1..=8).contains(v));
    let thumb = (|| {
        let off = uint(Tag::JPEGInterchangeFormat, In::THUMBNAIL)? as usize;
        let len = uint(Tag::JPEGInterchangeFormatLength, In::THUMBNAIL)? as usize;
        let data = e.buf().get(off..off.checked_add(len)?)?;
        data.starts_with(&[0xFF, 0xD8]).then(|| data.to_vec())
    })();
    (orientation, thumb)
}
