"""Build the ChronoSort UI font: Inter (text) + Tabler icons, in one file per weight.

    PYTHONPATH=D:\\Claude_Code_Home\\tools\\pylibs  python  build_font.py

Inputs (fonts/src, licences beside them):
  Inter-Regular.ttf / Inter-SemiBold.ttf  Inter 4, SIL OFL 1.1 (no Reserved Font Name), copy from craft-libs
                                          77c7c62 crates/ui-contract/fonts/inter
  tabler-icons.ttf / -filled.ttf + .css    @tabler/icons-webfont 3.50.0, MIT (the Tabler version craft-libs pins)

Output:
  fonts/ChronoSortUI-Regular.ttf, fonts/ChronoSortUI-SemiBold.ttf  (embedded in the exe with include_bytes!)
  src/icons.rs                                                     (one `pub const NAME: &str` per icon)

Every icon keeps Tabler's own Private Use Area code point. The symbols the UI writes as plain text
(✓ ★ ▶ ⚠ …) are also mapped to an icon when Inter has no glyph for them, so text like "★★★" just works.
Icons are scaled to the text, centred on the cap height and given a square advance, so they line up
with letters in buttons and labels. The family is renamed "ChronoSort UI" (OFL: a modified font gets its
own name).
"""

import re
import sys
from pathlib import Path

from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont, newTable
from fontTools.ttLib.tables._c_m_a_p import CmapSubtable

HERE = Path(__file__).resolve().parent
SRC = HERE / "src"

# (Rust name, tabler name). "-filled" names come from the filled set.
ICONS = """
back:arrow-left forward:arrow-right up:arrow-up down:arrow-down
chevron_left:chevron-left chevron_right:chevron-right chevron_up:chevron-up chevron_down:chevron-down
caret_right:caret-right-filled caret_down:caret-down-filled
rotate_left:rotate rotate_right:rotate-clockwise flip_h:flip-vertical flip_v:flip-horizontal
crop:crop resize:resize save:device-floppy zoom_in:zoom-in zoom_out:zoom-out
play:player-play-filled pause:player-pause-filled stop:player-stop-filled
skip_back:player-skip-back-filled skip_forward:player-skip-forward-filled
volume:volume volume_off:volume-off repeat:repeat music:music
plus:plus minus:minus slideshow:presentation histogram:chart-histogram
star:star star_filled:star-filled star_off:star-off print:printer wallpaper:wallpaper
compare:layout-columns tag:tag tag_filled:tag-filled undo:arrow-back-up redo:arrow-forward-up refresh:refresh
thumbs:layout-grid tiles:layout-list list:list details:list-details
sidebar:layout-sidebar preview:eye search:search
pc:device-desktop drive:server usb:usb network:network disc:disc sd_card:device-sd-card camera:camera
home:home pictures:photo desktop:device-desktop downloads:download documents:file-text videos:movie
sort:arrows-sort sort_asc:sort-ascending sort_desc:sort-descending
open:folder-open explorer:external-link calendar:calendar duplicates:copy people:users person:user
face:face-id pin:map-pin pin_filled:map-pin-filled world:world map:map
fullscreen:maximize close:x check:check check_circle:circle-check
tools:tool settings:settings help:help-circle info:info-circle warning:alert-triangle
error:alert-octagon lock:lock exit:logout
folder:folder folder_filled:folder-filled image:photo raw:aperture video:movie audio:file-music
archive:file-zip program:app-window document:file sidecar:file-code
trash:trash cut:cut paste:clipboard rename:pencil edit:edit copy_to:copy move_to:arrow-move-right
favourite:heart favourite_filled:heart-filled filter:filter sparkles:sparkles ai:brain
sun:sun moon:moon palette:palette text_size:typography adjust:adjustments wand:wand brush:brush
droplet:droplet contrast:contrast clock:clock cake:cake keyboard:keyboard mail:mail scan:scan
dots:dots menu:menu-2 grid:grid-dots magnifier:zoom-scan straighten:angle
""".split()

# Plain-text symbols -> icon, used only where Inter has no glyph of its own.
TEXT_SYMBOLS = {
    0x2713: "check", 0x2714: "check", 0x2715: "x", 0x2716: "x", 0x2717: "x",
    0x2605: "star-filled", 0x2606: "star", 0x25B6: "player-play-filled", 0x25BA: "player-play-filled",
    0x23F5: "player-play-filled", 0x23F8: "player-pause-filled", 0x23F9: "player-stop-filled",
    0x25CF: "circle-filled", 0x25CB: "circle", 0x26A0: "alert-triangle", 0x2699: "settings", 0x2302: "home",
    0x266A: "music", 0x1F512: "lock", 0x1F4CD: "map-pin", 0x1F4C2: "folder-open", 0x1F5C0: "folder",
    0x270E: "pencil", 0x2922: "arrows-diagonal", 0x25B2: "caret-up-filled", 0x25BC: "caret-down-filled",
    0x25B8: "caret-right-filled", 0x25BE: "caret-down-filled", 0x25C0: "caret-left-filled",
    0x2B05: "arrow-left", 0x2B06: "arrow-up", 0x2B07: "arrow-down", 0x27A1: "arrow-right",
    0x25A6: "layout-grid", 0x1F50D: "search", 0x2665: "heart-filled", 0x27F2: "rotate", 0x27F3: "rotate-clockwise",
    0x1F5D1: "trash", 0x2139: "info-circle", 0x1F4F7: "camera", 0x1F3B5: "music",
}


def css_codes(css: Path) -> dict[str, int]:
    pat = re.compile(r'\.ti-([a-z0-9-]+):before\s*\{\s*content:\s*"\\([0-9a-fA-F]+)"')
    return {m.group(1): int(m.group(2), 16) for m in pat.finditer(css.read_text(encoding="utf-8"))}


def build(weight: str, icons_rs: bool) -> None:
    font = TTFont(SRC / f"Inter-{weight}.ttf")
    outline = TTFont(SRC / "tabler-icons.ttf")
    filled = TTFont(SRC / "tabler-icons-filled.ttf")
    # The filled set uses the same class names as the outline set: "star-filled" = "star" in the filled set.
    codes = css_codes(SRC / "tabler-icons.css")
    codes |= {f"{k}-filled": v for k, v in css_codes(SRC / "tabler-icons-filled.css").items()}

    upm = font["head"].unitsPerEm
    cap = font["OS/2"].sCapHeight
    box = round(cap * 1.32)  # tallest icon side, in font units
    advance = round(box * 1.12)
    cmap_inter = font.getBestCmap()
    order = font.getGlyphOrder()
    glyf, hmtx = font["glyf"], font["hmtx"]
    added: dict[str, str] = {}  # tabler name -> glyph name in the output font
    new_map: dict[int, str] = {}
    missing = []

    def add(tname: str) -> str | None:
        if tname in added:
            return added[tname]
        src = filled if tname.endswith("-filled") else outline
        cp = codes.get(tname)
        gname_src = src.getBestCmap().get(cp) if cp is not None else None
        if gname_src is None:
            missing.append(tname)
            return None
        rec = DecomposingRecordingPen(src.getGlyphSet())
        src.getGlyphSet()[gname_src].draw(rec)
        # Bounds of the source outline.
        tmp = TTGlyphPen(None)
        rec.replay(tmp)
        g0 = tmp.glyph()
        g0.recalcBounds(None)
        w, h = g0.xMax - g0.xMin, g0.yMax - g0.yMin
        s = box / max(w, h, 1) * min(1.0, max(w, h) / 860)  # small icons stay small, big ones fill the box
        dx = advance / 2 - (g0.xMin + w / 2) * s
        dy = cap / 2 - (g0.yMin + h / 2) * s
        pen = TTGlyphPen(None)
        rec.replay(TransformPen(pen, (s, 0, 0, s, dx, dy)))
        g = pen.glyph()
        g.recalcBounds(None)
        name = f"ti.{tname}"
        order.append(name)
        glyf.glyphs[name] = g
        glyf.glyphOrder = order
        hmtx.metrics[name] = (advance, g.xMin if g.numberOfContours else 0)
        new_map[cp] = name
        added[tname] = name
        return name

    consts = []
    for pair in ICONS:
        rust, tname = pair.split(":")
        if add(tname):
            consts.append((rust.upper(), tname, codes[tname]))
    for cp, tname in TEXT_SYMBOLS.items():
        if cp not in cmap_inter and (g := add(tname)):
            new_map[cp] = g
    if missing:
        sys.exit(f"Tabler has no icon named: {', '.join(sorted(set(missing)))}")

    font.setGlyphOrder(order)
    # cmap: BMP code points into every Unicode subtable; beyond the BMP only into format 12.
    tables = font["cmap"].tables
    if any(cp > 0xFFFF for cp in new_map) and not any(t.format == 12 for t in tables):
        t12 = CmapSubtable.newSubtable(12)
        t12.platformID, t12.platEncID, t12.language = 3, 10, 0
        t12.cmap = dict(font.getBestCmap())
        tables.append(t12)
    for t in tables:
        if t.isUnicode():
            for cp, g in new_map.items():
                if cp <= 0xFFFF or t.format in (12, 13):
                    t.cmap[cp] = g
    font["post"].formatType = 3.0  # no glyph names: smaller, and nothing reads them
    font["post"].extraNames, font["post"].mapping = [], {}

    # Name: "ChronoSort UI" (OFL: a modified version gets its own name).
    style = "Regular" if weight == "Regular" else "SemiBold"
    names = {1: "ChronoSort UI" if style == "Regular" else "ChronoSort UI SemiBold",
             2: "Regular", 3: f"ChronoSortUI-{style};1.0", 4: f"ChronoSort UI {style}",
             6: f"ChronoSortUI-{style}", 16: "ChronoSort UI", 17: style}
    nt = font["name"]
    nt.names = [n for n in nt.names if n.nameID not in names and n.nameID not in (21, 22, 25)]
    for nid, val in names.items():
        nt.setName(val, nid, 3, 1, 0x409)
    nt.setName("ChronoSort UI is Inter (SIL OFL 1.1, The Inter Project Authors) with Tabler Icons "
               "(MIT, Paweł Kuna) added for ChronoSort.", 0, 3, 1, 0x409)

    out = HERE / f"ChronoSortUI-{style}.ttf"
    font.save(out)
    print(f"{out.name}: {len(added)} icons, {len(new_map)} code points added, {out.stat().st_size // 1024} KB")

    if icons_rs:
        lines = ["//! Icons of the ChronoSort UI font (generated by fonts/build_font.py; do not edit).",
                 "//! Tabler Icons 3.50.0 (MIT). Use them in any text: `format!(\"{} Back\", icons::BACK)`.",
                 "#![allow(dead_code)]", ""]
        for rust, tname, cp in consts:
            lines.append(f"/// `{tname}`\npub const {rust}: &str = \"\\u{{{cp:x}}}\";")
        lines.append("\n/// Every icon, for the Help page and tests.")
        lines.append("pub const ALL: &[(&str, &str)] = &[")
        lines += [f"    (\"{tname}\", {rust})," for rust, tname, _ in consts]
        lines.append("];")
        (HERE.parent / "src" / "icons.rs").write_text("\n".join(lines) + "\n", encoding="utf-8")


if __name__ == "__main__":
    build("Regular", icons_rs=True)
    build("SemiBold", icons_rs=False)
