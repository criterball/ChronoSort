# ChronoSort — research notes (2026-11-20)

Goal: fully offline app. Scans a source tree, sorts photos and videos into year folders.
Safety first. Local AI for undated photos. Windows first, then Linux and macOS.

## Projects to learn from / reuse

### Sorting and EXIF
| Project | Use | License |
|---|---|---|
| ExifTool (exiftool.org) | Reads/writes metadata for nearly every photo, video and RAW format. Built-in offline geolocation database (GeoNames cities). Geotags from GPX files. | Perl Artistic / GPL. OK to bundle. |
| pyexiftool | Python wrapper. Keeps one ExifTool process running (`-stay_open`), so batches are fast. | BSD/GPL |
| ivandokov/phockup | Python + exiftool. Sorts into year/month/day, has an `unknown` folder, copy/move. Small, readable reference. | MIT |
| seancummins/sortphotos | Takes the oldest date across all metadata tags. Has flexible folder format strings. | MIT |
| jmathai/elodie | Sorts by date and location. Its location lookup needs an online API, so don't copy that part. | Apache-2.0 |
| google-photos-takeout-helper (GPTH) | Matches Takeout `.json` sidecars to their photos. | Apache-2.0 |
| osxphotos | Reads the Apple Photos library on macOS (later). | MIT |

### Faces
| Project | Use | License |
|---|---|---|
| OpenCV Zoo YuNet (detect) + SFace (recognise) | ONNX, fast on CPU. digiKam's default since 8.5. Commercial use OK. 128-d embeddings, cosine ~0.36+ match. | Apache-2.0 |
| InsightFace buffalo_l (SCRFD + ArcFace) | More accurate, 512-d. Used by Immich. **Models are for non-commercial use only.** | Code MIT, models research-only |
| digiKam | Desktop reference for the face workflow: detect, cluster, name, confirm/reject. | GPL (learn, don't copy code) |
| Immich (immich-app/immich) | Best UI reference for people/face clustering and timelines. | AGPL (learn, don't copy code) |
| PhotoPrism | Detection → embedding → DBSCAN clustering pipeline, documented. | AGPL (learn only) |
| MiVOLO | Age estimation from face + body. Authors discourage ONNX for batches. Check license. | Check |

### Dating photos without metadata
- DEW dataset ("Date Estimation in the Wild", 1930–1999) + EauDeData/DEW-NDCG. Research only. Accuracy is about decade level.
- Local vision LLM on llama.cpp (`llama-server -m model.gguf --mmproj mmproj.gguf`). Qwen3-VL has been supported since late 2025. Use it for: printed date stamps, handwriting on scanned backs, guessing the era.
  - Watch: llama-server vision encode was reported slow for some models (issue #22582). Benchmark llama-server against llama-mtmd-cli.

### Duplicates
- Exact: hash (BLAKE3 / xxHash for speed, SHA-256 stored in the journal).
- Near-duplicate: perceptual hash (imagehash, BSD) or the czkawka engine (MIT).

### GPS / location (phase 2)
- ExifTool `-geolocation` (offline city database) and `-geotag` from GPX tracks.
- reverse_geocoder (GeoNames cities, offline).
- Offline map: MapLibre GL / Leaflet + Protomaps PMTiles. Ship a low-zoom world basemap and offer per-country packs. Fonts/glyphs must be bundled too.
- Ideas: pick a city / country / pin per batch; copy GPS from same-day phone photos to camera photos; import GPX/KML.

### UI
- Recommended: Python core + web UI (Svelte or React) in a native window.
  - pywebview (BSD): WebView2 on Windows, WebKit on macOS, GTK/Qt on Linux. One backend language.
  - Tauri 2: smaller and more polished, but adds Rust plus a Python sidecar.
  - Electron: biggest download, but the most proven.
- Component ideas: a virtualised thumbnail grid, PhotoSwipe lightbox (MIT), a shadcn-style component kit, light/dark themes.
- Reference UIs: Immich (timeline, people), digiKam (face confirm), Apple Photos (calm, minimal).

## Sources
- https://github.com/ivandokov/phockup
- https://github.com/seancummins/sortphotos
- https://github.com/mathiasvr/elodie
- https://www.insightface.ai/solutions/face-recognition-licensing
- https://huggingface.co/opencv/face_recognition_sface
- https://linuxtoday.com/blog/digikam-8-5-released-with-face-management-improvements-color-label-usability
- https://docs.photoprism.app/developer-guide/vision/face-recognition/
- https://github.com/WildChlamydia/MiVOLO
- https://arxiv.org/pdf/2106.05618 , https://github.com/EauDeData/DEW-NDCG
- https://github.com/ggml-org/llama.cpp/blob/master/docs/multimodal.md
- https://github.com/ggml-org/llama.cpp/issues/22582
- https://github.com/ideoforms/reverse-geocoder , https://github.com/fg1/exif-revgeo
- https://docs.protomaps.com/pmtiles/
- https://pypi.org/project/czkawka/
