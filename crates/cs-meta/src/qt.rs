//! Bounded ISO-BMFF (MP4/MOV/3GP/CR3) walker: finds `moov/mvhd` creation time.

use std::io::{self, Read, Seek, SeekFrom};

/// Seconds between 1904-01-01 (QuickTime epoch) and 1970-01-01.
const MAC_EPOCH_OFFSET: i64 = 2_082_844_800;
const MAX_BOXES: usize = 4096;

struct BoxHeader {
    kind: [u8; 4],
    /// Payload start / end (absolute file offsets).
    start: u64,
    end: u64,
}

fn header<R: Read + Seek>(r: &mut R, at: u64, limit: u64) -> io::Result<Option<BoxHeader>> {
    if limit.saturating_sub(at) < 8 {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(at))?;
    let mut h = [0u8; 8];
    r.read_exact(&mut h)?;
    let size32 = u32::from_be_bytes(h[..4].try_into().unwrap()) as u64;
    let kind: [u8; 4] = h[4..8].try_into().unwrap();
    let (size, hdr) = match size32 {
        0 => (limit - at, 8),
        1 => {
            let mut l = [0u8; 8];
            r.read_exact(&mut l)?;
            (u64::from_be_bytes(l), 16)
        }
        s => (s, 8),
    };
    if size < hdr || at.checked_add(size).is_none_or(|e| e > limit) {
        return Ok(None); // inconsistent: stop, never guess
    }
    Ok(Some(BoxHeader { kind, start: at + hdr, end: at + size }))
}

/// Find a child box of `kind` between `from` and `to`.
fn find<R: Read + Seek>(
    r: &mut R,
    mut at: u64,
    to: u64,
    kind: &[u8; 4],
    budget: &mut usize,
) -> io::Result<Option<BoxHeader>> {
    while let Some(b) = header(r, at, to)? {
        *budget = budget.checked_sub(1).ok_or_else(|| io::Error::other("too many boxes"))?;
        if &b.kind == kind {
            return Ok(Some(b));
        }
        at = b.end;
    }
    Ok(None)
}

/// `mvhd` creation_time as unix seconds (UTC). `None` if absent or zero.
pub fn mvhd_created_utc<R: Read + Seek>(r: &mut R, file_len: u64) -> io::Result<Option<i64>> {
    let mut budget = MAX_BOXES;
    let Some(moov) = find(r, 0, file_len, b"moov", &mut budget)? else { return Ok(None) };
    let Some(mvhd) = find(r, moov.start, moov.end, b"mvhd", &mut budget)? else { return Ok(None) };
    r.seek(SeekFrom::Start(mvhd.start))?;
    let mut vf = [0u8; 4];
    r.read_exact(&mut vf)?;
    let t = match vf[0] {
        0 if mvhd.end - mvhd.start >= 8 => {
            let mut b = [0u8; 4];
            r.read_exact(&mut b)?;
            u32::from_be_bytes(b) as i64
        }
        1 if mvhd.end - mvhd.start >= 12 => {
            let mut b = [0u8; 8];
            r.read_exact(&mut b)?;
            i64::try_from(u64::from_be_bytes(b)).unwrap_or(0)
        }
        _ => return Ok(None),
    };
    // 0 means "not set" (many encoders); values before 1970 are bogus too.
    Ok((t > MAC_EPOCH_OFFSET).then_some(t - MAC_EPOCH_OFFSET))
}

/// Apple's `com.apple.quicktime.creationdate` (iPhone/iPad videos): the
/// capture time in *local* time with its UTC offset, e.g.
/// "2019-07-14T16:41:30+0200". Read from `moov/meta` (QuickTime style) or
/// `moov/udta/meta` (ISO style, with version/flags). Bounded like the rest.
pub fn apple_creationdate<R: Read + Seek>(r: &mut R, file_len: u64) -> io::Result<Option<String>> {
    const KEY: &[u8] = b"com.apple.quicktime.creationdate";
    let mut budget = MAX_BOXES;
    let Some(moov) = find(r, 0, file_len, b"moov", &mut budget)? else { return Ok(None) };
    let mut metas = vec![];
    if let Some(m) = find(r, moov.start, moov.end, b"meta", &mut budget)? {
        metas.push(m);
    }
    if let Some(u) = find(r, moov.start, moov.end, b"udta", &mut budget)?
        && let Some(m) = find(r, u.start, u.end, b"meta", &mut budget)?
    {
        metas.push(m);
    }
    for meta in metas {
        // QuickTime `meta` has children right away; ISO `meta` first has 4 bytes of version/flags.
        for start in [meta.start, meta.start + 4] {
            let (Some(keys), Some(ilst)) =
                (find(r, start, meta.end, b"keys", &mut budget)?, find(r, start, meta.end, b"ilst", &mut budget)?)
            else {
                continue;
            };
            // keys: version/flags, count, then [size][namespace][name] entries (1-based index).
            r.seek(SeekFrom::Start(keys.start + 4))?;
            let mut n = [0u8; 4];
            r.read_exact(&mut n)?;
            let count = u32::from_be_bytes(n).min(512);
            let mut at = keys.start + 8;
            let mut wanted = None;
            for i in 1..=count {
                let Some(k) = header(r, at, keys.end)? else { break };
                let len = (k.end - k.start) as usize;
                if len == KEY.len() {
                    let mut name = vec![0u8; len];
                    r.seek(SeekFrom::Start(k.start))?;
                    r.read_exact(&mut name)?;
                    if name == KEY {
                        wanted = Some(i);
                        break;
                    }
                }
                at = k.end;
            }
            let Some(idx) = wanted else { continue };
            // ilst: items whose box type is the key index; value in a `data` box (type, locale, bytes).
            let mut at = ilst.start;
            while let Some(item) = header(r, at, ilst.end)? {
                budget = budget.checked_sub(1).ok_or_else(|| io::Error::other("too many boxes"))?;
                if u32::from_be_bytes(item.kind) == idx
                    && let Some(d) = find(r, item.start, item.end, b"data", &mut budget)?
                    && d.end - d.start > 8
                    && d.end - d.start <= 8 + 64
                {
                    let mut v = vec![0u8; (d.end - d.start - 8) as usize];
                    r.seek(SeekFrom::Start(d.start + 8))?;
                    r.read_exact(&mut v)?;
                    return Ok(String::from_utf8(v).ok().map(|s| s.trim_end_matches('\0').trim().to_string()));
                }
                at = item.end;
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut b = ((8 + payload.len()) as u32).to_be_bytes().to_vec();
        b.extend(kind);
        b.extend(payload);
        b
    }

    fn iphone_movie(iso: bool) -> Vec<u8> {
        let key = b"com.apple.quicktime.creationdate";
        let mut keys = vec![0u8; 4];
        keys.extend(2u32.to_be_bytes());
        keys.extend(bx(b"mdta", b"com.apple.quicktime.make"));
        keys.extend(bx(b"mdta", key));
        let mut data = vec![0, 0, 0, 1, 0, 0, 0, 0];
        data.extend(b"2019-07-14T16:41:30+0200");
        let mut ilst = bx(&1u32.to_be_bytes(), &bx(b"data", &[0, 0, 0, 1, 0, 0, 0, 0, b'A']));
        ilst.extend(bx(&2u32.to_be_bytes(), &bx(b"data", &data)));
        let mut meta = if iso { vec![0u8; 4] } else { vec![] };
        meta.extend(bx(b"hdlr", &[0u8; 24]));
        meta.extend(bx(b"keys", &keys));
        meta.extend(bx(b"ilst", &ilst));
        let meta = bx(b"meta", &meta);
        let inner = if iso { bx(b"udta", &meta) } else { meta };
        let mut moov = bx(b"mvhd", &[0u8; 100]);
        moov.extend(inner);
        let mut f = bx(b"ftyp", b"qt  ");
        f.extend(bx(b"moov", &moov));
        f
    }

    #[test]
    fn reads_apple_creationdate_both_layouts() {
        for iso in [false, true] {
            let f = iphone_movie(iso);
            let n = f.len() as u64;
            assert_eq!(
                apple_creationdate(&mut Cursor::new(f), n).unwrap().as_deref(),
                Some("2019-07-14T16:41:30+0200"),
                "iso={iso}"
            );
        }
    }

    #[test]
    fn creationdate_survives_garbage() {
        let mut f = iphone_movie(false);
        let n = f.len();
        for i in (40..n).step_by(7) {
            f[i] ^= 0x5a; // corrupt bytes all over: must never panic or loop
            let _ = apple_creationdate(&mut Cursor::new(f.clone()), n as u64);
        }
    }

    #[test]
    fn reads_v0() {
        let t = (1_341_424_800i64 + MAC_EPOCH_OFFSET) as u32; // 2012-07-04 18:00 UTC
        let mut mvhd = vec![0u8; 4];
        mvhd.extend(t.to_be_bytes());
        mvhd.extend([0u8; 92]);
        let mut f = bx(b"ftyp", b"isom\0\0\0\0");
        f.extend(bx(b"moov", &bx(b"mvhd", &mvhd)));
        let n = f.len() as u64;
        assert_eq!(mvhd_created_utc(&mut Cursor::new(f), n).unwrap(), Some(1_341_424_800));
    }

    #[test]
    fn hostile_sizes_do_not_loop_or_panic() {
        // box claims to be larger than the file
        let mut f = 0xFFFF_FFF0u32.to_be_bytes().to_vec();
        f.extend(b"moov");
        let n = f.len() as u64;
        assert_eq!(mvhd_created_utc(&mut Cursor::new(f), n).unwrap(), None);
        // size 4 (< header) must not loop forever
        let mut f = 4u32.to_be_bytes().to_vec();
        f.extend(b"free\0\0\0\0");
        let n = f.len() as u64;
        assert_eq!(mvhd_created_utc(&mut Cursor::new(f), n).unwrap(), None);
        // zero creation time = unknown
        let mut f = bx(b"moov", &bx(b"mvhd", &[0u8; 100]));
        f.truncate(f.len());
        let n = f.len() as u64;
        assert_eq!(mvhd_created_utc(&mut Cursor::new(f), n).unwrap(), None);
    }
}
