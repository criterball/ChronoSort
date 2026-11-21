# Rust dependencies: review log

## 2026-11-20: Phase 0 baseline
`cargo audit` (cargo-audit 0.23.0, RustSec DB of 1550 advisories), 80 crates in Cargo.lock: **no vulnerabilities, no warnings.**

| Crate | Why | License | Notes |
|---|---|---|---|
| rusqlite 0.43 (`bundled`) | journal/catalog | MIT (SQLite: public domain) | SQLite C source compiled with w64devkit GCC 16. WAL + synchronous=FULL. |
| image 0.25 (jpeg, png only) | test corpus encode; thumbnails later | MIT/Apache-2.0 | Decoders are pure Rust (zune-jpeg, png). Features kept minimal to shrink the attack surface. |
| windows-sys 0.61 | MoveFileExW, GetDiskFreeSpaceExW | MIT/Apache-2.0 | Bindings only. |
| libc 0.2 | renameat2, renamex_np, statvfs | MIT/Apache-2.0 | Bindings only. |
| serde / serde_json | manifest, decisions | MIT/Apache-2.0 | |
| clap 4, anyhow, thiserror | CLI / errors | MIT/Apache-2.0 | |
| tempfile (dev) | tests | MIT/Apache-2.0 | |

All licenses are GPL-3.0 compatible.

## Toolchain note
Rust (windows-gnu) + w64devkit GCC 16 are used together. An empty `libgcc_eh.a` shim is needed (w64devkit keeps the unwinder in libgcc.a). Mixing in Rust's self-contained MinGW instead caused `undefined reference to nanosleep64` (mismatched mingw-w64 headers vs runtime), so everything is linked with w64devkit.

## 2026-11-20: Phase 1 additions
`cargo audit` again clean after adding these.

| Crate | Why | License | Review notes |
|---|---|---|---|
| kamadak-exif 0.6.2 | EXIF | BSD-2 | Full review: [kamadak-exif.md](kamadak-exif.md). TIFF read capped at 64 MiB by us. |
| quick-xml 0.42 | XMP sidecars | MIT | Never resolves external entities or DTDs (no XXE). Our loop is capped at 100k events, input capped at 4 MiB. Note: 0.42 changed names/values to `&str`. |
| regex 1.13 | filename/folder dates | MIT/Apache-2.0 | Guaranteed linear-time matching, so no ReDoS from hostile file names. |
| walkdir 2.5 | directory walk | MIT/Unlicense | Used with `follow_links(false)`. We also skip Windows junctions (reparse points) ourselves. |
| blake3 1.8 | content hashes / duplicates | CC0/Apache-2.0 | Uses hand-written SIMD asm/C on x86 and NEON (that's our "assembly for speed"). The `cs-core/pure` feature switches to pure Rust for cross-OS compile checks. |
| rayon 1.12 | parallel metadata reading | MIT/Apache-2.0 | |

## Our own parsers (not third-party, but untrusted input)
- `cs-meta/src/qt.rs`: MP4/MOV box walker. Rejects boxes larger than their parent or smaller than a header, caps the walk at 4096 boxes, handles 64-bit sizes and size 0 = "to end of file". Unit tests cover hostile sizes.
- `cs-meta/src/damage.rs`: JPEG/PNG integrity. At most 1024 segments, tail search at most 4 MiB.
- All are exercised by `crates/cs-core/tests/mutation.rs` on every test run.

## 2026-11-20: Phase 1 part 2 (planning + safe copy)
`cargo audit` clean.

| Crate | Why | License | Review notes |
|---|---|---|---|
| trash 5.4 | move mode: originals go to the OS trash, never deleted | MIT | Called only after the whole group is copied, verified, and the journal is forced to disk (`durable_barrier`). Undo does not rely on trash restore (that is not supported on macOS); it copies the verified copy back instead. |
| unicode-normalization 0.1 | NFC name keys (macOS NFD vs Windows NFC) | MIT/Apache-2.0 | |

## Performance finding (our code)
On this machine's D: drive, one file fsync takes about 40 ms and one SQLite FULL commit about 35 ms. The first executor (per-file commits, sequential) took about 175 ms/file. Now: batched journal transactions (64 files), WAL+NORMAL (crash-safe; power-cut gaps are handled by `recover`), and 4 parallel copies for files < 16 MiB, giving about 60 ms/file on this disk. The per-file fsync is deliberately kept: "verified" must mean "on disk".

## 2026-11-20: Desktop app (Tauri 2.12.7, Svelte 5, Vite 8)
- `npm audit`: 0 vulnerabilities (43 packages, build-time only; nothing from npm runs on the network at runtime).
- `cargo audit`: **0 vulnerabilities**, 3 warnings, all inside Tauri's dependency tree:
  - RUSTSEC-2024-0429 `glib` 0.18 (unsound `VariantStrIter`) and RUSTSEC-2024-0370 `proc-macro-error` (unmaintained): **Linux-only** (GTK via muda/tauri). We never use `VariantStrIter`. Recheck when the Linux port starts / when Tauri moves to gtk4.
  - RUSTSEC-2026-0543 `chrono` unmaintained: pulled by `serde_with` (Tauri config). We do no date handling with it (our own `cs_model::civil`).
- Windows/MinGW: needs `WebView2Loader.dll` (Microsoft redistributable, shipped next to the exe). The WebView2 runtime is part of Windows 10/11.
- Security config: strict CSP (no remote origins; `img-src` only self/data/thumb), capability = `core:default` + `dialog:allow-open`, no fs/shell/http plugins. The UI cannot read files: thumbnails are served by the `thumb://` scheme only for hashes from the current analysis. Verified in the release build: `fetch("https://example.com")` from the UI fails (blocked).
- Our own bug found: `ChronoSort.exe` (app) and `chronosort.exe` (CLI) are the same name on case-insensitive Windows, so the CLI build overwrote the app. The CLI is now `chronosort-cli.exe`.
