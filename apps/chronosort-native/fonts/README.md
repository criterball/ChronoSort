# ChronoSort UI font

`ChronoSortUI-Regular.ttf` / `ChronoSortUI-SemiBold.ttf` = **Inter 4** (text) + **131 Tabler Icons 3.50.0**
(icons), built by `build_font.py` and embedded in `ChronoSortNative.exe` (nothing is installed on the PC).

| Source (in `src/`) | Licence | SHA-256 |
|---|---|---|
| Inter-Regular.ttf (craft-libs 77c7c62 `crates/ui-contract/fonts/inter`) | SIL OFL 1.1, no Reserved Font Name (`src/Inter-OFL.txt`) | 5f3bb53a5fb9d1281725820400cfa38fd68d1dc6855ce956c0e549f74f8ee379 |
| Inter-SemiBold.ttf (same) | SIL OFL 1.1 | 74d7f819ee687bab291cb9272750cea9c701a169b500f6cc461645a4461bd6a5 |
| tabler-icons.ttf + .css (npm `@tabler/icons-webfont@3.50.0`) | MIT (`src/Tabler-LICENSE.txt`) | b94165ceaa38041acd2b8bf842f637b58f3bdba7f1186ca9a98fd94c511f5d04 |
| tabler-icons-filled.ttf + .css (same package) | MIT | 143c3bf87336e9cafb400094a581fb6a054539ea3c93c4c6cf00ccf2024a1adb |

Rebuild after changing the icon list (`ICONS` in `build_font.py`):

    set PYTHONPATH=D:\Claude_Code_Home\tools\pylibs        (fontTools 4.60.1)
    D:\Claude_Code_Home\tools\python-full\python.exe build_font.py

This writes both fonts and `../src/icons.rs` (one constant per icon). See every icon with
`ChronoSortNative.exe --icons`.

What the build does: icons keep Tabler's Private Use Area code points; symbols Inter lacks (✓ ✕ ★ ☆ ▶ ⏸ ● ⚠ ⚙
♪ 🔒 📍 ▲ ▼ …) are mapped to icons so plain text works; icons are scaled to 1.32 × cap height, centred on the cap
height, square advance. The family is renamed "ChronoSort UI" (a modified OFL font gets its own name). The OFL
and MIT licence texts ship in `dist\licenses\`.

Not covered (needs another font later): Chinese / Japanese / Korean, Arabic, Hebrew, Thai, Indic scripts, colour emoji.
