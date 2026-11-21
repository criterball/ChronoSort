# ChronoSort — licensing, hardware tiers, Rust stack (2026-11-20)

## Decisions from the user so far
- Will probably be open source. No decision on selling yet (probably not).
- Dev machine: 64 GB RAM, 12 GB VRAM. Must also run on 16 GB RAM / 6 GB VRAM machines.
- Language: Rust first. C++ and assembly where they help.

## Licensing
Key rule: keep **every bundled dependency commercial-safe**. Then any license choice can still be changed later.
The user owns the copyright to their own code, so they can relicense or sell a separate license later.
If outside contributors join, a CLA is needed to keep that option.

| Option | Who can do what | Effect on dependencies |
|---|---|---|
| MIT / Apache-2.0 (dual, the Rust norm) | Anyone can use it, sell it, or fork it closed. | Must avoid GPL code in the binary. |
| GPL-3.0-or-later | Forks must stay open. Selling binaries and support is still allowed. | Can use GPL libraries. |
| Dual (GPL + paid commercial license) | Open for the community, paid for closed users. | Needs CLA. Must avoid third-party GPL code. |
| Closed commercial | Only permissive dependencies. No InsightFace models, no GPL. | Strictest. |

Component check:
- llama.cpp MIT. ONNX Runtime MIT. Tauri MIT/Apache. OpenCV Zoo YuNet/SFace Apache-2.0. All OK.
- Qwen3-VL weights: Apache-2.0 (verify per model card). Gemma and Llama have custom licenses with use restrictions.
- ExifTool: Perl license (Artistic/GPL). It runs as a separate process, so it does not affect our license.
- ffmpeg: use an LGPL build as a separate process. Avoid GPL-only codecs if the app might ever be sold.
- libheif LGPL (dynamic link or separate process). LibRaw LGPL/CDDL.
- InsightFace buffalo_l: models are **non-commercial**. Never bundle them. At most, an optional "bring your own model" slot.
- Age model: still open. Need a permissive one or train our own.

## Hardware tiers (auto-detected, user can override)
| Tier | Target | Vision LLM | Faces |
|---|---|---|---|
| Max | ≥12 GB VRAM, ≥32 GB RAM | Qwen3-VL-8B Q5/Q6 fully on GPU | GPU |
| Standard | 6–8 GB VRAM, 16 GB RAM | Qwen3-VL-4B Q4_K_M on GPU | GPU or CPU |
| Light | no/weak GPU, 8–16 GB RAM | Qwen3-VL-2B Q4 on CPU (slow), runs only on undated photos | CPU (YuNet/SFace are small) |
| Metadata-only | anything | off | optional |
Load one model at a time (faces pass, then VLM pass). The VLM only sees the uncertain subset of photos.
GPU backends: llama.cpp CUDA / Vulkan / Metal. ONNX Runtime DirectML / CUDA / CoreML (via the `ort` crate).

## Rust stack
- UI: Tauri 2 + Svelte. Uses WebView2 on Windows, WebKit on macOS, WebKitGTK on Linux.
- Workspace crates: core (date-evidence engine), io (safe copy + verify + timestamps), meta, faces, ai, geo, db, app.
- Crates: jwalk, rayon, blake3 (has hand-written SIMD asm), nom-exif / kamadak-exif, rusqlite, image, zune-jpeg, fast_image_resize, filetime, windows (SetFileTime for creation time), trash, ort.
- Run as separate processes: llama-server (llama.cpp), exiftool (writes and edge cases), ffmpeg (video frames). This isolates crashes and lets us swap GPU builds at runtime.
- Assembly: we get it through blake3, libjpeg-turbo / zune and fast_image_resize SIMD. No hand-written asm unless profiling shows a hotspot.
- Platform note: **Linux cannot set file birth/creation time** from user space. Keep mtime there and store the original creation time in the journal and XMP.
- Toolchain: portable rustup in tools\rust (RUSTUP_HOME/CARGO_HOME). Target x86_64-pc-windows-gnu with the existing w64devkit, or MSVC build tools if Tauri needs them.
