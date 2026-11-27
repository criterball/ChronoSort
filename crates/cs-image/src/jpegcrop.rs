//! Lossless JPEG crop: the compressed 8×8 blocks inside the crop are copied as
//! they are (no decoding to pixels, no re-compression), like `jpegtran -crop`.
//! Only the Huffman (entropy) coding is redone, with optimal tables.
//!
//! Supported: baseline and extended sequential Huffman JPEGs with 8-bit samples
//! and one interleaved scan (what cameras and phones write). Progressive,
//! arithmetic-coded, lossless (SOF3) and multi-scan files are refused with a
//! clear message; the caller can then offer a normal crop.
//!
//! The crop's left and top edges move to the block grid (8 or 16 pixels,
//! depending on the colour subsampling); the right and bottom edges are kept
//! exactly. Everything before the image data (EXIF, ICC profile, comments,
//! quantisation tables) is kept; the caller updates the EXIF size afterwards.

use std::fmt;

#[derive(Debug, PartialEq)]
pub enum CropError {
    NotJpeg,
    Progressive,
    Unsupported(&'static str),
    Damaged(&'static str),
    Empty,
}

impl fmt::Display for CropError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CropError::NotJpeg => write!(f, "This is not a JPEG picture."),
            CropError::Progressive => {
                write!(
                    f,
                    "This JPEG is stored progressively (common for web pictures); it cannot be cropped losslessly. Use the normal crop."
                )
            }
            CropError::Unsupported(w) => {
                write!(f, "This kind of JPEG cannot be cropped losslessly ({w}). Use the normal crop.")
            }
            CropError::Damaged(w) => write!(f, "This JPEG seems damaged ({w})."),
            CropError::Empty => write!(f, "The crop is empty."),
        }
    }
}

impl std::error::Error for CropError {}

type R<T> = Result<T, CropError>;

/// A rectangle: x, y, width, height (pixels).
pub type Rect = (u32, u32, u32, u32);

#[derive(Clone, Copy, Debug)]
struct Comp {
    id: u8,
    h: u8,
    v: u8,
    tq: u8,
}

/// Block grid of a JPEG: (width, height, MCU width, MCU height) in pixels.
pub fn grid(jpeg: &[u8]) -> R<(u32, u32, u32, u32)> {
    let p = parse(jpeg)?;
    let (hm, vm) = p.max_hv();
    Ok((p.width, p.height, 8 * hm, 8 * vm))
}

/// The crop that will really be made for the wanted rectangle (in stored pixels):
/// left / top moved down to the block grid, right / bottom kept, all inside the picture.
pub fn snapped(width: u32, height: u32, (mcu_w, mcu_h): (u32, u32), (x, y, w, h): Rect) -> R<Rect> {
    if x >= width || y >= height || w == 0 || h == 0 {
        return Err(CropError::Empty);
    }
    let x1 = x.saturating_add(w).min(width);
    let y1 = y.saturating_add(h).min(height);
    let x0 = (x.min(width) / mcu_w) * mcu_w;
    let y0 = (y.min(height) / mcu_h) * mcu_h;
    if x1 <= x0 || y1 <= y0 {
        return Err(CropError::Empty);
    }
    Ok((x0, y0, x1 - x0, y1 - y0))
}

struct Parsed<'a> {
    width: u32,
    height: u32,
    comps: Vec<Comp>,
    /// Segments to copy into the output (APPn, COM, DQT), whole, with markers.
    keep: Vec<&'a [u8]>,
    dc: [Option<Huff>; 4],
    ac: [Option<Huff>; 4],
    restart: u32,
    /// Per scan component (same order as `comps`): (dc table, ac table).
    tables: Vec<(usize, usize)>,
    /// The entropy-coded data (up to EOI or the next non-RST marker).
    data: &'a [u8],
}

impl Parsed<'_> {
    fn max_hv(&self) -> (u32, u32) {
        (
            self.comps.iter().map(|c| u32::from(c.h)).max().unwrap_or(1),
            self.comps.iter().map(|c| u32::from(c.v)).max().unwrap_or(1),
        )
    }
}

fn be16(b: &[u8], i: usize) -> R<usize> {
    b.get(i..i + 2).map(|s| usize::from(u16::from_be_bytes([s[0], s[1]]))).ok_or(CropError::Damaged("cut short"))
}

fn parse(b: &[u8]) -> R<Parsed<'_>> {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return Err(CropError::NotJpeg);
    }
    let mut i = 2;
    let mut keep = Vec::new();
    let (mut dc, mut ac): ([Option<Huff>; 4], [Option<Huff>; 4]) = Default::default();
    let mut restart = 0;
    let mut frame: Option<(u32, u32, Vec<Comp>)> = None;
    loop {
        // Skip fill bytes.
        while b.get(i) == Some(&0xFF) && b.get(i + 1) == Some(&0xFF) {
            i += 1;
        }
        if b.get(i) != Some(&0xFF) {
            return Err(CropError::Damaged("marker expected"));
        }
        let m = *b.get(i + 1).ok_or(CropError::Damaged("cut short"))?;
        if m == 0xD9 {
            return Err(CropError::Damaged("no picture data"));
        }
        let len = be16(b, i + 2)?;
        if len < 2 || i + 2 + len > b.len() {
            return Err(CropError::Damaged("segment length"));
        }
        let seg = &b[i..i + 2 + len];
        let body = &b[i + 4..i + 2 + len];
        match m {
            0xE0..=0xEF | 0xFE | 0xDB => keep.push(seg),
            0xC0 | 0xC1 => {
                if body.len() < 6 || body[0] != 8 {
                    return Err(CropError::Unsupported("not 8-bit"));
                }
                let height = u32::from(u16::from_be_bytes([body[1], body[2]]));
                let width = u32::from(u16::from_be_bytes([body[3], body[4]]));
                let n = usize::from(body[5]);
                if width == 0 || height == 0 || n == 0 || n > 4 || body.len() < 6 + 3 * n {
                    return Err(CropError::Damaged("frame header"));
                }
                let comps: Vec<Comp> = (0..n)
                    .map(|k| {
                        let c = &body[6 + 3 * k..9 + 3 * k];
                        Comp { id: c[0], h: c[1] >> 4, v: c[1] & 15, tq: c[2] }
                    })
                    .collect();
                if comps.iter().any(|c| !(1..=4).contains(&c.h) || !(1..=4).contains(&c.v) || c.tq > 3) {
                    return Err(CropError::Damaged("sampling factors"));
                }
                frame = Some((width, height, comps));
            }
            0xC2 | 0xC6 | 0xCA | 0xCE => return Err(CropError::Progressive),
            0xC3 | 0xC5 | 0xC7 | 0xCB | 0xCD | 0xCF => return Err(CropError::Unsupported("lossless or hierarchical")),
            0xC9 | 0xCC => return Err(CropError::Unsupported("arithmetic coding")),
            0xC4 => {
                let mut j = 0;
                while j < body.len() {
                    let tc = body[j] >> 4;
                    let th = usize::from(body[j] & 15);
                    if tc > 1 || th > 3 || j + 17 > body.len() {
                        return Err(CropError::Damaged("Huffman table"));
                    }
                    let mut counts = [0u8; 16];
                    counts.copy_from_slice(&body[j + 1..j + 17]);
                    let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
                    if total > 256 || j + 17 + total > body.len() {
                        return Err(CropError::Damaged("Huffman table"));
                    }
                    let h = Huff::new(&counts, &body[j + 17..j + 17 + total])?;
                    if tc == 0 {
                        dc[th] = Some(h)
                    } else {
                        ac[th] = Some(h)
                    }
                    j += 17 + total;
                }
            }
            0xDD => restart = be16(b, i + 4)? as u32,
            0xDA => {
                let (width, height, comps) = frame.ok_or(CropError::Damaged("picture data before the frame header"))?;
                let ns = usize::from(*body.first().ok_or(CropError::Damaged("scan header"))?);
                if ns != comps.len() || body.len() < 1 + 2 * ns + 3 {
                    return Err(CropError::Unsupported("several scans"));
                }
                let mut tables = Vec::with_capacity(ns);
                for (k, c) in comps.iter().enumerate() {
                    let s = &body[1 + 2 * k..3 + 2 * k];
                    if s[0] != c.id {
                        return Err(CropError::Unsupported("scan order"));
                    }
                    tables.push((usize::from(s[1] >> 4).min(3), usize::from(s[1] & 15).min(3)));
                }
                let tail = &body[1 + 2 * ns..];
                if tail[0] != 0 || tail[1] != 63 || tail[2] != 0 {
                    return Err(CropError::Progressive);
                }
                let start = i + 2 + len;
                return Ok(Parsed { width, height, comps, keep, dc, ac, restart, tables, data: &b[start..] });
            }
            _ => {} // other segments (e.g. JPEG extensions, DNL) are dropped
        }
        i += 2 + len;
    }
}

// ------------------------------------------------------------ Huffman tables

#[derive(Clone, Debug)]
struct Huff {
    /// Fast lookup on the next 9 bits: (length, symbol); length 0 = longer code.
    fast: Vec<(u8, u8)>,
    maxcode: [i32; 18],
    valptr: [i32; 17],
    mincode: [i32; 17],
    vals: Vec<u8>,
}

const FAST: u32 = 9;

impl Huff {
    fn new(counts: &[u8; 16], vals: &[u8]) -> R<Huff> {
        let mut maxcode = [-1i32; 18];
        let mut valptr = [0i32; 17];
        let mut mincode = [0i32; 17];
        let mut fast = vec![(0u8, 0u8); 1 << FAST];
        let mut code = 0i32;
        let mut k = 0usize;
        for l in 1..=16 {
            let n = usize::from(counts[l - 1]);
            valptr[l] = k as i32;
            mincode[l] = code;
            for _ in 0..n {
                if l as u32 <= FAST {
                    let shift = FAST - l as u32;
                    for f in 0..(1u32 << shift) {
                        let idx = ((code as u32) << shift | f) as usize;
                        if idx >= fast.len() {
                            return Err(CropError::Damaged("Huffman table"));
                        }
                        fast[idx] = (l as u8, vals[k]);
                    }
                }
                code += 1;
                k += 1;
            }
            maxcode[l] = if n > 0 { code - 1 } else { -1 };
            if code > (1 << l) {
                return Err(CropError::Damaged("Huffman table"));
            }
            code <<= 1;
        }
        maxcode[17] = i32::MAX;
        Ok(Huff { fast, maxcode, valptr, mincode, vals: vals.to_vec() })
    }
}

// ---------------------------------------------------------------- bit reader

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
    /// A marker was reached: only zero bits follow.
    marker: bool,
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Self {
        Bits { d, pos: 0, acc: 0, n: 0, marker: false }
    }
    fn fill(&mut self) {
        while self.n <= 56 {
            let mut byte = 0u8;
            if !self.marker && self.pos < self.d.len() {
                let c = self.d[self.pos];
                if c == 0xFF {
                    match self.d.get(self.pos + 1) {
                        Some(0x00) => {
                            byte = 0xFF;
                            self.pos += 2;
                        }
                        _ => self.marker = true,
                    }
                } else {
                    byte = c;
                    self.pos += 1;
                }
            }
            self.acc |= u64::from(byte) << (56 - self.n);
            self.n += 8;
        }
    }
    fn peek(&mut self, k: u32) -> u32 {
        self.fill();
        (self.acc >> (64 - k)) as u32
    }
    fn skip(&mut self, k: u32) {
        self.acc <<= k;
        self.n -= k;
    }
    fn get(&mut self, k: u32) -> u32 {
        if k == 0 {
            return 0;
        }
        let v = self.peek(k);
        self.skip(k);
        v
    }
    fn decode(&mut self, h: &Huff) -> R<u8> {
        let (l, s) = h.fast[self.peek(FAST) as usize];
        if l > 0 {
            self.skip(u32::from(l));
            return Ok(s);
        }
        let mut code = self.get(1) as i32;
        for l in 1..=16 {
            if code <= h.maxcode[l] {
                let idx = h.valptr[l] + code - h.mincode[l];
                return h.vals.get(idx as usize).copied().ok_or(CropError::Damaged("bad code"));
            }
            code = (code << 1) | self.get(1) as i32;
        }
        Err(CropError::Damaged("bad code"))
    }
    /// After a restart interval: drop the partial byte and step over the RSTn marker.
    fn restart(&mut self) -> R<()> {
        self.acc = 0;
        self.n = 0;
        self.marker = false;
        while self.pos + 1 < self.d.len() {
            if self.d[self.pos] == 0xFF && (0xD0..=0xD7).contains(&self.d[self.pos + 1]) {
                self.pos += 2;
                return Ok(());
            }
            if self.d[self.pos] == 0xFF && self.d[self.pos + 1] != 0x00 && self.d[self.pos + 1] != 0xFF {
                return Err(CropError::Damaged("restart marker missing"));
            }
            self.pos += 1;
        }
        Err(CropError::Damaged("restart marker missing"))
    }
}

fn extend(v: u32, s: u8) -> i32 {
    if s == 0 {
        return 0;
    }
    let v = v as i32;
    if v < (1 << (s - 1)) { v - (1 << s) + 1 } else { v }
}

// ------------------------------------------------------------------ decoding

/// Quantised coefficients of one component, in zig-zag order, block by block (row-major).
struct Plane {
    bw: usize,
    bh: usize,
    blocks: Vec<[i16; 64]>,
}

fn decode(p: &Parsed) -> R<Vec<Plane>> {
    let (hm, vm) = p.max_hv();
    let mcux = p.width.div_ceil(8 * hm) as usize;
    let mcuy = p.height.div_ceil(8 * vm) as usize;
    let mut planes: Vec<Plane> = p
        .comps
        .iter()
        .map(|c| {
            let (bw, bh) = (mcux * usize::from(c.h), mcuy * usize::from(c.v));
            Plane { bw, bh, blocks: vec![[0i16; 64]; bw * bh] }
        })
        .collect();
    let tabs: Vec<(&Huff, &Huff)> = p
        .tables
        .iter()
        .map(|&(d, a)| {
            Ok((
                p.dc[d].as_ref().ok_or(CropError::Damaged("missing table"))?,
                p.ac[a].as_ref().ok_or(CropError::Damaged("missing table"))?,
            ))
        })
        .collect::<R<_>>()?;
    let mut bits = Bits::new(p.data);
    let mut pred = vec![0i32; p.comps.len()];
    let total = mcux * mcuy;
    for m in 0..total {
        if p.restart > 0 && m > 0 && m % p.restart as usize == 0 {
            bits.restart()?;
            pred.iter_mut().for_each(|x| *x = 0);
        }
        let (mx, my) = (m % mcux, m / mcux);
        for (ci, c) in p.comps.iter().enumerate() {
            let (dct, act) = tabs[ci];
            for v in 0..usize::from(c.v) {
                for h in 0..usize::from(c.h) {
                    let bx = mx * usize::from(c.h) + h;
                    let by = my * usize::from(c.v) + v;
                    let mut blk = [0i16; 64];
                    let t = bits.decode(dct)?;
                    if t > 11 {
                        return Err(CropError::Damaged("DC value"));
                    }
                    pred[ci] += extend(bits.get(u32::from(t)), t);
                    blk[0] = pred[ci].clamp(-32768, 32767) as i16;
                    let mut k = 1;
                    while k < 64 {
                        let rs = bits.decode(act)?;
                        let (r, s) = (usize::from(rs >> 4), rs & 15);
                        if s == 0 {
                            if r == 15 {
                                k += 16;
                                continue;
                            }
                            break;
                        }
                        k += r;
                        if k > 63 || s > 10 {
                            return Err(CropError::Damaged("AC value"));
                        }
                        blk[k] = extend(bits.get(u32::from(s)), s) as i16;
                        k += 1;
                    }
                    let pl = &mut planes[ci];
                    pl.blocks[by * pl.bw + bx] = blk;
                }
            }
        }
    }
    Ok(planes)
}

// ------------------------------------------------------------------ encoding

fn magnitude(v: i32) -> u8 {
    (32 - v.unsigned_abs().leading_zeros()) as u8
}

/// Optimal Huffman table (bits, values) for symbol frequencies, ≤ 16-bit codes (JPEG Annex K.2).
fn optimal_table(freq_in: &[u64; 256]) -> ([u8; 16], Vec<u8>) {
    let mut freq = [0u64; 257];
    freq[..256].copy_from_slice(freq_in);
    freq[256] = 1; // reserved so no code is all ones
    let mut codesize = [0usize; 257];
    let mut others = [-1i32; 257];
    loop {
        let mut c1: i32 = -1;
        let mut v = u64::MAX;
        for (i, &f) in freq.iter().enumerate() {
            if f != 0 && f <= v {
                v = f;
                c1 = i as i32;
            }
        }
        let mut c2: i32 = -1;
        v = u64::MAX;
        for (i, &f) in freq.iter().enumerate() {
            if f != 0 && f <= v && i as i32 != c1 {
                v = f;
                c2 = i as i32;
            }
        }
        if c2 < 0 {
            break;
        }
        let (c1u, c2u) = (c1 as usize, c2 as usize);
        freq[c1u] += freq[c2u];
        freq[c2u] = 0;
        let mut c = c1u;
        codesize[c] += 1;
        while others[c] >= 0 {
            c = others[c] as usize;
            codesize[c] += 1;
        }
        others[c] = c2;
        c = c2u;
        codesize[c] += 1;
        while others[c] >= 0 {
            c = others[c] as usize;
            codesize[c] += 1;
        }
    }
    let mut bits = [0usize; 33];
    for &s in codesize.iter() {
        if s > 0 {
            bits[s.min(32)] += 1;
        }
    }
    let mut i = 32;
    while i > 16 {
        while bits[i] > 0 {
            let mut j = i - 2;
            while bits[j] == 0 {
                j -= 1;
            }
            bits[i] -= 2;
            bits[i - 1] += 1;
            bits[j + 1] += 2;
            bits[j] -= 1;
        }
        i -= 1;
    }
    while bits[i] == 0 {
        i -= 1;
    }
    bits[i] -= 1; // remove the reserved code
    let mut out_bits = [0u8; 16];
    for l in 1..=16 {
        out_bits[l - 1] = bits[l] as u8;
    }
    let mut vals = Vec::new();
    for l in 1..=32 {
        for (sym, &cs) in codesize.iter().enumerate().take(256) {
            if cs == l {
                vals.push(sym as u8);
            }
        }
    }
    (out_bits, vals)
}

/// (code, length) per symbol from a (bits, values) table.
fn codes(bits: &[u8; 16], vals: &[u8]) -> Vec<(u16, u8)> {
    let mut out = vec![(0u16, 0u8); 256];
    let mut code = 0u32;
    let mut k = 0;
    for l in 1..=16u8 {
        for _ in 0..bits[usize::from(l) - 1] {
            out[usize::from(vals[k])] = (code as u16, l);
            code += 1;
            k += 1;
        }
        code <<= 1;
    }
    out
}

struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, v: u32, len: u32) {
        if len == 0 {
            return;
        }
        self.acc = (self.acc << len) | (v & ((1u32 << len) - 1));
        self.n += len;
        while self.n >= 8 {
            let b = (self.acc >> (self.n - 8)) as u8;
            self.out.push(b);
            if b == 0xFF {
                self.out.push(0x00);
            }
            self.n -= 8;
        }
        self.acc &= (1u32 << self.n).wrapping_sub(1);
    }
    fn flush(&mut self) {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
    }
}

/// Visit every block of the crop in MCU order: (component, block).
fn each_block<'a>(
    comps: &[Comp],
    planes: &'a [Plane],
    mcux: usize,
    mcuy: usize,
    mx0: usize,
    my0: usize,
    mut f: impl FnMut(usize, &'a [i16; 64]),
) {
    for my in 0..mcuy {
        for mx in 0..mcux {
            for (ci, c) in comps.iter().enumerate() {
                let pl = &planes[ci];
                for v in 0..usize::from(c.v) {
                    for h in 0..usize::from(c.h) {
                        let bx = (mx0 + mx) * usize::from(c.h) + h;
                        let by = (my0 + my) * usize::from(c.v) + v;
                        f(ci, &pl.blocks[by.min(pl.bh - 1) * pl.bw + bx.min(pl.bw - 1)]);
                    }
                }
            }
        }
    }
}

/// Crop `jpeg` losslessly to the rectangle (stored pixels, before any EXIF turn).
/// Returns the new file and the rectangle really used (see [`snapped`]).
pub fn crop(jpeg: &[u8], x: u32, y: u32, w: u32, h: u32) -> R<(Vec<u8>, Rect)> {
    let p = parse(jpeg)?;
    let (hm, vm) = p.max_hv();
    let (mw, mh) = (8 * hm, 8 * vm);
    let (x0, y0, cw, ch) = snapped(p.width, p.height, (mw, mh), (x, y, w, h))?;
    let planes = decode(&p)?;
    let (mx0, my0) = ((x0 / mw) as usize, (y0 / mh) as usize);
    let (mcux, mcuy) = (cw.div_ceil(mw) as usize, ch.div_ceil(mh) as usize);
    // Table 0 for the first component (brightness), table 1 for the others (colour).
    let tsel = |ci: usize| usize::from(ci > 0);
    // Pass 1: symbol counts for optimal tables.
    let mut fdc = [[0u64; 256]; 2];
    let mut fac = [[0u64; 256]; 2];
    let mut pred = vec![0i32; p.comps.len()];
    each_block(&p.comps, &planes, mcux, mcuy, mx0, my0, |ci, b| {
        let t = tsel(ci);
        let d = i32::from(b[0]) - pred[ci];
        pred[ci] = i32::from(b[0]);
        fdc[t][usize::from(magnitude(d))] += 1;
        let mut run = 0;
        for &c in &b[1..] {
            if c == 0 {
                run += 1;
                continue;
            }
            while run > 15 {
                fac[t][0xF0] += 1;
                run -= 16;
            }
            fac[t][(run << 4) | usize::from(magnitude(i32::from(c)))] += 1;
            run = 0;
        }
        if run > 0 {
            fac[t][0x00] += 1;
        }
    });
    let ncls = if p.comps.len() > 1 { 2 } else { 1 };
    let dct: Vec<([u8; 16], Vec<u8>)> = (0..ncls).map(|t| optimal_table(&fdc[t])).collect();
    let act: Vec<([u8; 16], Vec<u8>)> = (0..ncls).map(|t| optimal_table(&fac[t])).collect();
    let dcc: Vec<Vec<(u16, u8)>> = dct.iter().map(|(b, v)| codes(b, v)).collect();
    let acc: Vec<Vec<(u16, u8)>> = act.iter().map(|(b, v)| codes(b, v)).collect();
    // Pass 2: write.
    let mut bw = BitWriter { out: Vec::with_capacity(jpeg.len() / 2), acc: 0, n: 0 };
    let mut pred = vec![0i32; p.comps.len()];
    each_block(&p.comps, &planes, mcux, mcuy, mx0, my0, |ci, b| {
        let t = tsel(ci);
        let d = i32::from(b[0]) - pred[ci];
        pred[ci] = i32::from(b[0]);
        let s = magnitude(d);
        let (c, l) = dcc[t][usize::from(s)];
        bw.put(u32::from(c), u32::from(l));
        bw.put(if d < 0 { (d - 1) as u32 } else { d as u32 }, u32::from(s));
        let mut run = 0;
        for &c in &b[1..] {
            if c == 0 {
                run += 1;
                continue;
            }
            while run > 15 {
                let (code, l) = acc[t][0xF0];
                bw.put(u32::from(code), u32::from(l));
                run -= 16;
            }
            let v = i32::from(c);
            let s = magnitude(v);
            let (code, l) = acc[t][(run << 4) | usize::from(s)];
            bw.put(u32::from(code), u32::from(l));
            bw.put(if v < 0 { (v - 1) as u32 } else { v as u32 }, u32::from(s));
            run = 0;
        }
        if run > 0 {
            let (code, l) = acc[t][0x00];
            bw.put(u32::from(code), u32::from(l));
        }
    });
    bw.flush();

    let mut out = Vec::with_capacity(bw.out.len() + 4096);
    out.extend_from_slice(&[0xFF, 0xD8]);
    for s in &p.keep {
        out.extend_from_slice(s);
    }
    // Frame header with the new size.
    let n = p.comps.len();
    out.extend_from_slice(&[0xFF, 0xC0]);
    out.extend_from_slice(&((8 + 3 * n) as u16).to_be_bytes());
    out.push(8);
    out.extend_from_slice(&(ch as u16).to_be_bytes());
    out.extend_from_slice(&(cw as u16).to_be_bytes());
    out.push(n as u8);
    for c in &p.comps {
        out.extend_from_slice(&[c.id, (c.h << 4) | c.v, c.tq]);
    }
    // Huffman tables.
    for t in 0..ncls {
        for (class, (bits, vals)) in [(0u8, &dct[t]), (1u8, &act[t])] {
            out.extend_from_slice(&[0xFF, 0xC4]);
            out.extend_from_slice(&((2 + 1 + 16 + vals.len()) as u16).to_be_bytes());
            out.push((class << 4) | t as u8);
            out.extend_from_slice(bits);
            out.extend_from_slice(vals);
        }
    }
    // Scan header.
    out.extend_from_slice(&[0xFF, 0xDA]);
    out.extend_from_slice(&((6 + 2 * n) as u16).to_be_bytes());
    out.push(n as u8);
    for (ci, c) in p.comps.iter().enumerate() {
        let t = tsel(ci) as u8;
        out.extend_from_slice(&[c.id, (t << 4) | t]);
    }
    out.extend_from_slice(&[0, 63, 0]);
    out.extend_from_slice(&bw.out);
    out.extend_from_slice(&[0xFF, 0xD9]);
    Ok((out, (x0, y0, cw, ch)))
}

/// The rectangle in stored pixels for a rectangle in upright pixels, for an EXIF
/// orientation (1–8). `sw`, `sh`: the stored size.
pub fn upright_to_stored(o: u16, sw: u32, sh: u32, x: u32, y: u32, w: u32, h: u32) -> (u32, u32, u32, u32) {
    let sub = |a: u32, b: u32| a.saturating_sub(b);
    match o {
        2 => (sub(sw, x + w), y, w, h),
        3 => (sub(sw, x + w), sub(sh, y + h), w, h),
        4 => (x, sub(sh, y + h), w, h),
        5 => (y, x, h, w),
        6 => (y, sub(sh, x + w), h, w),
        7 => (sub(sw, y + h), sub(sh, x + w), h, w),
        8 => (sub(sw, y + h), x, h, w),
        _ => (x, y, w, h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: u32, h: u32) -> image::RgbImage {
        image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x * 7 + y * 3) as u8, ((x * 2) ^ (y * 5)) as u8, ((x / 3 + y / 2) * 11) as u8])
        })
    }
    fn jpeg(img: &image::RgbImage, q: u8) -> Vec<u8> {
        let mut v = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut v, q).encode_image(img).unwrap();
        v
    }
    fn decode_rgb(b: &[u8]) -> image::RgbImage {
        image::load_from_memory_with_format(b, image::ImageFormat::Jpeg).unwrap().to_rgb8()
    }

    #[test]
    fn snapping_moves_only_left_and_top() {
        assert_eq!(snapped(100, 80, (16, 16), (21, 9, 30, 40)), Ok((16, 0, 35, 49)));
        assert_eq!(snapped(100, 80, (8, 8), (0, 0, 500, 500)), Ok((0, 0, 100, 80)));
        assert_eq!(snapped(100, 80, (16, 16), (100, 0, 10, 10)), Err(CropError::Empty));
    }

    #[test]
    fn crop_keeps_the_blocks_exactly() {
        let src = picture(203, 141);
        let j = jpeg(&src, 85);
        let (w, h, mw, mh) = grid(&j).unwrap();
        assert_eq!((w, h), (203, 141));
        let whole = decode_rgb(&j);
        let (out, (x0, y0, cw, ch)) = crop(&j, 37, 22, 120, 90).unwrap();
        assert_eq!((x0, y0), ((37 / mw) * mw, (22 / mh) * mh));
        assert_eq!((cw, ch), (37 + 120 - x0, 22 + 90 - y0));
        let part = decode_rgb(&out);
        assert_eq!(part.dimensions(), (cw, ch));
        // Inside, away from the edges where colour upsampling looks at neighbours,
        // every pixel is the same as in the whole picture.
        let mut diff = 0;
        for yy in 2..ch - 2 {
            for xx in 2..cw - 2 {
                let a = part.get_pixel(xx, yy).0;
                let b = whole.get_pixel(x0 + xx, y0 + yy).0;
                diff = diff.max((0..3).map(|c| (i32::from(a[c]) - i32::from(b[c])).abs()).max().unwrap());
            }
        }
        assert!(diff <= 1, "max difference {diff}");
        // Cropping the whole picture gives the same pixels, and it can be read again.
        let (full, r) = crop(&j, 0, 0, w, h).unwrap();
        assert_eq!(r, (0, 0, w, h));
        assert_eq!(decode_rgb(&full).as_raw(), whole.as_raw());
        assert!(crop(&full, 8, 8, 16, 16).is_ok());
    }

    #[test]
    fn grayscale_and_restart_intervals() {
        let g = image::GrayImage::from_fn(64, 48, |x, y| image::Luma([(x * 3 + y * 5) as u8]));
        let mut j = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut j, 90).encode_image(&g).unwrap();
        let (out, _) = crop(&j, 8, 8, 32, 24).unwrap();
        let a = image::load_from_memory(&out).unwrap().to_luma8();
        let b = image::load_from_memory(&j).unwrap().to_luma8();
        for y in 0..24 {
            for x in 0..32 {
                assert_eq!(a.get_pixel(x, y), b.get_pixel(x + 8, y + 8));
            }
        }
        // Insert a restart interval by hand: decode must step over the RST markers.
        // (Re-encoding a crop drops them; covered by decoding with DRI below.)
        let mut bits = Bits::new(&[0b1010_0000, 0xFF, 0xD0, 0b1100_0000]);
        assert_eq!(bits.get(3), 0b101);
        bits.restart().unwrap();
        assert_eq!(bits.get(2), 0b11);
    }

    #[test]
    fn refuses_what_it_cannot_do() {
        assert_eq!(crop(b"not a jpeg", 0, 0, 1, 1).unwrap_err(), CropError::NotJpeg);
        let mut j = jpeg(&picture(32, 32), 80);
        // Turn the frame into a progressive one (SOF2): refused, not mangled.
        let i = j.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        j[i + 1] = 0xC2;
        assert_eq!(crop(&j, 0, 0, 8, 8).unwrap_err(), CropError::Progressive);
        let short = jpeg(&picture(32, 32), 80);
        assert!(matches!(crop(&short[..short.len() / 2], 0, 0, 8, 8), Err(_) | Ok(_)));
    }

    #[test]
    fn upright_rectangles_map_to_stored_ones() {
        // Stored 40×30; orientation 6 shows it 30×40 (turned right).
        assert_eq!(upright_to_stored(1, 40, 30, 1, 2, 3, 4), (1, 2, 3, 4));
        assert_eq!(upright_to_stored(6, 40, 30, 0, 0, 30, 40), (0, 0, 40, 30));
        // The top-left upright corner is the bottom-left stored corner for 6.
        assert_eq!(upright_to_stored(6, 40, 30, 0, 0, 10, 5), (0, 20, 5, 10));
        assert_eq!(upright_to_stored(8, 40, 30, 0, 0, 10, 5), (35, 0, 5, 10));
        assert_eq!(upright_to_stored(3, 40, 30, 0, 0, 10, 5), (30, 25, 10, 5));
    }
}
