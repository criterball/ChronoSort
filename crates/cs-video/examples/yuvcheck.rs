//! Decoder check without any colour conversion: the first frame's planes of an
//! MP4/MOV compared sample by sample with a raw planar file of the same frame
//! (e.g. `ffmpeg -i x.mp4 -frames:v 1 -f rawvideo -pix_fmt yuv420p10le ref.yuv`).
//! cargo run --release -p cs-video --example yuvcheck -- <video> <ref.yuv>
use craft_media::isobmff::{self, TrackKind};
use craft_video::TrackDecoder;
use craft_video::frame::Samples;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let file = std::fs::File::open(&a[0]).unwrap();
    let mp4 = isobmff::open(&file).unwrap();
    let ti = mp4.track_of_kind(TrackKind::Video).unwrap();
    let t = &mp4.tracks[ti];
    let mut dec = TrackDecoder::with_threads(&t.codec_params(), 1).unwrap();
    let mut got = dec.decode(&mp4.read_sample(&file, ti, 0).unwrap(), t.samples[0].pts).unwrap();
    if got.is_empty() {
        got = dec.flush().unwrap();
    }
    let f = &got[0];
    let reference = std::fs::read(&a[1]).unwrap();
    let wide = f.bit_depth() > 8;
    let mut off = 0usize;
    for (k, p) in f.planes().iter().enumerate() {
        let (w, h) = (p.width() as usize, p.height() as usize);
        let (mut se, mut max_diff, mut n) = (0f64, 0i64, 0usize);
        for y in 0..h {
            for x in 0..w {
                let ours = match p.samples() {
                    Samples::U8(v) => i64::from(v[y * p.stride() + x]),
                    Samples::U16(v) => i64::from(v[y * p.stride() + x]),
                };
                let theirs = if wide {
                    let i = off + 2 * (y * w + x);
                    i64::from(u16::from_le_bytes([reference[i], reference[i + 1]]))
                } else {
                    i64::from(reference[off + y * w + x])
                };
                let d = ours - theirs;
                se += (d * d) as f64;
                max_diff = max_diff.max(d.abs());
                n += 1;
            }
        }
        off += w * h * if wide { 2 } else { 1 };
        let peak = f64::from((1u32 << f.bit_depth()) - 1);
        let psnr = if se == 0.0 { f64::INFINITY } else { 10.0 * (peak * peak / (se / n as f64)).log10() };
        println!("plane {k}: {w}x{h} depth {} max diff {max_diff} PSNR {psnr:.1} dB", p.bit_depth());
    }
}
