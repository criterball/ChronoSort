# photocraft / craft-libs (image and video codecs), rev 77c7c62345c1f4213f59a07d9fdc156ecde0b540

**Source:** https://github.com/storytold/photocraft (app) and https://github.com/storytold/craft-libs (libraries).
Pinned as git dependencies of `crates/cs-image` and `crates/cs-video` to the commit above. Reviewed 2026-11-22 (image), 2026-11-24 (video).

**Licence:** craft-libs Apache-2.0 (workspace crates MIT OR Apache-2.0). heic-rs 0.1.4: MIT OR Apache-2.0.
Both are compatible with ChronoSort's GPL-3.0-or-later. The Apache NOTICE goes into the release licence list.

**What we use (image code only, not the photocraft GUI, which is egui):**
- `craft-image-codecs`, feature `heif`:
  - decode JPEG / PNG / TIFF / WebP / GIF / BMP / HEIC
  - encode JPEG / PNG / TIFF / WebP; it keeps EXIF, XMP and ICC
  - `Image::oriented`: rotate / flip at any bit depth, with the EXIF orientation reset to 1
  - `fidelity_warnings`
- `craft-heif`: a pure-Rust HEIC decoder (wraps heic-rs).
- `craft-media` (feature `isobmff` only): the MP4 / MOV demuxer, for video posters.
- `craft-video` (features `h264-decode`, `hevc-decode`, `vp9-decode`, `av1-decode`): one poster frame per video (`cs-video`).
- Not used: `craft-raw`. RAW previews stay with cs-meta's embedded-preview reader; demosaicing would need `craft-develop`.

## Findings
- **Security:**
  - `craft-image-codecs` and `craft-heif` both declare `#![forbid(unsafe_code)]` and deny unwrap/expect/panic in library code.
  - craft-heif runs every heic-rs call inside `catch_unwind`, so a malformed HEIC becomes an error, not a crash.
  - heic-rs itself has 3 `unsafe` mentions; it runs only behind that guard.
  - Decoding has default `Limits`. cs-image also refuses pictures over 200 MP before decoding (`probe_size`).
- **Bugs:** none found in the parts used. Tested here:
  - decode + save round-trips for JPEG/PNG/TIFF/WebP
  - two real HEIC files (1280×854 in 76 ms, 3992×2992 in 127 ms, release build)
  - orientation math checked for all 8 EXIF orientations
- **Performance:** HEIC thumbnails no longer need FFmpeg (in-process, about 0.1 s for 12 MP). FFmpeg stays as the fallback.

### Video (2026-11-24)
- **Security:** `craft-video` and `craft-media` contain no `unsafe` at all (the craft-libs workspace sets `unsafe_code = "forbid"`) and deny unwrap/expect/panic in library code. ChronoSort still runs the poster code in a separate helper process (the program itself with `--cs-video-poster`), with the same 20 s limit and output cap as FFmpeg, so a hang or crash costs one thumbnail. `cs-video` also caps picture size (80 MP), packet size (64 MiB) and packets decoded (60).
- **Correctness:** the first frame's decoded planes compared with FFmpeg 8.1 (`examples/yuvcheck.rs`): **bit-identical** (max difference 0) for H.264 High, HEVC Main and Main 10 (also 4K), VP9 and AV1 test clips. Colour conversion vs FFmpeg: 48.9 dB PSNR for BT.709-tagged clips and 48.7 dB for untagged SD. (FFmpeg's own 10-bit RGB conversions disagree with each other at about 34 dB, so 10-bit is checked on the planes.)
- **Not supported:** H.264 High 10 (no picture), MKV / WebM / AVI (not demuxed here). These fall back to FFmpeg when it is set up.
- **Robustness:** `crates/cs-video/tests/robustness.rs`, a mutation fuzzer over five tiny generated clips in `tests/seeds`.
- **Speed (release):** 10–70 ms per thumbnail; a full 4K HEVC 10-bit frame in 340 ms.

## What we do differently
- Edits run on an upright in-memory copy. Saving never overwrites: it uses a temp name, then `rename_no_replace`.
- A JPEG that is only turned or flipped is saved losslessly with ExifTool's Orientation tag, not re-encoded.
- Metadata writing uses ExifTool (bundled, out of process) with a verified backup and undo, not craft-metadata.

**Upstream issues/PRs filed:** none yet.

**Still to do:** a cargo-fuzz target for `cs_image::open_bytes` before the first release.
