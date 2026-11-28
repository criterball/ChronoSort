//! How fast pictures and sound decode (release): cargo run --release -p cs-video --example playspeed -- FILE…
fn main() {
    for a in std::env::args().skip(1) {
        let p = std::path::Path::new(&a);
        let info = cs_video::play::info(p);
        let t = std::time::Instant::now();
        let mut s = cs_video::play::pictures(p, 0.0, 1920);
        let (mut n, mut size) = (0, (0, 0));
        while !s.ended() {
            match s.try_next() {
                Some(f) => {
                    n += 1;
                    size = (f.width, f.height);
                }
                None => std::thread::sleep(std::time::Duration::from_millis(1)),
            }
        }
        let el = t.elapsed().as_secs_f64();
        let snd = cs_video::play::sound(p, 0.0).map(|mut s| {
            let mut samples = 0;
            while !s.ended() {
                match s.try_next() {
                    Some(b) => samples += b.stereo.len() / 2,
                    None => std::thread::sleep(std::time::Duration::from_millis(1)),
                }
            }
            samples
        });
        println!("{a}: {info:?}\n  {n} frames {size:?} in {el:.2} s = {:.1} fps; sound frames: {snd:?}", n as f64 / el);
    }
}
