# STATUS (resume point)

Updated: 2026-11-24

## Decided
- Name: ChronoSort. License: GPL-3.0-or-later.
- Rust + Tauri 2/Svelte. C++ via llama.cpp / ONNX Runtime as separate processes or libraries. Asm only through SIMD libraries unless profiling says otherwise.
- Windows first; portable design (`cs-platform`). Linux and macOS later.
- Hardware tiers: Max (dev PC: RTX 3060 12 GB, 64 GB RAM), Standard (6 GB / 16 GB), Light, Metadata-only.
- Only commercial-safe dependencies, no non-commercial models (GPL compatibility).
- Every reused project gets a security/bug/perf review (docs/THIRD_PARTY_POLICY.md).

## Environment
- Rust: `tools\rust` (RUSTUP_HOME=tools\rust\rustup, CARGO_HOME=tools\rust\cargo), rustc 1.100.0, target x86_64-pc-windows-gnu. Linux and macOS targets are installed. Smoke test passed.
- Node v24.15.0: `tools\node`.
- llama.cpp b10276 (Vulkan, has llama-server + mtmd): `tools\llama-bin`.

- ExifTool 13.64: `tools\exiftool\exiftool.exe` (checksum verified).
- Toolchain: w64devkit GCC 16 links Rust and compiles C. An empty libgcc_eh.a shim (scripts/mingw-shim, made by env.sh) is needed. Do NOT mix in the Rust self-contained MinGW (causes nanosleep64 link errors). Always `. scripts/env.sh` first.
- cargo-audit 0.23 installed. check.sh runs it offline.

## Done: Phase 0 (2026-11-20)
- Workspace: cs-platform, cs-model, cs-db, cs-testgen, chronosort-cli. 18 tests pass. `scripts/check.sh` passes (fmt, clippy -D warnings, tests, Linux/macOS cargo check).
- Corpus: `chronosort testgen D:/Claude_Code_Home/temp/cs_corpus` → 34 files + manifest.json. Independently verified with ExifTool.
- Local git repo initialised, nothing committed yet.

## Done: Phase 1, part 1 (analysis, read-only), 2026-11-20
- New crates: cs-scan (walk, content sniffing, junk, grouping), cs-meta (EXIF via kamadak-exif, MP4/MOV mvhd, XMP, Takeout, filename/folder dates, JPEG/PNG damage), cs-date (weights, sanity rules, decisions, readable reasons), cs-core (parallel pipeline, BLAKE3, duplicate detection).
- CLI: `chronosort analyze SRC [-v] [--json report.json]`, `chronosort eval CORPUS`.
- Corpus: 34/34 files exactly as expected (`crates/cs-core/tests/corpus.rs`, and `chronosort eval`).
- Robustness: 1,020 mutated files, no panics (`tests/mutation.rs`).
- Weights are all in `crates/cs-date/src/lib.rs`. Auto needs score ≥ 0.90 and no contradicting evidence ≥ 0.50. Suggestions need ≥ 0.25.
- Duplicate primary rule: not-a-copy path > oldest mtime > shortest path > name.

## Done: Phase 1, part 2 (planning + safe copy), 2026-11-20
User decisions: plain year folders (`2005/`). If the source has month folders, ASK the user (keep `2005/<month>/` or flatten). Names: keep the original by default, clash -> `_1`, `_2` at the end; optional `datetime` or a template ({original} {year} {month} {day} {hour} {minute} {second}). Duplicates are found BEFORE copying, so nothing redundant is copied.
- cs-plan: layout, month question, naming/sanitising (Windows reserved names, illegal chars), NFC+lowercase clash keys, group-consistent names (RAW+JPG, sidecars), duplicate skip (whole group only if every member is redundant), destination pre-scan (hashes only same-size files) so re-runs copy nothing twice.
- cs-exec: verified copy (BLAKE3 while copying + re-read), times restored (modified/accessed/created), no-replace rename, partial files, chunked journal, parallel small files, recover after a crash, undo (removes only unchanged copies; move-undo copies back), refuses dest-inside-source, free-space check, plan JSON saved in dest/.chronosort/.
- CLI: `plan`, `run [--move] [-y] [--months keep|flatten] [--rename keep|datetime|TEMPLATE] [--accept-suggested]`, `undo DEST [--run N]`.
- Tests: 49 total. e2e: copy + verify every hash/time, source unchanged, re-run copies 0, undo, an edited copy is kept on undo, crash with partial file + pending journal -> recover -> resume.
- NOT yet tested end-to-end: move mode (it would put test files in the real Recycle Bin). Needs a test on a scratch user/VM, or an injectable trash function.

## Done: desktop app + more (2026-11-20)
- Move mode: injectable `ExecOptions.trash` (default OS Recycle Bin). e2e test with a fake trash: originals moved, undo restores them with original times.
- `--threads N` for runs. Progress callbacks (analysis + copy).
- cs-thumb: JPEG/PNG/GIF/BMP/WebP via `image` with decode limits (40k px, 768 MiB) so decompression bombs are refused; EXIF orientation; RAW/HEIC via the embedded EXIF preview; content-addressed cache.
- apps/chronosort-app: Tauri 2 + Svelte 5. Screens: Setup → Review (grouped by folder, batch accept / set-all, evidence chips, distrusted evidence struck through) → Plan (month question, stats, preview) → Copy (progress) → Report + Undo. `thumb://` scheme, strict CSP. Start options `--source --dest --analyze` (+ debug-only `--goto plan`).
- Screenshots: docs/screenshots/01..06. NOTE: Grabby captures WebView2 windows as blank (GPU composition), so screenshots are taken with `apps/chronosort-app/scripts/cdp.mjs PORT shot FILE` (start the app with WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=PORT). cdp.mjs also does `eval` / `click` for UI testing.
- CLI renamed `chronosort-cli` (case clash with ChronoSort.exe on Windows).
- dist/: ChronoSort.exe, WebView2Loader.dll, chronosort-cli.exe, README.txt, LICENSE.
- Build app: `npm run build` in apps/chronosort-app, then `cargo build --release -p chronosort-app --features chronosort-app/custom-protocol`.

## Done: 2026-11-20 afternoon (stopped 15:50 at the user's request; nothing committed)
- **Grabby v0.6.0** (the user's repo, working copy `tools\grabby-src`, installed in `tools\grabby`, v0.5.0 backed up in `backups\grabby-v0.5.0`). NOT pushed to GitHub, so ask the user first.
  - Fixes: PW_RENDERFULLCONTENT (WebView2/Tauri/Electron no longer blank); crash on emoji window titles (UTF-8 stdout).
  - New: --delay / delay_seconds / GUI countdown (Esc cancels); --client / client_area; blank detection; DPI awareness; DWM frame trimming; fast PNG; Help page F1 + shortcut keys + Exit; --help examples. 13 unit tests.
  - Built with `tools\python-full` (portable CPython 3.12.16 + Tk + PyInstaller): `$env:GRABBY_PYTHON=...python-full\python.exe; & '.\Build Portable.ps1'`.
  - Release notes: tools\grabby-src\RELEASE_NOTES_v0.6.0.md
- **ChronoSort engine (done + tested, 57 tests):**
  - Naming rules for user patterns (cs-plan/naming.rs): no / \ : * ? etc., max 80 chars, max 40 chars of fixed text, must contain {original} or the full date+time, no leading/trailing dot/space. Plain-language error messages. `cs_plan::naming_preview(items, naming, n)` gives live examples.
  - Duplicate finder `cs_core::dup::find_duplicates(roots, include_all, progress)`: size buckets first, BLAKE3 only same-size files, never counts the same file twice (overlapping folders), suggests which copy to keep (full-path original_rank), `write_csv` (UTF-8 BOM for Excel, never overwrites).
  - `cs_exec::resolve_duplicates(actions, DupMode::MoveTo(dir)|Trash, roots, journal_dir, trash)`: re-checks the kept copy and every extra by hash right before acting, never touches the kept copy, refuses a Duplicates folder inside the checked folders, cross-drive = verified copy + trash. Journal op `relocate` / `trash` (DB schema v3 migration keeps old rows). `undo` moves relocated duplicates back; `recover` is now operation-aware.
  - `cs_platform::open_in_file_manager`.
- **App backend commands added (compiles, not yet used by the UI):** check_naming, find_duplicates (event dup-progress), resolve_duplicates(keep: {hash: index}, mode "move"|"trash", target), save_duplicate_report(path), undo_duplicates(target), open_folder, exit_app. Capabilities: dialog open/save/ask/message.

## Done: UI rework (2026-11-20 evening; nothing committed)
- Home menu (big tiles + big Exit), header always has Home / Help / Settings / Exit (confirm only while a job runs).
- Sort flow: Folders -> Overview -> Check -> File names (Keep / Date+time / Date+original / Year-month+original / own pattern with live check_naming validation + token buttons + examples from the user's files) -> Final check (month question) -> Copy -> Report (Undo, Open folder, Back to menu, Sort another).
- Find duplicates: several folders (Browse or typed path), include-all-types, progress, **preview pane** (user asked: "hard to tell which to keep without a live preview"): list of sets left, 1280 px preview right (new preview:// scheme, cs_thumb::PREVIEW_SIZE, cached as <hash>_1280.jpg), every copy with full folder, date, keep/extra chip, **Show in Explorer** (new command show_duplicate(hash,index) -> cs_platform::show_file_in_file_manager, file selected; only files of the current scan). Keys: Up/Down set, 1-9 keep, Space include/leave alone. Actions: move to sibling "<name> duplicates" (default) / Recycle Bin (confirm) / save CSV. Done screen with Undo + Open folder.
- Settings: themes System/Light/Dark/Navy/Darker Pastel/High contrast, font, text size (real webview zoom, permission core:webview:allow-set-webview-zoom), This computer (RAM, threads). localStorage key chronosort.prefs.v1.
- Help: topics + shortcut table (F1, Esc/Alt+Home, Ctrl+1/2, Ctrl+, , Ctrl+Q, Ctrl+Enter = button marked data-continue).
- State lives in src/lib/state.svelte.js (survives visiting Settings/Help); sort/dups views stay mounted.
- Tested in the real app via cdp.mjs + Grabby screenshots (docs/screenshots/ui2/01-14): corpus sort 27 files copied+verified; dups: 18 files, 12 extras moved with a user-chosen keeper, undo restored all 18. check.sh passes. dist/ refreshed.
- Found: old target/release/chronosort-cli.exe predated schema v3 (refused the journal). CLI package name is `chronosort` (cargo build -p chronosort).
- Open question to the user: the header badge "Offline" is a static privacy badge, not a network check; proposed renaming to "Private - works without internet" or removing it.

## Done: 2026-11-20 later evening (nothing committed)
- **Check screen large preview** (Lightbox.svelte, native <dialog>): click a thumbnail -> 1280 px preview, all evidence, year box focused; Enter = confirm + next, Left/Right = prev/next, Esc closes (App ignores Esc / Ctrl+Enter while a dialog is open). List is frozen on open so confirmed files do not vanish mid-review.
- **GPU detection** `cs_platform::gpus()` (gpu.rs): Windows registry display class key (HardwareInformation.qwMemorySize / MemorySize, no COM), Linux sysfs + nvidia-smi, macOS none yet. Integrated vs dedicated by name; virtual adapters dropped. `chronosort-cli info` prints them. This PC: RTX 3060 12.0 GiB dedicated, UHD 750 integrated.
- **crates/cs-ai** (new): Tier (Metadata/Light/Standard/Max, thresholds per research/02 table), assess() with plain-language reason, catalogue.json (faces-opencv, qwen3-vl-2b/4b/8b, qwen3.5-9b), fit() Gpu/Cpu/TooBig, recommend() (prefers packs the user already has), Linked = models.json (pack -> role -> path/size/mtime; status Missing/Partial/Ready/Changed), link() checks GGUF magic / ONNX first byte / size within 0.5-2x. 8 tests.
  - Catalogue is a DRAFT: all packs `verified: false`, url/sha256 null, sizes approximate. `downloadable()` requires verified + url + sha256. Before enabling downloads: verify each source, licence and SHA-256 per docs/THIRD_PARTY_POLICY.md.
- App: commands ai_overview, link_model_file(pack, role, path), unlink_model(pack); models.json in app data dir. Settings -> This computer (RAM, each GPU, suggested level + reason) and AI models (per purpose, suggested pack, fit, status, "Use a file I have...", Change, Forget). Tested with the real Qwen3.5-9B + mmproj from project 001 (wrong file refused, partial -> ready, recommendation switched to 9B), then unlinked again. Help updated.
- check.sh passes, 70 tests. dist/ refreshed. Screenshots docs/screenshots/ui2/15-17.

## Decided 2026-11-20 (user): privacy rule
"No internet" means photos and videos never leave the PC and no online AI is used. Downloading AI models (to run locally) and other tools is OK, as long as those tools never send photos or videos out. The header badge now reads "🔒 Photos stay on this PC". README/Help were updated to match.

## Done: model downloads (2026-11-20 night; nothing committed)
- Catalogue verified (schema 2): exact bytes + SHA-256 from the HF API, URLs pinned to commits, licences checked (YuNet is MIT, not Apache). Record: research/03_model_sources.md. Review note: third_party/ureq-rustls-sha2.md.
- cs-ai/src/download.rs (the only network code): ureq 3 + rustls; https only; allow-list huggingface.co (+ redirects to *.huggingface.co / *.hf.co), hand-followed redirects (max 5); `.part` + resume via Range (206 must start at the part length; 200 restarts); exact size + SHA-256 before rename; mismatch deletes the part; Cancel keeps it. Real-network test: `CS_NET_TESTS=1 cargo test -p cs-ai real_download` (download, resume, corruption caught) passes.
- link() now hashes a file whose size matches exactly and marks it `verified`. Other plausible files are accepted as "your own copy".
- App: download_pack (one at a time, free-space check of missing bytes + 1 GB, progress event model-progress, links each file as soon as it is verified), cancel_download, delete_model_files (only catalogue names inside <app data>/models/<pack>/, and keeps links to the user's own files). UI: Download / "Download the checked missing part", progress + Cancel, ✓ checked / your own copy chips, Delete downloaded files vs Forget, models folder link + free space.
- Tested in the app: faces pack downloaded (37 MB, Ready, both ✓ checked; KEPT in AppData\Roaming\org.chronosort.app\models\faces-opencv for the faces phase); Qwen3-VL 2B start / cancel / resume / delete; local Qwen3.5-9B model verified (48 s hash), local mmproj-BF16 (675 MB, unknown origin) = "your own copy", missing-part download fetches only the mmproj; test links removed afterwards.
- 75 tests, check.sh passes, dist refreshed. Screenshot docs/screenshots/ui2/18.

## Done: faces engine + AI engine downloads (2026-11-20, late; nothing committed)
- crates/cs-faces: YuNet detect + SFace fingerprint via tract (pure Rust, no telemetry; third_party/tract.md), ArcFace alignment, NMS, cluster() (CLUSTER 0.42), identify(); folder::scan_folder (cs_scan + cs_thumb::load, rayon). Note: cs_thumb::load is new (safe decode + orient + downscale).
- CLI: `chronosort-cli faces DIR [--crops OUT] [--json]` writes OUT/groups.png (contact sheet). Test set: scripts/fetch_face_testdata.py (public-domain Commons portraits, manifest with licence) -> temp/cs_faces. First result: 36 photos in 1.6 s; visually pure groups for Aldrin (decades apart), Glenn 1962 vs 1998, Armstrong, Collins, Ride; the only mix-up is the Kelly identical twins. Example `cargo run --release -p cs-faces --example eval -- MODELS SET` reports same/different stats.
- llama.cpp as downloadable "engine" packs (Windows: vulkan 34 MB = default with a GPU, cpu 18 MB), GitHub release b10276 + GitHub SHA-256 digests; allow-list github.com/ggml-org/llama.cpp/releases/download/ + release-assets/objects.githubusercontent.com. cs-ai/unpack.rs: safe zip extraction (enclosed_name, no symlinks, size/entry caps, tmp + rename). zip crate 9.x (2.x had 6 RUSTSEC advisories). Tested: download -> unpack -> llama-server --version = 10276.
- Home: "Set up the local AI for this PC?" offer (Set up AI… / Not now / Don't ask again). Settings: "Download all" for the suggested missing parts. Big Exit on Home removed (user).
- .gitignore tightened: a repo would be ~3.8 MB / 147 files (no node_modules, dist, exe/dll/zip, gguf/onnx, tools). Retake screenshots 16-18 (local paths) before publishing.
- 82 tests, check.sh passes, dist refreshed.

## Earlier resume point (2026-11-20): face eval (DONE 2026-11-21, see below)
- Test set: temp/cs_faces (72 public-domain photos, 9 people, manifest.json with source/licence). Commons rate-limits hard (429), so the fetch script is resumable and skips existing files.
- Grabby v0.6.0: RELEASED 2026-11-21 (commit ac782fd, tag v0.6.0, zip sha256 verified on GitHub).

## Done: face eval + People screen (2026-11-21; nothing committed)
- **Real accuracy** (eval with label filter + manifest `"wrong_person": true`): mark_kelly_02 is USAF Col Mark **D.** Kelly, not astronaut Mark **E.** Kelly (checked by eye); the model was right again. Result: 423 faces, 71 ms/photo detect, 39 ms/face fingerprint; at SAME_PERSON 0.363 different-person false matches **1/270 (0.37 %)**, same-person misses 7/55 (12.7 %, mostly decades apart). All 8 clusters with labelled faces are pure at CLUSTER 0.42, so 0.42 stays.
- **cs-db v4**: tables `people`, `face_samples` (fingerprint blob, year, 128 px JPEG crop; never photo paths). `people.rs`: add/rename/delete(+samples)/merge people, add_samples (cap MAX_SAMPLES 300, oldest dropped), all_samples, person_cover, clean_name. 3 tests.
- **cs-faces**: `centroid()`, `face_crop()` (square, 1.5x bbox, clamped). 1 test.
- **App** (`src-tauri/src/people.rs`): people library = `<app data>/people.db`. Commands find_faces (event faces-progress; models from models.json "faces-opencv" or pack_dir; years from the sort analysis if the same folder), suggest_person, save_person (new name / known name / person_id), list/rename/forget/merge_people, show_face_photo. Schemes `face://<n>` (crop of the current search) and `person://<id>_<version>` (`/` would be %2F-encoded by convertFileSrc). CSP updated.
- **UI** `People.svelte` (Home tile Ctrl+3, Help section "People and faces"): known people (picture, faces, years, Rename, Forget with confirm, "Same person as…" merge), folder → Find faces, groups with "Looks like X (91 % alike)" + "Yes, that's X", click faces → Split off, tick groups → Merge, double-click → Explorer, singles collapsed. Save is disabled once saved under that name.
- **Fixed**: Ctrl+Enter could press a *hidden* screen's continue button (views stay mounted); now only visible buttons count.
- Tested in the real app (cdp.mjs + Grabby): 147 faces/63 photos/24 groups (app skips faces < 40 px), name 2 groups → rescan suggests both (91 %/93 %), no wrong suggestions, split 8→6+2, merge back, rename with whitespace cleanup, merge people 12+10=22, forget → empty, blank name refused. Screenshots docs/screenshots/people/. Test people.db deleted afterwards. check.sh passes (86 tests).

## Tested 2026-11-21: "per-year appearance" does NOT work with SFace (dropped)
- `cargo run --release -p cs-faces --example age_eval -- <faces models> D:/Claude_Code_Home/temp/cs_faces`: leave-one-out on 23 dated faces of 7 people (near-duplicate crops excluded). Guessing a face's year from the person's most similar dated faces: mean error **9.2 years**; just taking the person's median year: **7.5 years**. The fingerprint is built to ignore age (Glenn 1998 matches Glenn 1962 best).
- So identity is the useful signal, not appearance. Plan instead: optional **birth year per person** + an **age estimate** from the vision model ("Anna, born 1990, looks ~5" -> ~1995), and the hard rule "not before the birth year".

## In progress 2026-11-21: vision engine + "Ask the AI" (nothing committed)
- **crates/cs-vision**: `Engine::start` runs llama-server with --host 127.0.0.1, random free port, random 128-bit key via --api-key-file (file deleted after start), --offline, --no-webui, --reasoning off, ctx 8192, 1 slot, CREATE_NO_WINDOW, log in <work>/vision-server.log; waits for /health. `date_photo(jpeg)` -> DateGuess {date_stamp, earliest/latest/best_year, confidence, clues} via JSON-schema constrained output; `sane()` clamps to 1826..2100. Own base64. 3 tests. Eval: `examples/date_eval.rs` (not run yet: model downloading).
- **cs-platform::end_with_us(child)** -> ChildGuard: Windows job object KILL_ON_JOB_CLOSE, so the engine dies even if ChronoSort crashes (test: dropping the guard ends a child). No-op on Linux/macOS for now (TODO PR_SET_DEATHSIG / kqueue).
- **App** `src-tauri/src/vision.rs`: commands ai_date(rels) (events ai-status, ai-date per photo; engine started lazily; picks the recommended ready vision pack, Vulkan engine if the model fits the GPU), ai_cancel, ai_stop; engine stopped on RunEvent::Exit.
- **UI Check step**: "Not sure? Ask the AI." bar with progress/Stop/"Turn the AI off"; amber "AI ≈ 1985 (1980–1990)" chip (clues in tooltip) and "Printed date" chip, also in the large preview; the AI year is used as the suggestion only when the date engine has none; never auto-accepted. Help updated (and the old claim that faces date photos removed).
- Downloading qwen3-vl-8b via the app's verified download (~3 MB/s, started 11:10).

## Done 2026-11-21 afternoon (nothing committed)
- **Vision eval (qwen3-vl-8b, 63 dated photos)**: v1 prompt: mean error 4.3 y, 44/63 within 5 y, but the model's own ranges held only 29/63, confidence was always "high", and clues named people (Glenn called "Gagarin"). v2 (research/vision_eval_2026-11-21_v2.txt): prompt forbids identifying people; schema = date_stamp, best_year, clues; range = best ± TYPICAL_ERROR_YEARS (5: measured 70 %, ±8 = 87 %). Result 4.4 y mean error, 45/63 inside range, 3.3 s/photo, no person names in clues. Caveat: famous photos, clue text can contain wrong facts.
- **Ask the AI tested in the real app**: 72 undated photos estimated (engine ready in ~7 s), chips + one-click suggestions, accept per folder, "Turn the AI off" kills llama-server. Screenshots docs/screenshots/ai/.
- **Progress cues (user rule, memory progress-cues-always)**: global download state (state.svelte.js `dl`, speed + time left), header chip on every screen while downloading (click -> Settings), Exit warns during a download, moving (indeterminate) bar + spinner CSS for unknown-length steps (download connect, AI start-up, first photo, face models loading, analyse/duplicates start, Planning). Tested: header chip 0 % -> 92 % -> "Checking the download… 100 %" -> gone.
- **Existing destination (user question)**: already safe (dest scanned, existing folders reused, name clash -> _1, identical content -> skipped as "already there", copy re-checks and never overwrites). New: `PlannedFile.name_taken_by` ("destination" | "this_run", test), Final check preview with thumbnails (click = large), tabs All / Name taken / Already there, explanation chip. Real-app test: a different 2009/barack_obama_01.jpg stayed byte-identical, new one -> _1; identical copy under another name skipped; undo removed exactly the 66 copies and 25 created folders, pre-existing files and folder kept.
- Download of qwen3-vl-8b got cut at 3.25 GB when the app (started inside a background shell task) was ended from outside; resume via Range worked. Start test apps with PowerShell Start-Process, not inside a background bash task.
- check.sh passes, 91 tests.

## Done 2026-11-21 (afternoon/evening; nothing committed)
- **Portable USB mode** (`src-tauri/src/portable.rs`, `Portable.svelte`): `portable.txt` next to the exe → data in `ChronoSortData` beside it; "make portable" copies program files + exiftool folder + optionally models/engine/people, with progress; program files always replaced, same-size data files skipped. A real copy onto a USB stick has NOT been tested yet.
- **Add a place / bulk GPS** (crates/cs-geo + `src-tauri/src/places.rs` + `Places.svelte`, Home has 6 tiles): offline place list from ExifTool's built-in geolocation DB (116 054 places, ~2 s, loaded once), search by town/country, tick photos per year folder, write GPS + XMP city/country via ExifTool with verified backups in `<folder>/.chronosort`, undo per run. ExifTool ships in `exiftool/` next to the exe (dist too). Screenshots docs/screenshots/ai/06–07.
  - Scan marks files that already have a place (one ExifTool `-j -r` pass; JSON taken after the summary lines; paths compared via `same_path`, so a typed trailing `\` still matches; unit test). Tested 21:00: try_place example 3 changed in 0.9 s, read back Adelaide −34.93/138.60, undo → all files byte-identical; scan command on temp/geo_scan correctly flags GPS + Country-only files.
- check.sh passes (12.5 min).
- **Click-through test in the real app (2026-11-21 evening)** via cdp.mjs + `apps/chronosort-app/scripts/press_dialog.ps1` (presses native ask() dialogs: they are task dialogs whose buttons are `CCPushButton` panes without an Invoke pattern, so the script sends BM_CLICK). Folder typed with a trailing `\`: 2 pinned + unticked, 3 ticked; Adelaide → 3 photos (GPS + City + Country), pinned ones untouched; "Add another place" rescan → 5 pinned + "Earlier in this folder"; Undo from that list; Paris with an already-placed photo ticked → dialog says "1 photo already has a place; it will be replaced"; Undo on the done screen. End: all 5 files byte-identical to the start. Screenshots docs/screenshots/places/01–07.
  - **Fixed (Places.svelte)**: Undo from the "Earlier in this folder" list restored the files but left the screen stale (all pins, nothing ticked), showed no message, never showed failures, and dropped the run from the list even if files could not be put back. Now it rescans (the run list comes from disk, so a partly failed run stays), shows "✓ Undone …" plus a "Could not be put back" list on both screens; the done-screen Undo button stays while failures remain; a new apply clears an old undo message.
  - Small, not changed: after Undo on the done screen the heading still says "… was added to 4 photos" (the green "Undone" line is under it). Backup run IDs use UTC time (internal only).
- dist/ChronoSort.exe rebuilt with these fixes (2026-11-21 evening).

## 2026-11-24 (later): item 5 video thumbnails without FFmpeg, 3c, item 4 / 6 closed (not committed)
- **`crates/cs-video`** (new): `poster(path, max_side)` / `poster_bytes` for MP4/MOV with craft-media (isobmff) + craft-video (h264/hevc/vp9/av1 decode), same pinned craft-libs rev as cs-image. Key frame at or before ~1 s (a tenth of short clips), flush after it; display matrix → upright (iPhone Rotation 90 checked); colour by H.273 matrix tag (untagged: 709 for ≥720 lines, else 601), 10-bit = value / 4. Caps: 80 MP, 64 MiB packet, 60 packets.
  - Checked against FFmpeg 8.1 (ShareX's ffmpeg.exe, test clips in temp/cs_video): decoded planes **bit-identical** for H.264, HEVC 8/10-bit (+4K), VP9, AV1 (`examples/yuvcheck.rs`); RGB 48.9 dB (709-tagged), 48.7 dB (SD). 10–70 ms per thumbnail, 4K HEVC10 full frame 340 ms.
  - Not supported: H.264 High 10, MKV/WebM/AVI → FFmpeg fallback.
  - `tests/robustness.rs` + 5 tiny seed clips (tests/seeds, generated with FFmpeg testsrc2): 300 cases debug; **100 000 release cases: 0 panics, 0 hangs, slowest 16 ms**.
- **cs-thumb**: videos go to the built-in helper first (`video_helper_main()`: the program re-runs itself with `--cs-video-poster <max> <file>`, PNG on stdout, same 20 s limit / 64 MiB cap as FFmpeg), then FFmpeg if set up. App and CLI call `video_helper_main()` first thing in main. CLI tested: PNG out, exit 1 with a message for missing / non-video files, 2 for missing args.
- Careful: `target/release/chronosort.exe` IS the app (`ChronoSort.exe`, case-insensitive). The CLI binary is `chronosort-cli.exe`. Running the wrong one opened three app windows one after another (2026-11-24, one background test with three calls); all closed.
- Texts: Help (built-in MP4/MOV), video placeholders in Duplicates / Lightbox / Review, FFmpeg pack description ("previews for more kinds of video"), third_party/photocraft-craft-libs.md (video section: no unsafe in craft-video / craft-media, findings above).
- **3c done:** `cs-geo/examples/try_video_place.rs` on 6 copies (5 generated + real 4500x3000 camera MP4): town → Keys/UserData GPSCoordinates + names read back; area → GPS and City removed; undo x2 → byte-identical. Tagged videos still decode (built-in helper exit 0, FFmpeg full decode 0 errors).
- **Item 4 closed:** birth year + "not before the birth year" rule already built (vision.rs BirthCheck). Age estimate dropped on evidence (research/age_eval_2026-11-21.txt: 7.9 y mean error vs 4.4 y for the whole-photo AI date). No child photos in the test set; revisit only with a public test set of children.
- **Item 6 closed:** downloads already use `RootCerts::PlatformVerifier` (cs-ai/download.rs, since 74ff1f0). Light theme looked at: Tools, People, Settings, Places + map: fine (Sort review step not checked).
- **Settings → AI models hardware check** already exists (GPU name + VRAM shown).
- **Release app tested (20:05):** `ChronoSort.exe --cs-video-poster` (windows subsystem) writes the PNG to a pipe, exit 0, no window. Browse on temp/cs_video: 13/13 supported clips get thumbnails, rotated MOVs upright, H.264 High 10 shows the video icon (no FFmpeg here). No helper processes left behind. dist/ChronoSort.exe (46.8 MB, stripped) + chronosort-cli.exe updated (local only, not uploaded).

## 2026-11-24: Places 3a "general location" + 3b map (not committed)
- `Places::regions(query, n)` in cs-geo/places.rs: unique region + country, the region name must start with the text before a comma, words after it narrow by country ("queensland, aus"); exact name first, then bigger (summed town population). Folded names precomputed → 3–5 ms on the real list (150 ms before). ExifTool's list has English names (Tuscany, Bavaria), not local ones (Toscana, Bayern).
- `Target::Region { name, country, code }` writes XMP-photoshop:State + Country + iptcCore:CountryCode, no GPS. **Region and Country now remove the finer old place** (`clear_finer`: GPS:all + XMP-exif GPS / Keys+UserData GPSCoordinates for video, City; Country also State). Before this, "country only" over a Paris photo left Paris GPS + City in it.
- `places_search` returns `regions`; Places.svelte lists them between towns and countries ("general area, no town"), confirm text per kind; Help updated.
- Real ExifTool test (`cargo run -p cs-geo --example try_region -- dist/exiftool/exiftool.exe <copies> paris tuscany`): Paris → Tuscany (GPS + City gone) → Italy only (State gone), undo ×3 → all 6 files byte-identical.
- **EXIF panel** (InfoPanel + meta.rs): the place search also lists areas and countries. Picking one writes the names, removes the old position and town (`place` with an empty city and no gps), and typed coordinates win over a picked area. Unit test + the real-ExifTool round trip extended: Adelaide → Tuscany removes the GPS + City, keeps the Title, two undos → exact bytes.
- **3b map preview DONE:** `WorldMap.svelte` + `src/lib/worldmap.js` (Natural Earth 110 m land, public domain, `third_party/natural-earth.md`). Town = pin (36° window); area / country = dashed ring around a population-weighted rough middle (`Region/Country.lat/lon`, never written). Theme tokens via color-mix, so all themes work. Shown in Places, in the EXIF summary (photo GPS) and when picking in the EXIF edit form.
- Places ordering: areas come first when the best area has more than twice the people of the biggest matching town and that town is under 1 million (tuscany/texas/new south/queensland/washington → area; new york/paris/adelaide → town).
- **Click-tested in the debug app** (cdp, 2026-11-24): Tuscany on 6 photos including one tagged Houston; the dialog said "1 photo already has a place; it will be replaced"; ExifTool read back State/Country/Code only (Houston GPS + City gone); Undo → all 6 byte-identical. Shots in temp/cs_shots/map_*.png (not in git). press_dialog.ps1 needs `powershell -ExecutionPolicy Bypass -File …` here.
- Not click-tested: the EXIF panel map/area pick in the viewer (covered by backend tests only). Full check.sh passed on the 3a state; after that clippy + cs-geo + app tests pass.

## PAUSED 2026-11-23 late (user: "save progress and where you left off and stop for today")
- **Git:** pushed up to decb2ff (criterball/ChronoSort). Near-duplicates + robustness test + strip + STATUS are **staged, NOT committed** (commit was blocked: user rule "no commits until asked"). Ask the user before committing / pushing. The user also said "don't upload just yet" about a compiled release (sizes: dist 116 MB, release zip ~27 MB, ~20 MB expected with strip).
- `scripts/check.sh` passes on the staged state. dist/ChronoSort.exe is from before near-duplicates (target/release has the near-dup build, not stripped yet).
- **SD card E: "CHRONOSORT"** (exFAT, 59.5 GB) holds a portable copy (E:\ChronoSort, with models); may be reused or reformatted.
- **Resume here: Next item 3 "Places, more"**, nothing edited yet. Plan:
  - 3a general location: `Places::regions(query, n)` (unique region+country from the ExifTool list, biggest population first) in crates/cs-geo/src/places.rs; `Target::Region { name, country, code }` in cs-geo/src/lib.rs writing XMP-photoshop:State + Country + iptcCore:CountryCode, no GPS; Region/Country targets also clear an old City (Country also State) when replacing; `places_search` returns `regions`; Places.svelte shows them between towns and countries (results list ~line 179-191, confirm text line 72-73) and the EXIF panel could offer them too.
  - 3b map preview: offline world outline (Natural Earth 110 m, public domain) embedded as SVG with a pin for the chosen town / region.
  - 3c places on real MP4/MOV: do it with the video work (item 5), using real clips.
- Then item 4 (birth year per person + age estimate from the vision model), then item 5 (video thumbnails with storytold craft-libs craft-video + craft-media, child process with time limit; user informed about H.264/HEVC patent pools).
- Item 2 (find faces after Sort analysis) turned out to exist already: Overview.svelte "Find the faces in these photos" -> People autostart.
- Test helpers: `temp/cs_scripts/cs.sh` (ev/key/shot via cdp.mjs on port 9333), `go.js` (browser navigation), `ai_time.js`, `make_demo.py`, `dup_ui.py`; demo photos `temp/cs_demo`, near-dup test set `temp/cs_neardup` (+ tool `temp/cs_mkdup`).

## 2026-11-23
- `scripts/check.sh` passes (fmt, clippy, all tests, linux/mac checks, audit). Release build → `dist/ChronoSort.exe` (starts OK).
- EXIF panel: a town picked from the offline list now also writes City / State / Country / CountryCode (same tags as Add a place); ✕ removes the position and the names. Typing coordinates by hand leaves existing names alone. Unit test + real ExifTool round trip cover it. Not yet in dist (built after the release build).
- Pushed b2df843 to criterball/ChronoSort at the user's request.
- **Zoom:** % and 1:1 now mean the REAL picture in SCREEN pixels (header size via `image_size`, devicePixelRatio). Zooming past the preview loads the original (`fthumb://<id>_o`, JPEG/PNG/GIF/BMP/WebP only, ≤ 256 MB); falls back to the preview on error. Verified: 960×1470 photo → Fit 69 %, 1:1 = 960 device px from `_o`.
- **Bug fixed (safety):** `recycle_bin()` said "bin available" for a `subst` drive letter (the volume APIs fail on it, so the per-drive NukeOnDelete check was skipped) → "Recycle Bin" would have deleted for good. Now resolves the real path first (canonicalize; mapped network letters become \\?\UNC → no bin) and fails safe (unknown = no bin). Test `subst_drive_follows_the_real_drive`; verified in the app (X: offers only Delete permanently).
- **Screenshots:** `docs/screenshots/browser/01–09` retaken with public-domain NASA photos (CREDITS.md lists every source). Demo set: `temp/cs_demo` built by `temp/cs_scripts/make_demo.py` from the face test data; shown via `subst X:` so the tree shows no private folders. The old dev shots (local folders, a portrait) are in `temp/cs_old_browser_shots`, not in git. README has a short Browse section with two shots.
- Careful: the app reopens the LAST folder; before any screenshot, navigate away from private folders first.
- **cs-image robustness test** (`crates/cs-image/tests/robustness.rs`): deterministic mutation fuzzer (truncation, bit flips, chunk drop/repeat, splices, huge size fields) over every save format + real HEIC/JPEG. 500 cases in debug, 3000 release; long run `CS_FUZZ_CASES=100000` = 0 panics, 0 hangs, slowest 176 ms. (`cargo fuzz` needs nightly + libFuzzer, not available on windows-gnu.)
- **Portable mode tested on a real SD card** (user's 60 GB card, formatted exFAT "CHRONOSORT" with the user's OK; UAC script `temp/cs_scripts/format_sd.ps1` checks disk 3 = USB SD reader first). Copy with models: 6.0 GB in ~4.5 min; all 107 data files + program byte-identical; run from E:: portable=true, data in E:\ChronoSort\ChronoSortData, PC app data untouched, window 0.4 s, AI engine ready 5 s + first answer 8 s (warm OS cache; cold from card estimated 1–2.5 min). Not tested: another PC.
- **Near-duplicates** (Find duplicates, tick box): cs-core `find_duplicates_ext` + 128-bit dHash from the 320 px thumbnail cache + aspect check (2 %), threshold 10 bits, sets only take photos matching ALL members; one file per content (identical copies stay in their own sets). Every file carries its own BLAKE3; cs-exec `DupAction.extras` now has a per-file expected hash (edited after the scan = skipped, test). UI: "similar" chip, sets left alone until "include this set", big preview follows the pointer, pixels + size per version, keeper = most pixels. Real test (temp/cs_neardup, NASA photos + resized / quality-30 / cropped copies): 4 near-dup sets + 1 identical set found, crop and another shot of the same crew NOT matched; move + Undo → all 12 files byte-identical.
- Release profile now `strip = true`.
- **Next agreed with the user:** items 2–4 of Next, then a new item: video thumbnails with storytold craft-libs `craft-video` (pure-Rust H.264 / HEVC Main10 / VP9 / AV1) + `craft-media` (MP4/MOV/MKV) instead of the FFmpeg download, decoded in a short-lived child process with a time limit. Mention HEVC/H.264 patent pools (user informed).

## PAUSED 2026-11-22 late (user: "stop for today")
- **Nothing committed or pushed.** The user asked about pushing, then decided to stop. The repo belongs to the **criterball** account (remote criterball/ChronoSort, commits as criterball's noreply). Before pushing, `gh auth switch` to criterball. Keep out `docs/screenshots/browser/01_thumbs_m.png` and `02_delete_no_bin.png`: they show the dev PC's own folders. Push only when the user says go.
- **Last fixes, not yet re-verified by a full check.sh:** clippy items in cs-image + fileops (test helpers moved before `mod tests`, collapsed if). `cargo clippy -D warnings` was clean afterwards; the final `cargo test --workspace` run was stopped before it finished. Next session: run `scripts/check.sh`, then the release build → `dist/ChronoSort.exe` (+ exiftool folder).
- **Latest UI changes, tested in the debug app:**
  - The viewer's top fly-out is taller (165×120 thumbnails with names, follows the current photo).
  - The bottom toolbar is fixed: it was renamed from `.bar` to `.vbar` because the global progress-bar `.bar` squashed it to 8 px.
  - Viewer window buttons: minimise (leaves full screen first, since Windows ignores minimise on a full-screen window, and returns to full screen on focus), restore / full screen toggle, close = back to the browser.
- The Vite dev server was stopped. If the debug app looks stale, kill any node on port 5173 and start `npx vite --force`.

## In progress 2026-11-22: File browser = start screen (FastStone-style look; nothing committed)
Plan: `~/.claude/plans/encapsulated-painting-hopcroft-*.md`. Clean-room study: `docs/ui-reference/FASTSTONE_STUDY.md` (screenshots git-ignored in docs/ui-reference/shots). User decisions: the app opens in Browse (old Home tiles = "Tools"), all files shown, Thumbnails S/M/L + Tiles/List/Details, housekeeping incl. permanent delete + Undo, photocraft (craft-libs, MIT/Apache) for image code only, v1 edits = rotate/flip, crop, resize, Save As + EXIF panel with edit mode; nice-to-haves chosen at the end.
- **Done + tested:**
  - `cs_platform::volumes()/known_folders()` (Windows/Linux/macOS, compile-checked for linux + mac).
  - `cs_platform::recycle_bin(path)`: policy NoRecycleFiles, drive type (removable/network/CD have no bin), per-volume `BitBucket\Volume\{GUID}\NukeOnDelete`. **On the dev PC C: and D: have the bin switched OFF and E: is removable**, so only "Delete permanently" is offered there (user's request). Verified against `mountvol` + registry.
  - `src-tauri/src/browse.rs`: list_dir (no content reads, 10 000 files < 1 s), list_subdirs, folder_samples, reveal_item, `fthumb://<id>_<s|m|l|p>` (ids only, never paths; cache key = blake3(path+len+mtime)).
  - `src-tauri/src/fileops.rs`: new folder, rename (name check incl. Windows reserved names), copy/move (never overwrite, " (2)" names, other drive = `cs_exec::copy_verified` then remove the original), Recycle Bin (only where available) / permanent delete, guards (drive roots, system/program folders, app data, own folder), undo stack (copies: removes only unchanged copies). 10 unit tests.
  - **Bug fixed (pre-existing):** Find duplicates "Recycle Bin" would have deleted for good on drives with the bin off. Backend now refuses with the reason; the UI option is disabled with an explanation.
  - UI: `Browser.svelte` (tree + preview + views + status bar + right-click + keys + drag-and-drop + job overlay), `TreeNode.svelte`, `ContextMenu.svelte`, `Viewer.svelte` (full screen, wheel/arrows, zoom, fly-outs left/right/top), `Icon.svelte` (own icons), `lib/paths.js`. App starts in Browse; header Browse + Tools; Esc → Browse, Ctrl+B, Ctrl+T.
  - Click-tested in the debug app (cdp + `temp/fs_ops_test`): rename (stem selected, "con.txt" refused), F8 new folder, Ctrl+C/V twice → " (2)" copies, Ctrl+Z, cut folder + paste + undo, Del with bin off → permanent dialog → deleted. Screenshots docs/screenshots/browser/.
- **Viewer + edits + EXIF (later the same evening):**
  - **Fly-outs:** user feedback: "Esc must return to the normal interface"; FastStone's bottom edge has the zoom/navigation toolbar (`shots/15_fs_bottom.png`; the pointer must touch the last pixel row).
    - Viewer.svelte now has 4 fly-outs: left = edit actions/forms, right = InfoPanel (EXIF / All tags / ✎ Edit), top = thumbnail strip, bottom = prev/next, "n / N", zoom −/%/+, 1:1, fit, rotate, crop, resize, save, info, exit, plus a Navigator mini-map when zoomed.
    - The **Tauri window** goes full screen (`core:window:allow-set-fullscreen`), not DOM fullscreen, so Esc reaches the viewer and returns to the browser (tested).
  - **`crates/cs-image`:** photocraft's `craft-image-codecs` + `craft-heif`, pinned rev (review: `third_party/photocraft-craft-libs.md`).
    - Open (upright), rotate/flip/crop/resize, save (never overwrites), lossless JPEG orientation math. 6 tests incl. real HEIC.
    - cs-thumb now decodes HEIC with it (no FFmpeg needed).
  - **`src-tauri/src/edit.rs`:**
    - Edit session plus `edit://` preview.
    - Save as copy (`<name>_edited.<ext>`) or replace. Replace keeps the original in `<folder>/.chronosort/edit-originals/<run>/` (not the Recycle Bin: works on every PC and Undo is reliable).
    - JPEG turn/flip-only edits are saved losslessly via the ExifTool Orientation tag. The original's file dates are kept.
  - **`src-tauri/src/meta.rs`:**
    - ExifTool read (all tags, `-G1`). Note: no `-q`, because it hides `{ready}` and caused a 60 s hang (fixed).
    - Write date taken / title / description / keywords / author / copyright / GPS (offline place search) with a verified backup in `.chronosort/info-backups/` and Undo.
  - fileops got `Undo::Replaced` + `Undo::MetaEdited`. Tests share the history behind `TEST_LOCK`; `places::exiftool()` uses `../tools/exiftool` in tests.
  - **App tests:** 19 pass. Real ExifTool round trip: write + undo is byte-identical. Lossless turn + crop/replace + undo is byte-identical.
  - **Click-tested in the app:** R R + Ctrl+S → lossless `portrait_edited.jpg` with the original time; crop 1:1 → 768×768; EXIF edit (title, keywords, Adelaide via place search) verified with ExifTool. Screenshots `docs/screenshots/browser/03–08`.
  - Help: new section "Browsing, viewing and editing" + shortcut groups. The Tools page has a Browse tile.
- **Not yet:**
  - Recycle-Bin path untested on a drive that HAS a bin (none on this PC).
  - Nice-to-have list for the user to pick from (FASTSTONE_STUDY.md §7).
  - (Done 2026-11-23: town names in the EXIF panel, zoom in real pixels, cs-image robustness test, release build + dist.)

## Next (re-checked against the code 2026-11-23)
Already built (found in code, was still listed): review decisions saved per folder (review.rs, content-hash checked), Forget everyone, QuickTime creationdate, XMP in JPEG, Takeout truncated sidecar names (cs-scan `truncated_takeout_partner`), stalled-download timeout (cs-ai IDLE), child-process guard (cs-platform/child.rs, Windows job object + Linux PDEATHSIG), video thumbnails via the ffmpeg pack, Open folder after a run.
1. Near-duplicates (resized / re-saved copies, bursts): perceptual hash in Find duplicates, user picks the keeper with the big preview.
2. Find faces right after Sort analysis (optional, only if face models are installed).
3. Places, more: "general location" (region/area without a town), map preview, places for videos tested on real MP4/MOV.
4. Birth year per person + age estimate from the vision model (faces + "looks about 5" -> year).
5. rustls-platform-verifier (use the OS certificate store for downloads); light-theme check.
6. Needs hardware / the user: portable mode on a real USB stick; Linux + macOS builds and testing (only compile-checked); Windows installer / release.
7. Git: https://github.com/criterball/ChronoSort (public, commits as criterball, push via gh auth switch). Screenshots ui2/17 + 18 are git-ignored (local model paths): retake before adding.

## New request (2026-11-20 15:52): AI models in Settings
The user wants Settings to **suggest AI models that fit this PC** and offer **download** etc.
Design, keeping the "fully offline" promise:
- **Hardware check** (no network): RAM ✅ DONE `cs_platform::total_ram()` (Win/Linux/macOS, tested; shown in `chronosort-cli info`); still TODO: (GlobalMemoryStatusEx / sysinfo), GPU + VRAM (DXGI adapter list on Windows, nvidia-smi if present, Metal/Vulkan later). Map to tiers: Max (VRAM ≥ 11 GB), Standard (6–10 GB), Light (< 6 GB or no GPU), Metadata-only.
- **Model catalogue** shipped inside the app (JSON): name, purpose (faces / vision), licence (only commercial-safe + GPL-compatible, e.g. Qwen3-VL Apache-2.0, OpenCV YuNet/SFace Apache-2.0; never InsightFace), size, RAM/VRAM needed, exact download URL(s) and **SHA-256**.
- Settings → "AI models": shows "Recommended for this PC" with a reason ("12 GB graphics memory → Qwen3-VL 8B"), what is installed, disk space needed.
- **Download is the only network action in the whole app**, and only when the user clicks Download. It's done by the Rust backend (never the UI: the CSP keeps the UI offline), resumable, written to `<name>.part`, verified by SHA-256, then renamed. Progress bar, Cancel, Delete model.
- **Offline alternatives:** "Import model file…" (from a USB stick or another folder, verified by checksum), and "Use existing file" (e.g. `001_Portable_Ai\models\Qwen3.5-9B-Q4_K_M.gguf` + mmproj).
- A clear "Never connect to the internet" switch that hides/disables Download entirely (default ON for elderly users? ask the user).
- Models live in the app data folder (or a user-chosen folder), outside the photo folders.
- Open question for the user: the README promise was "no internet required at all". Download stays optional and user-triggered; import from file covers fully offline PCs.

## AI notes (user asked "what about the AI bit?")
- Found a local vision model: `001_2026_11_02_Portable_Ai\models\Qwen3.5-9B-Q4_K_M.gguf` + `mmproj-BF16.gguf` (Qwen3.5 has vision). llama.cpp b10276 (Vulkan) in tools\llama-bin. RTX 3060 12 GB fits it.
- Plan: Phase 2 faces (YuNet + SFace ONNX via `ort`, Apache-2.0), then Phase 3 llama-server sidecar for printed date stamps / handwriting / era guesses on review items only. First step: a quick experiment running llama-server with that model on test scans.
- Later TODO: iPhone quicktime creationdate; XMP in JPEG; Takeout truncated names; near-duplicates (perceptual hash); video thumbnails (ffmpeg helper).

## Older next-items
- UI: keyboard flow in Review (arrows, Enter = accept, number keys), a big preview on click, light-theme check, folder-open button after a run (opener plugin, local only).
- Video thumbnails + HEIC/RAW full previews via a bundled ffmpeg/libheif helper process (out of process, as per the security design).
- Persist review decisions (resolutions) in the DB so a long review can be resumed.
- Then Phase 2: faces (YuNet + SFace via ort).
- Later TODO: iPhone `com.apple.quicktime.creationdate`; XMP in JPEG; Takeout truncated names; near-duplicates.
