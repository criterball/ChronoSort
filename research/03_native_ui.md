# 03 — Native Rust UI (no WebView2 / Chromium)

Decided with the user 2026-11-27: ChronoSort is always portable and everything lives in its own folder.
WebView2 (a system part, plus a profile in %LOCALAPPDATA%) does not fit that, so the web UI is replaced by a
native Rust UI built on the storytold / ArtCraft stack. The user suggested FilmCraft for video playback.

## Backup of the last WebView2 version
`D:\Claude_Code_Home\backups\ChronoSort_2026-11-27_tauri_webview\` — `app\` (working dist 15:11), `source\`
(incl. .git and the uncommitted item 12), `chronosort-history.bundle`, README.txt. The Tauri app stays in the
repo and keeps working until the native app does everything.

## What WebView2 did, and what replaces it
| Web UI job | Native replacement |
|---|---|
| Screens (35 Svelte components, ~9.5k lines) | egui (storytold fork, rev 8a3c5ec = what craft-libs 77c7c62 uses); craft-ui widgets / craft-theme later |
| `<video>` / `<audio>` playback (viewer, slideshow) | craft-video + craft-media decode (already in cs-video), craft-audio decode, cpal output as audio master clock (FilmCraft `apps/filmcraft/src/audio.rs` approach), FilmCraft `crates/platform/media_foundation.rs` for hardware decode if software is too slow |
| `window.print()` | craft-print (PDF → system spooler); our own PDF writer (create.rs) makes the pages |
| Canvas text (watermark, contact sheets) | craft-text + craft-fonts (ship one open font) |
| `fthumb://` / `media://` protocols | gone — loader threads decode straight into textures |

## Third-party review (docs/THIRD_PARTY_POLICY.md)
- craft-libs UI crates (ui, ui-contract, ui-widgets, ui-shell, ui-patterns, ui-panels, ui-canvas, theme, host,
  cloud-ui, cloud, esign): no network code (no reqwest/ureq/hyper/TcpStream). craft-cloud is data types only.
- craft-settings defaults to %APPDATA% unless given `Location::Dir` → always pass `<exe>\ChronoSortData\settings`.
- craft-ui-shell keeps window placement in memory unless given a store; eframe `persistence` feature is OFF.
- FilmCraft is "young and moving fast" (19k stars): pin revs, copy only what we need, review each file.
- License: craft-libs / FilmCraft / egui fork are MIT OR Apache-2.0 → fine inside GPL-3.0-or-later.

## Layout
- `apps/chronosort-native` (bin `ChronoSortNative`): data.rs (ChronoSortData only, refuses a read-only folder),
  browse.rs (listing; same thumbnail cache key as the Tauri app, so both share `ChronoSortData\cache`),
  loader.rs (worker threads, newest request first, generation drop), viewer.rs, main.rs (tree + grid).
- Root Cargo.toml: `[patch.crates-io] winit` = storytold fork (egui fork needs it; Tauri uses tao, unaffected).
- Later: move the Tauri commands' logic (src-tauri/src/*.rs) into UI-free crates so the native app calls it.

## Steps
1. Prototype: tree + grid S/M/L + viewer (fit/zoom/pan/next/prev, preview then original, neighbours ahead). ← now
2. Video + sound playback in the viewer (FilmCraft approach), then slideshow with music.
3. User compares look + speed with the Tauri app → go / no-go.
4. Port screen by screen (Home, Sort flow, Duplicates, People, Places, Settings, Help, dialogs …).
5. One-time move of old AppData data (6 GB) into ChronoSortData with progress + verify.
6. Remove the Tauri app when the native one does everything.
