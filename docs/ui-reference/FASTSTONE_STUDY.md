# FastStone Image Viewer: clean-room look-and-feel study

Date: 2026-11-22. Subject: FastStone Image Viewer 8.5, the copy installed at `D:\Claude_Code_Home\FastStone Image Viewer`.
It was run only on **copies** of our own test images (`temp\fs_study`: public-domain portraits + the generated corpus).

**Clean-room rules.**
- This file describes only what can be seen and used on screen: layout, behaviour, key bindings. It is written in our own words.
- No FastStone code, icons (`FSIcons.db`, `FSMenuIcons*.db`), logo, help text, DLLs or plugins were opened or used. Nothing was decompiled.
- The menu and dialog structure was read from screenshots of the running program.
- The screenshots are in `shots/` (git-ignored, private, never published).
- ChronoSort's UI is designed from this description, with our own icons, wording and code. Image code comes from photocraft (MIT/Apache-2.0).

---

## 1. Main window ("browser")

```
┌ title: "<selected file> - <app name>" ───────────────────────────────────────────────┐
│ menu bar: File Edit Colors Effects View Tag Rating Favorites Create Tools Settings Help│
│ main toolbar (one row of ~28 small icons) ............ layout buttons (4) at far right│
├──────────────────────┬────────────────────────────────────────────────────────────────┤
│ Folder tree          │ browser toolbar: ← → ↑ | folder actions | ✕ | view: ▦ ▥ ☰ |   │
│ (whole PC: drives,   │   [Images & Videos ▾] [Filename ▾] [Search ▾]                   │
│  folders, lazy +/-)  │ address bar (editable path, history drop-down)                 │
│                      ├────────────────────────────────────────────────────────────────┤
│  ~20 % width,        │ thumbnail grid, left to right, wrapping                        │
│  ~70 % height        │   each tile: rounded frame, image centred,                     │
├──────────────────────┤   bottom strip inside the frame: "960x1280"  …  "JPG"           │
│ "Preview" pane       │   file name under the frame                                    │
│ (selected image,     │   folder tiles: a yellow folder with 4 sample photos inside    │
│  fitted, grey bg)    │   selected tile: thick blue frame                              │
│ info line: WxH (MP)  │                                                                │
│  bits · type · size  │                                                                │
│  · date · 1:1 · fit  │                                                                │
├──────────────────────┴────────────────────────────────────────────────────────────────┤
│ status bar: "file [ 1 / 8 ]" | "0 Folders" | "8 Files (2.03 MB)" | "1 Selected (266 KB)"│
└───────────────────────────────────────────────────────────────────────────────────────┘
```
Screens: `shots/01_browser_default.png`, `02_browser_folder_selected.png`.

Observed details worth copying (as behaviour):
- **Folder tree** shows the whole PC with expand boxes; the current folder is highlighted. The tree follows when you double-click a folder in the grid.
- **Folder tiles** in the grid show up to 4 small sample photos from inside the folder ("4 Sample Thumbnails"; a setting).
- **Thumbnail label strip** shows the pixel size on the left and the **real** file type on the right. A PNG named `.jpg` is shown as "PNG": the type comes from content, not the extension (`08_browser_misc.png`).
- **Preview pane** (bottom-left, collapsible with a » button): the fitted image, with one info line under it: `960 x 1280 (1.23 MP) 24bit JPG 266 KB 2026-11-22 20:22:18`, plus 1:1 and fit buttons.
- **Status bar:** current file with position "[ 1 / 8 ]", folder count, file count with total size, selected count with size.
- **Main toolbar** (left → right, by tooltip/meaning): acquire, open, save, prev, next, zoom in/out, actual size, Smooth checkbox, position "1 / 8", zoom % box, hand/scroll tool, full screen, slideshow-ish actions, colours, rotate left/right, resize, crop, draw, email, print, settings. At the far right are **4 layout buttons**: browser with tree left / browser with tree right / large-preview layout / full screen.
- **Browser toolbar:** back, forward, up, refresh-ish folder actions, copy to / move to folder, delete (red ✕). There are three view buttons: **Thumbnails / Details / List**. Then a **filter** drop-down (default "Images & Videos"; other choices include all files), a **sort** drop-down (default "Filename"), and a **search** box with history.

## 2. View modes (`04_view_details.png`, `05_view_list.png`)
- **Thumbnails:** the grid described above. Thumbnail size is a **setting** (default 120 x 90, chosen from a list), not a toolbar button.
- **Details:** a table with columns Filename (with a small type icon) | Type | Dimension | Size | Date. Selection is a full-row blue bar.
- **List:** small icon + name, in columns.

ChronoSort plan: Thumbnails **S / M / L** directly on the toolbar (user's request), plus Tiles, List and Details.

## 3. Menus (structure only; `m_*.png`, `s_*.png`)
| Menu | Items (shortcut) |
|---|---|
| File | New (Ctrl+N), Open (Ctrl+O), Save As (Ctrl+S), Reload (Shift+R), Email (F6), Print ▸, Import photos and videos, Screen capture ▸, Acquire from scanner, Exit (**Esc**) |
| Edit | Undo (Ctrl+Z), Redo (Ctrl+Y), Edit with external program ▸, Select ▸, Copy image (Ctrl+C), Paste image (Ctrl+V), **Rotate ▸** (Left L, Right R, Flip H, Flip V, Straighten/any angle Alt+R), **Resize/Resample (Ctrl+R)**, Canvas size (Ctrl+K), Set DPI, **Crop board (X)**, Draw board (D), Clone and heal (O), Red-eye removal, JPEG comment (T) |
| Colors | Auto-adjust (Ctrl+Shift+B), Lighting (Ctrl+T), Colours (Ctrl+E), Levels (Ctrl+L), Curves (Ctrl+M), Sharpen/Blur (Ctrl+U), Reduce noise (Ctrl+J), Swap colours ▸, Reduce colours ▸, Count colours, Grayscale (Ctrl+G), Sepia, Negative (Ctrl+I) |
| Effects | Border, Frame mask, Watermark, Bump map, Sketch, Oil painting, Lens, Obscure/blur (Ctrl+Shift+O) |
| View | Full screen (Enter), Layout ▸ (tree left / tree right / windowed), Browser style ▸ (Thumbnails / Details / List), Preview (toggle), Slide show (S), Open in file explorer (W), Histogram (Ctrl+H), **Image properties (EXIF) (I)**, File properties (Alt+I), Magnifier ▸, **Zoom ▸** (Actual size A, Best fit B, Fit width Shift+W, Fit height Shift+H, 100–900 % = keys 1–9, Best fit 0, Zoom in +, Zoom out −), Refresh (F5), Shuffle (F7) |
| Tag | Enable tagging, Tag/untag (Q), Untag all, Select tagged/untagged only, Tag filter ▸ |
| Rating | Enable rating, Set 1–5 (Alt+1…5), Remove rating (Alt+0), Rating filter ▸ |
| Favorites | Add folder to favorites, Organize favorites |
| Create | Contact sheet (Alt+C), Multi-page file builder/splitter, Image strip (Alt+T), ZIP archive (Alt+Z) |
| Tools | Batch convert (F3), Batch rename (F4), Batch dialog, Change timestamp, **JPEG lossless rotate ▸** (Ctrl+Alt+L/R/H/V, Auto-rotate by EXIF orientation Ctrl+Alt+A), Remove JPEG metadata, Compare images (P), Wallpaper ▸ |
| Settings | Language ▸, Theme ▸ (Bright / Gray / Dark), thumbnail database tools, Settings (F12) |
| Help | Help (F1), tutorial, licence, registration, donation, home page, about |

**Right-click menus** (`c_thumb.png`, `c_tree.png`, `c_background.png`):
- **On a file:** Full screen, Slide show, Edit with external program ▸, Open in file explorer, Refresh / complete refresh, Shuffle, Tools ▸, Select ▸, **Copy to folder… (C)**, **Move to folder… (M)**, Email, Print ▸, Create ▸, Copy image, **Copy / Cut / Paste**, **New folder (F8)**, **Delete (Del)**, **Rename (F2)**, File properties (Alt+I).
- **On a tree folder:** Open in file explorer, Refresh, Add to favorites, Set root here, Copy to folder…, Move to folder…, Copy / Cut / Paste, New folder, Delete, Rename (F2), Import photos here, Folder properties.
- **On the empty background:** Full screen, Open in file explorer, Refresh, Complete refresh, Shuffle, Select all (Ctrl+A), Paste, New folder (F8), Import here, Folder properties.

## 4. Full-screen viewer (`10_fullscreen.png`, `11–14_fs_*.png`)
- Black background with the image fitted. Top-left is a small green info line: `name ( W x H = MP , size ) [ n / total ] zoom%`.
- **Fly-out menus at the screen edges** (the user also pointed these out on 2026-11-22): moving the mouse to the far left, far right or top makes a panel slide out over the image, and it hides again when the mouse leaves. This is how all viewer actions are reached without leaving full screen:
  - **Left edge:** an action panel grouped as *File & slideshow* (open, save as, copy, print, slideshow, external editor), *Rotation, resize & text* (rotate L/R/straighten, flip H/V, resize, quick resize, canvas size, crop board, draw board, clone/heal, red-eye), *Colours* (auto, lighting, colours, levels, curves, sharpen, noise, grayscale, sepia, negative, colour depth), *Effects*, and Undo/Redo at the bottom.
  - **Right edge:** "File and attributes": name [n/total], location, type, size, date/time, pixel size + bit depth, print size + DPI. Below that are tabs **EXIF | Histogram | JPEG comment**. The EXIF tab is a two-column table: Make, Model, Software, Date taken, Exposure time, F number, ISO, Exposure program, Metering, Exposure bias, Flash, Focal length, 35 mm equivalent, Lens model, GPS.
  - **Top edge:** a slim browser: the address bar, a horizontal thumbnail strip of the folder, a mini toolbar, and a status line (folder tree toggle, counts, Image properties).
  - **Bottom edge** (re-checked 2026-11-22 at the user's request; the pointer must touch the very last pixel row; `shots/15_fs_bottom.png`): a toolbar slides up along the bottom-left. Left to right it holds:
    - acquire, open, save
    - **previous / next**
    - **zoom in / zoom out / actual size (1:1)**, a "Smooth" checkbox
    - a **position box "1 / 8"** and a **zoom % box with a drop-down** ("84%")
    - a hand/pan tool
    - slideshow, resize, crop board, draw board, colour adjust, clone/heal, **rotate left / right**, compare, wallpaper, scanner, email, print, settings
    - **exit full screen** (last button)

    Above its left end is a collapsible **"Navigator"** tab, a small overview map of the zoomed picture.
- **Esc** in full screen goes back to the browser (the normal window), not out of the program.
- Mouse/keys (from settings + menus): wheel = previous/next image, Enter/double-click = enter/leave full screen, 1–9 zoom, A/B/0 fit, +/−.

## 5. Dialogs
- **Resize/Resample** (`20_dlg_resize.png`): original vs new size (pixels + MP). Three exclusive modes: **Pixels** (W x H), **Percent**, **Print size** (W x H in inch/cm + DPI). Also **Preserve aspect ratio**, a **Filter** (Lanczos3 default) and OK/Cancel.
- **Crop board** (`22_crop_board.png`): its own resizable window, titled with name, size and zoom. Large image on grey, with a rubber-band selection. Bottom bar: selection position and size boxes, a "pick other size" preset list, **paper ratio** (Free hand + presets), rotate L/R, grid toggle, outside-darkness (OD) slider, prev/next image, fit. Actions: **Lossless crop to file** (JPEG), Crop, Crop to clipboard, Close.
- **Image properties** (`21_dlg_properties.png`, `25_properties_exif.png`): the same content as the right full-screen panel, as a window with Close. It is **read-only**; the only metadata edits are the JPEG comment, change timestamp and remove metadata.
- **Settings** (`23_settings.png`, `24_settings_thumbnail.png`): tabs Viewer, Thumbnail, Magnifier, JPEG, RAW, PDF, Video, Dual monitor, CMS, Associations, Favorites, Programs, Music. Notable options: show hidden files, confirm on overwrite, auto-rotate by EXIF, mouse wheel action, single-file delete goes to the Recycle Bin with confirmation, drag-and-drop = move on the same disk. Thumbnails: size, border shape, colours, show dimension/filename/tooltip, folder thumbnail style, video thumbnail with film-frame border, and a thumbnail cache database location.

## 6. Look
- Classic Windows light-grey chrome ("Bright" theme; Gray and Dark exist), small 16 px colour icons and dense toolbars.
- Off-white grid background, rounded light-grey tile frames, a blue selection frame, file names in small black text.
- Full screen is pure black with a green info text.

---

## 7. ChronoSort v1: what we take (Basic) and what waits (Nice to have)

### Basic (v1), being built now
| Area | ChronoSort v1 |
|---|---|
| Layout | Tree top-left, Preview + info line bottom-left (collapsible), big file area right, status bar. Layout buttons: tree left / tree right. |
| Tree | Drives (with labels, USB marked), quick places (Pictures, Desktop, Downloads, Home), lazy expand, follows navigation. |
| Toolbar | Back, Forward, Up, address bar, refresh. Views **Thumbnails S / M / L**, Details, List. Show: **All files** (default, user's wish) / Photos & videos. Sort by name, date, size, type. Search/filter box. |
| Tiles | Image centred in a rounded frame, size + **real type** label strip, name under it. Folder tiles with up to 4 sample photos. Non-media files get our own type icons. |
| Details | Name, Type, Dimensions, Size, Date taken / Modified. |
| Housekeeping | Copy to / Move to folder…, Copy/Cut/Paste, drag-and-drop, New folder (F8), Rename (F2), Delete → Recycle Bin (Del), **Delete permanently (Shift+Del)**, Undo (Ctrl+Z). |
| ChronoSort actions | Right-click a folder: **Sort this folder…, Find duplicates here, Find people here, Add a place here**. |
| Viewer | Enter/double-click = full screen, black background, info line, wheel/arrows = prev/next, zoom keys (1, 0/B, +, −), hot edges: **left = edit actions**, **right = file + EXIF panel**, top = thumbnail strip. |
| Edits | Rotate L/R, Flip H/V (JPEG: lossless via orientation tag), **Crop board** (free + ratio presets, prev/next), **Resize** (pixels / percent, keep aspect), **Save As / convert** (JPEG, PNG, TIFF, WebP + quality). Save as a copy by default. |
| EXIF panel + edit mode | The same fields as above plus a full tag list. **Edit mode**: date taken, title/description, keywords, author/copyright, GPS (place search). Backups + Undo. *(FastStone can't do this beyond the JPEG comment and timestamp.)* |
| Themes | Our existing themes + a new dark "Graphite" (viewer-style). |

### Nice to have (later, for the user to choose)
1. Colour tools: auto-adjust, lighting, levels, curves, colour balance, sharpen/blur, noise reduction, grayscale/sepia/negative.
2. Straighten / rotate by any angle; canvas size; set DPI.
3. Red-eye removal, clone and heal, draw/text board.
4. Effects: border, frame, watermark, etc.
5. Slideshow (with transitions and music).
6. Compare 2–4 images side by side (good for duplicate decisions).
7. Tagging (Q) and 1–5 star ratings with filters.
8. Batch convert / batch resize / batch rename (F3/F4).
9. Histogram tab; magnifier; fit width/height; 100–900 % zoom keys.
10. Favorites folders; "set root here".
11. Contact sheet, image strip, multi-page files, ZIP archive.
12. Print, email, wallpaper, screen capture, scanner import, import from camera/card.
13. Lossless JPEG crop; remove metadata; JPEG comment.
14. Dual-monitor full screen; colour management (ICC).
15. Thumbnail cache database tools (pre-scan folders).
