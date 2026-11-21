# ChronoSort — Roadmap

Each phase ends with a release that works and is safe. AI is never needed for the core to work.

## Phase 0 — Foundations
- Cargo workspace, `cs-platform`, `cs-model`, `cs-db` skeleton, CLI shell.
- Synthetic test photo set with a generator (`testdata/`). It covers every edge case in ARCHITECTURE §4 and §6.
- CI scripts: fmt, clippy, tests, cross-target `cargo check`, `cargo deny`.
- **Done when:** a Windows build compiles and `cargo check` passes for the Linux and macOS targets.

## Phase 1 — Safe metadata sorting (MVP)
- Scan + grouping (RAW+JPG, Live Photos, sidecars).
- Metadata: EXIF, XMP, QuickTime, Takeout JSON, filename patterns, folder patterns.
- Evidence engine + sanity rules.
- Plan: layouts (`YYYY`, `YYYY\YYYY-MM`), naming templates (keep / `YYYY-MM-DD_HH-MM-SS` / custom), collision handling.
- Safe exec: copy/move, verification, timestamps, journal, resume, undo.
- Exact-duplicate detection.
- Tauri UI: setup, review grid, dry run, run, report.
- **Done when:** 100% of the test set lands where expected; a killed run resumes cleanly; undo restores everything; source hashes never change.

## Phase 2 — Faces
- YuNet + SFace through `ort` (DirectML / CUDA / CPU).
- Clustering → People screen (name the groups, merge, split).
- Per-person, per-year appearance model, built from confirmed photos.
- Age estimation (find a model with a permissive license, or train our own).
- Suggests years for undated photos. Batch confirm propagates.
- Optional: export face regions as XMP MWG sidecars.

## Phase 3 — Vision LLM
- `llama-server` supervisor, hardware tier auto-detect, model manager (local files only).
- Tasks: printed date stamp, handwriting on scanned backs, era estimate. Output follows a strict JSON schema.
- Near-duplicate detection (perceptual hash).

## Phase 4 — Places
- Bulk GPS: country / city / map pin per batch, same-day GPS copied from phone photos, GPX/KML import.
- Offline reverse geocoding (GeoNames) and an offline map (MapLibre + PMTiles packs).
- Written to the copy's metadata or XMP only. Originals are never touched.

## Phase 5 — Linux and macOS releases
- Platform-specific testing; AppImage / .deb and a signed .dmg.
- Metal and Vulkan backends tested.
