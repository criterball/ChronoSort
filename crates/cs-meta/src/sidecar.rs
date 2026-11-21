//! XMP sidecars and Google Takeout JSON.

use cs_model::{PartialDate, Precision};
use quick_xml::events::Event;

/// XMP properties that carry a capture date (local name, any namespace prefix).
const XMP_DATE_PROPS: &[&str] = &["DateTimeOriginal", "DateCreated", "CreateDate"];

/// ISO-8601-ish XMP date: "1999", "1999-08", "1999-08-15", "1999-08-15T10:00[:00][.sss][Z|±hh:mm]".
pub fn parse_xmp_date(s: &str) -> Option<PartialDate> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b).filter(|t| t.bytes().all(|c| c.is_ascii_digit()))?.parse::<u32>().ok();
    let year = num(0, 4)? as i16;
    let mut d = PartialDate::year(year);
    if s.len() >= 7 {
        d.month = Some(num(5, 7)? as u8);
        d.precision = Precision::Month;
    }
    if s.len() >= 10 {
        d.day = Some(num(8, 10)? as u8);
        d.precision = Precision::Day;
    }
    if s.len() >= 16 && s.as_bytes()[10] == b'T' {
        let sec = num(17, 19).unwrap_or(0);
        d.time = Some((num(11, 13)? as u8, num(14, 16)? as u8, sec as u8));
        d.precision = Precision::Second;
        let tail = &s[16..];
        if let Some(i) = tail.find(['Z', '+', '-']) {
            d.utc_offset_min = crate::parse_offset(&tail[i..]);
        }
    }
    d.is_plausible().then_some(d)
}

/// The XMP packet inside a JPEG (APP1 "http://ns.adobe.com/xap/1.0/"), if any.
/// Walks the marker segments up to the image data; at most 64 segments.
pub fn jpeg_xmp<R: std::io::Read>(r: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    const SIG: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
    let mut soi = [0u8; 2];
    if r.read_exact(&mut soi).is_err() || soi != [0xFF, 0xD8] {
        return Ok(None);
    }
    for _ in 0..64 {
        let mut m = [0u8; 4];
        if r.read_exact(&mut m).is_err() || m[0] != 0xFF {
            return Ok(None);
        }
        if m[1] == 0xDA || m[1] == 0xD9 {
            return Ok(None); // image data / end: no more metadata
        }
        let len = u16::from_be_bytes([m[2], m[3]]) as usize;
        if len < 2 {
            return Ok(None);
        }
        let mut seg = vec![0u8; len - 2];
        r.read_exact(&mut seg)?;
        if m[1] == 0xE1 && seg.starts_with(SIG) {
            return Ok(Some(seg[SIG.len()..].to_vec()));
        }
    }
    Ok(None)
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn is_date_prop(name: &str) -> bool {
    XMP_DATE_PROPS.contains(&local(name))
}

/// Collect capture dates from an XMP packet. Handles both the attribute form
/// (`xmp:CreateDate="…"`) and the element form (`<xmp:CreateDate>…</…>`).
/// External entities are never resolved (quick-xml does not fetch anything).
pub fn xmp_dates(xml: &[u8]) -> Vec<PartialDate> {
    let mut out = Vec::new();
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut in_prop = false;
    for _ in 0..100_000 {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = e.name();
                in_prop = is_date_prop(name.as_ref());
                for a in e.attributes().flatten() {
                    if is_date_prop(a.key.as_ref())
                        && let Some(d) = parse_xmp_date(&a.value)
                    {
                        out.push(d);
                    }
                }
            }
            Ok(Event::Text(t)) if in_prop => {
                if let Some(d) = parse_xmp_date(t.as_ref()) {
                    out.push(d);
                }
            }
            Ok(Event::End(_)) => in_prop = false,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Google Takeout: `photoTakenTime.timestamp` (string or number, UTC seconds).
/// `creationTime` is the *upload* date and is deliberately ignored.
pub fn takeout_taken(json: &[u8]) -> Option<i64> {
    let v: serde_json::Value = serde_json::from_slice(json).ok()?;
    let ts = v.get("photoTakenTime")?.get("timestamp")?;
    let t = match ts {
        serde_json::Value::String(s) => s.trim().parse().ok()?,
        serde_json::Value::Number(n) => n.as_i64()?,
        _ => return None,
    };
    (t > 0).then_some(t) // 0 = "unknown" in some exports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xmp_inside_a_jpeg() {
        let xmp =
            br#"<x:xmpmeta><rdf:RDF><rdf:Description xmp:CreateDate="1987-06-12T10:20:00"/></rdf:RDF></x:xmpmeta>"#;
        let mut payload = b"http://ns.adobe.com/xap/1.0/".to_vec();
        payload.push(0);
        payload.extend_from_slice(xmp);
        let mut j = vec![0xFF, 0xD8];
        // An EXIF APP1 first (must be skipped), then the XMP APP1, then image data.
        j.extend([0xFF, 0xE1, 0x00, 0x08]);
        j.extend(b"Exif\0\0");
        j.extend([0xFF, 0xE1]);
        j.extend(((payload.len() + 2) as u16).to_be_bytes());
        j.extend(&payload);
        j.extend([0xFF, 0xDA, 0x00, 0x02]);
        let got = jpeg_xmp(&mut &j[..]).unwrap().unwrap();
        let d = xmp_dates(&got);
        assert_eq!((d[0].year, d[0].month, d[0].day), (1987, Some(6), Some(12)));
        assert!(jpeg_xmp(&mut &b"not a jpeg"[..]).unwrap().is_none());
        assert!(jpeg_xmp(&mut &[0xFF, 0xD8, 0xFF, 0xE1, 0xFF][..]).is_ok(), "truncated input is not a panic");
    }

    #[test]
    fn xmp_forms() {
        let attr = br#"<x:xmpmeta><rdf:RDF><rdf:Description xmp:CreateDate="1999-08-15T10:00:00+02:00"/></rdf:RDF></x:xmpmeta>"#;
        let d = xmp_dates(attr);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].utc_offset_min, Some(120));
        let elem = br#"<x:xmpmeta><rdf:Description><photoshop:DateCreated>1987-06</photoshop:DateCreated></rdf:Description></x:xmpmeta>"#;
        let d = xmp_dates(elem);
        assert_eq!((d[0].year, d[0].month, d[0].precision), (1987, Some(6), Precision::Month));
        assert!(xmp_dates(b"<<<not xml").is_empty());
    }

    #[test]
    fn takeout() {
        assert_eq!(takeout_taken(br#"{"photoTakenTime":{"timestamp":"1467000000"}}"#), Some(1_467_000_000));
        assert_eq!(takeout_taken(br#"{"photoTakenTime":{"timestamp":0}}"#), None);
        assert_eq!(takeout_taken(br#"{"creationTime":{"timestamp":"1600000000"}}"#), None);
        assert_eq!(takeout_taken(b"not json"), None);
    }
}
