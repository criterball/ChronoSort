# Natural Earth 1:110m land (v5.1.2)

- **Source:** https://github.com/nvkelso/natural-earth-vector, tag v5.1.2, `geojson/ne_110m_land.geojson` (138 160 bytes, SHA-256 `9e0729ee253ca7d7a5c4ae9395fb1902264c5377c52e224d13dd85010e2835d9`). Fetched 2026-11-24.
- **License:** public domain (https://www.naturalearthdata.com/about/terms-of-use/). No attribution required; credited here anyway.
- **Used as:** `apps/chronosort-app/src/lib/worldmap.js`: one SVG path (equirectangular, 0.1° rounding, 55 KB), drawn by `WorldMap.svelte` for the place preview. Data only, no code; nothing is loaded at run time, the app stays offline.
- **Regenerate:** convert the GeoJSON with a small script (x = lon + 180, y = 90 − lat, rings of fewer than 3 points dropped). The script is not kept; the conversion is 25 lines and described here.
