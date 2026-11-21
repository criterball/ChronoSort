# ChronoSort — Architecture

ChronoSort is an offline, cross-platform tool. It sorts photos and videos into year folders. Its first job is to **never lose or damage a file**.

License: **GPL-3.0-or-later**. Anyone who distributes it, or a modified version, must keep it open source.

---

## 1. Principles (hard rules)

1. **The source is read-only.** Source files are only opened for reading. We never write metadata into originals.
2. **Never overwrite, never hard-delete.** Name collisions get renamed. Move mode = copy → verify → send to the OS trash. No move without a verified copy.
3. **Log before acting.** Every action is written to SQLite *before* it runs and marked done *after* it is verified. A crash or power cut can always be resumed or undone.
4. **Verify everything.** The hash computed while reading the source must equal the hash from re-reading the destination. The timestamps on the copy are read back and compared.
5. **Explain every decision.** Each date comes with its evidence and a confidence score, shown in the UI ("EXIF DateTimeOriginal; folder agrees").
6. **Fully offline.** No network code in the app. The Tauri content-security policy blocks remote loads. AI helpers listen only on `127.0.0.1` with a random port and a random API key.
7. **Dry run first.** A plan is always built and shown before anything is written.
8. **Portable from day one.** All OS-specific code lives in one crate (`cs-platform`).

## 2. Pipeline

```
Scan → Extract → Decide → Review → Plan → Dry-run → Execute → Verify → Report
 │        │         │        │        │                 │
 walk   metadata  evidence  user    layout +          journal,
 group  filename  engine    confirm naming            undo, resume
 pairs  folder    scoring   (grid)  collisions
```

## 3. Workspace layout (Rust)

```
ChronoSort/
├─ crates/
│  ├─ cs-platform   OS seam: file times (incl. creation), no-replace rename, free space,
│  │                long paths, trash, Unicode normalisation, volume info, GPU detection
│  ├─ cs-model      shared types: MediaItem, Evidence, DateDecision, Confidence, Plan
│  ├─ cs-scan       parallel walk (jwalk/rayon), file type sniffing by content (not by
│  │                extension), groups RAW+JPG, Live Photo HEIC+MOV, .xmp/.THM/.aae/.json
│  ├─ cs-meta       native EXIF/XMP/QuickTime read (Rust parsers), ExifTool process for
│  │                hard cases, filename date parser, folder-path date parser
│  ├─ cs-date       evidence engine: collect → weigh → check conflicts → decide
│  ├─ cs-plan       destination layout, naming templates, collision resolution
│  ├─ cs-exec       safe copy/move, verification, timestamp restore, journal, undo
│  ├─ cs-db         SQLite (rusqlite): catalog, journal, faces, people, settings
│  ├─ cs-thumb      thumbnail cache (JPEG/PNG/WebP in Rust; HEIC/RAW/video via helpers)
│  ├─ cs-faces      [phase 2] ONNX Runtime (`ort`): YuNet detect, SFace embed,
│  │                clustering, people, per-year appearance model, age estimate
│  ├─ cs-ai         [phase 3] llama-server supervisor, prompts, JSON-schema output
│  └─ cs-geo        [phase 4] offline geocoding, GPS bulk assign, GPX import
├─ apps/
│  ├─ chronosort-cli   headless runner (testing, scripting, power users)
│  └─ chronosort-app   Tauri 2 shell + ui/ (Svelte + TypeScript)
├─ testdata/        synthetic edge-case photo set + generator
├─ third_party/     review notes for every reused project (see THIRD_PARTY_POLICY.md)
└─ docs/
```

## 4. Date evidence engine

Each source produces an `Evidence { date, precision, source, weight, note }`.
Precision can be second, day, month, year, or decade.

| Source | Base weight | Notes |
|---|---|---|
| EXIF DateTimeOriginal / CreateDate (+ OffsetTime) | 0.95 | Lowered if it is a known reset date (2000-01-01, 2002-01-01 00:00 …) |
| QuickTime/MP4 CreationDate (with time zone) | 0.90 | `mvhd` is often UTC. Convert to local time using a TZ tag or GPS. |
| Google Takeout `.json`, `.xmp` sidecar | 0.85 | |
| Filename pattern (`IMG_20050312_…`, `IMG-20190101-WA…`, `2005-03-12`) | 0.80 | Many patterns; each one has a test |
| Folder path (`2005\03\`, `Xmas 2003`, `2005-03 Holiday`) | 0.60–0.75 | Higher when year + month folders nest |
| AI: printed date stamp read from the image | 0.70 | phase 3 |
| AI: faces + age model | 0.40–0.70 | phase 2 |
| AI: era / scene guess | 0.20 | decade precision only |
| Filesystem mtime / ctime | 0.15 | Often just the copy date |

**Sanity rules:**
- Date in the future → reject.
- Date before the camera model existed → reject.
- Scanner make in EXIF → that date is the scan date, so drop it to low weight.
- Date before 1826 → reject.
- Burst or sibling consistency (files from the same folder or batch should agree).

**Decision:**
- Top year score ≥ 0.90 with no strong conflict → auto-sorted.
- Otherwise → review queue, showing every piece of evidence.
- The user can choose a year, a decade bucket (`1990s_Approx`), or `Unknown_Year`.

**Batch confirm:** confirming one item offers the same year to its siblings (same folder, same scan batch, look-alike photos).

## 5. Safe copy algorithm (cs-exec)

```
journal: PENDING(src, dst, src_size, src_mtime)
open src read-only (share-read); stream → dst.chronosort-partial, hashing with BLAKE3
flush + fsync dst; close
re-read dst, hash → must equal src hash (else delete partial, mark FAILED, keep going)
set dst times: modified, accessed, created (where the OS allows)
read back the times → compare
rename partial → final name, NO-REPLACE (fails if the name exists → pick a new collision name)
journal: DONE(hash)
move mode only: send src to OS trash → journal: SOURCE_TRASHED
```

- Free space is checked up front for the whole plan, plus a margin.
- Copies run in parallel per disk, and in sequence on spinning or network disks.
- A sidecar file is moved only after its main file succeeds.

## 6. Portability (cs-platform)

| Concern | Windows | macOS | Linux |
|---|---|---|---|
| Set creation time | `SetFileTime` | `setattrlist(ATTR_CMN_CRTIME)` | **Not possible** → keep in journal + XMP |
| No-replace rename | `MoveFileExW` without REPLACE_EXISTING | `renamex_np(RENAME_EXCL)` | `renameat2(RENAME_NOREPLACE)` (fallback: `link`+`unlink`) |
| Trash | `IFileOperation` (via `trash` crate) | NSFileManager trash | freedesktop trash spec |
| Long paths | `\\?\` prefix | n/a | n/a |
| Filename normalisation | NFC | **NFD on disk** → compare in NFC | as-is |
| Case-insensitive collisions | yes | usually yes | no → detect per volume |
| GPU detection | DXGI / nvml | Metal | Vulkan / nvml |

Rules:
- No `#[cfg(windows)]` code outside `cs-platform`.
- CI runs `cargo check` for `x86_64-unknown-linux-gnu`, `x86_64-apple-darwin` and `aarch64-apple-darwin`. Those targets are already installed in `tools\rust`.

## 7. AI and hardware tiers

The face models (YuNet ~0.3 MB, SFace ~37 MB) run inside the app process through ONNX Runtime.
The vision LLM runs as a **separate `llama-server` process**, for three reasons:
- If it crashes, the app and the copy job keep running.
- The app can choose a CUDA, Vulkan, Metal or CPU build at runtime.
- llama.cpp can be updated without rebuilding ChronoSort.

| Tier | Detect rule | Vision LLM | Faces |
|---|---|---|---|
| Max | VRAM ≥ 11 GB, RAM ≥ 32 GB | Qwen3-VL-8B Q5_K_M/Q6_K | GPU |
| Standard | VRAM 6–10 GB, RAM ≥ 16 GB | Qwen3-VL-4B Q4_K_M | GPU/CPU |
| Light | VRAM < 6 GB or none | Qwen3-VL-2B Q4 on CPU, review queue only | CPU |
| Metadata-only | user choice | off | off/optional |

- One model is loaded at a time: the face pass runs first, then the vision pass.
- The vision LLM only runs on items left in the review queue.

## 8. Security

Image and video files are **untrusted input**. Malformed files are a classic route for exploits.

- Parse in memory-safe Rust where we can.
- C/C++ parsers (libheif, LibRaw, ffmpeg, ExifTool/Perl) run **out of process**, with timeouts and size limits and no elevated rights. A crash there means "unreadable file", not a compromised app.
- Every parser in our own code is fuzzed (cargo-fuzz in the Linux build).
- `cargo deny` checks licenses and duplicate dependencies. `cargo audit` checks the RustSec advisories, run offline against a vendored DB snapshot.
- `Cargo.lock` is committed. Dependencies are pinned and reviewed before upgrades.
- The app never needs admin or root rights.
- Tauri: strict CSP and a minimal allowlist of capabilities. The UI cannot touch the filesystem directly; it can only call typed Rust commands.

## 9. UI (Tauri 2 + Svelte)

Screens:
1. **Setup:** source, destination, copy/move, naming (keep by default, clashes get `_1`; or date/time or a template, with a live preview), hardware tier. Layout is plain year folders; if the source has month folders the user is asked: keep `YYYY/<month>/` or flatten.
2. **Scan progress:** counts by date source.
3. **Review grid:** a virtualised grid of thumbnails, each with a suggested year, a confidence colour and evidence chips. Tick to accept, type a year to change it, batch confirm. Keyboard first.
4. **Plan / dry run:** a tree of the destination with counts, conflicts, and duplicates found before copying (only one copy of identical content is written; files already in the destination are not copied again).
5. **Run:** progress, pause/resume.
6. **Report:** a summary, with an undo button.
7. Later: **People** (name face clusters) and **Places** (offline map).

Light and dark themes. The design should feel calm and minimal, borrowing from Immich and Apple Photos.
