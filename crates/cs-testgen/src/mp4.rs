//! Minimal ISO-BMFF (MP4 / QuickTime) file with a `mvhd` creation time.
//! Phones and cameras write this time in **UTC**, which is the classic trap.

/// Seconds between 1904-01-01 (QuickTime epoch) and 1970-01-01.
const MAC_EPOCH_OFFSET: i64 = 2_082_844_800;

fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(8 + payload.len());
    b.extend(((8 + payload.len()) as u32).to_be_bytes());
    b.extend(kind);
    b.extend(payload);
    b
}

/// `brand` is e.g. `b"isom"` (MP4) or `b"qt  "` (MOV).
pub fn build(brand: &[u8; 4], created_unix_utc: i64, duration_s: u32) -> Vec<u8> {
    let mut ftyp = Vec::new();
    ftyp.extend(brand);
    ftyp.extend(0x200u32.to_be_bytes());
    ftyp.extend(brand);
    ftyp.extend(b"mp42");

    let t = u32::try_from(created_unix_utc + MAC_EPOCH_OFFSET).expect("date out of mvhd v0 range");
    let mut mvhd = Vec::with_capacity(100);
    mvhd.extend([0u8; 4]); // version 0, flags
    mvhd.extend(t.to_be_bytes()); // creation_time
    mvhd.extend(t.to_be_bytes()); // modification_time
    mvhd.extend(1000u32.to_be_bytes()); // timescale
    mvhd.extend((duration_s * 1000).to_be_bytes());
    mvhd.extend(0x0001_0000u32.to_be_bytes()); // rate 1.0
    mvhd.extend(0x0100u16.to_be_bytes()); // volume 1.0
    mvhd.extend([0u8; 10]); // reserved
    for v in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        mvhd.extend(v.to_be_bytes()); // unity matrix
    }
    mvhd.extend([0u8; 24]); // pre_defined
    mvhd.extend(2u32.to_be_bytes()); // next_track_ID
    debug_assert_eq!(mvhd.len(), 100);

    let mut out = bx(b"ftyp", &ftyp);
    out.extend(bx(b"moov", &bx(b"mvhd", &mvhd)));
    out.extend(bx(b"mdat", &[]));
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn mvhd_time_is_1904_based() {
        let f = super::build(b"isom", 0, 1);
        let i = f.windows(4).position(|w| w == b"mvhd").unwrap();
        let t = u32::from_be_bytes(f[i + 8..i + 12].try_into().unwrap());
        assert_eq!(t as i64, super::MAC_EPOCH_OFFSET);
    }
}
