<script>
  import { onMount, tick } from "svelte";
  import Icon from "./Icon.svelte";
  import InfoPanel from "./InfoPanel.svelte";
  import AdjustPanel from "./AdjustPanel.svelte";
  import RetouchPanel from "./RetouchPanel.svelte";
  import EffectsPanel from "./EffectsPanel.svelte";
  import RetouchOverlay from "./RetouchOverlay.svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    fthumbUrl, revealItem, formatBytes, confirmBox, imageSize,
    editOpen, editApply, editStepBack, editClose, editSave, editUrl, editSetDpi, editRetouch,
    readMarks, setRating, setTagged, starText,
  } from "../lib/api.js";

  // items: browser entries (photos and videos of the folder); index: the one to show first.
  // onclose(lastId): back to the browser. onchanged(message): a file was saved / changed.
  let { items, index = 0, onclose, onchanged } = $props();

  // svelte-ignore state_referenced_locally
  let i = $state(index); // starts at the clicked photo, then the viewer moves on its own
  let cur = $derived(items[i]);
  let el = $state();
  let natural = $state({ w: 0, h: 0 });
  let zoom = $state(0); // 0 = fit, otherwise the scale of the shown image (preview or original)
  let fullSize = $state(null); // { w, h, original } of the real picture, from its header
  let hiRes = $state(false); // showing the original file instead of the preview (after zooming in)
  let pan = $state({ x: 0, y: 0 });
  let edge = $state(""); // fly-out under the pointer: left | right | top | bottom
  let pinned = $state(""); // a fly-out kept open while working in it
  let failed = $state(false);
  let hideCursor = $state(false);
  let message = $state("");
  let cursorTimer, msgTimer;

  // editing
  let edit = $state(null); // edit_open / edit_apply state { width, height, version, edits, lossless, format }
  let busy = $state(false);
  let crop = $state(null); // { ratio, sel: {x,y,w,h} in screen px }
  let resize = $state(null); // { w, h, keep }
  let saveDlg = $state(null); // { mode, format, quality }
  let adjusting = $state(false); // the colour and light panel is open
  let effecting = $state(false); // the frame and watermark panel is open
  let straight = $state(null); // { angle, frame: {x,y,w,h}, draw, line: {x1,y1,x2,y2} | null }
  let canvasDlg = $state(null); // { w, h, anchor, color }
  let dpiDlg = $state(null); // { dpi }
  let rt = $state(null); // retouch tools state, shared by RetouchPanel and RetouchOverlay
  let rtRect = $state(null); // the picture on screen while retouching
  // Something is changed and not saved: edit steps or colour / light adjustments.
  let dirty = $derived(!!edit && (edit.edits > 0 || edit.adjusted || edit.has_effects || edit.dpi != null));
  let imgEl = $state();
  let stripEl = $state();
  let full = $state(false); // the window is full screen
  // Windows ignores "minimise" for a full-screen window: leave full screen first and
  // go back to it when the user returns to ChronoSort.
  let refullOnFocus = false;
  async function minimize() {
    const w = getCurrentWindow();
    if (full) {
      await w.setFullscreen(false).catch(() => {});
      full = false;
      refullOnFocus = true;
      await new Promise((r) => setTimeout(r, 250)); // let Windows finish leaving full screen
    }
    await w.minimize().catch(() => {});
  }
  async function toggleFull() {
    const w = getCurrentWindow();
    await w.setFullscreen(!full).catch(() => {});
    full = await w.isFullscreen().catch(() => !full);
    el?.focus();
  }
  // Keep the current photo visible in the top strip.
  $effect(() => {
    const n = i, o = open("top");
    if (o && stripEl) tick().then(() => stripEl.querySelector(`[data-n="${n}"]`)?.scrollIntoView({ inline: "center", block: "nearest" }));
  });

  const open = (side) => edge === side || pinned === side;

  function say(m) {
    message = m;
    clearTimeout(msgTimer);
    msgTimer = setTimeout(() => (message = ""), 6000);
  }

  // The whole ChronoSort window goes full screen (not the web page's own full-screen
  // mode, which would swallow Esc): Esc always reaches the viewer and returns to the browser.
  onMount(() => {
    tick().then(() => el?.focus());
    const win = getCurrentWindow();
    let wasFull = false;
    win.isFullscreen().then((f) => {
      wasFull = f;
      if (!f) win.setFullscreen(true).then(() => (full = true)).catch(() => {});
      else full = true;
    }).catch(() => {});
    // Back from the taskbar after "minimise": full screen again.
    const unFocus = win.onFocusChanged(({ payload: focused }) => {
      if (focused && refullOnFocus) {
        refullOnFocus = false;
        win.setFullscreen(true).then(() => (full = true)).catch(() => {});
        el?.focus();
      }
    });
    return () => {
      unFocus.then((f) => f()).catch(() => {});
      if (!wasFull) getCurrentWindow().setFullscreen(false).catch(() => {});
      editClose().catch(() => {});
    };
  });

  async function leaveEdits() {
    if (!dirty) return true;
    const ok = await confirmBox("The changes to this picture are not saved.\n\nLeave them?", "Unsaved changes", "Leave them", "Keep editing");
    if (ok) await dropEdits();
    return ok;
  }
  async function dropEdits() {
    edit = null;
    adjusting = effecting = false;
    straight = null;
    canvasDlg = null;
    dpiDlg = null;
    rt = null;
    crop = null;
    resize = null;
    saveDlg = null;
    pinned = "";
    await editClose().catch(() => {});
  }

  async function show(n) {
    if (!items.length || busy) return;
    if (!(await leaveEdits())) return;
    await dropEdits();
    i = (n + items.length) % items.length;
    zoom = 0;
    hiRes = false;
    pan = { x: 0, y: 0 };
    failed = false;
  }
  // Real size of the current picture (header read; no decoding).
  $effect(() => {
    const id = cur?.id;
    fullSize = null;
    if (id == null) return;
    imageSize(id).then((r) => { if (cur?.id === id) fullSize = r; }).catch(() => {});
  });
  // Stars (in the photo) and the tag mark of the current file.
  let mark = $state(null); // { rating, tagged }
  $effect(() => {
    const id = cur?.id;
    mark = null;
    if (id == null) return;
    readMarks([id]).then(([m]) => { if (cur?.id === id) mark = m ?? null; }).catch(() => {});
  });
  async function rateCur(stars) {
    if (mark?.rating == null) return say("Stars can be given to photos only.");
    if (dirty) return say("Save or leave the changes to this picture first, then give it stars.");
    if (busy) return;
    busy = true;
    try {
      const r = await setRating([cur.id], stars);
      if (r.failed.length) say(`Not possible: ${r.failed[0][1]}`);
      else {
        mark = { ...mark, rating: stars };
        say(stars ? `${starText(stars)} saved in the photo (Ctrl+Z in the browser undoes)` : "Stars removed");
        if (r.done) onchanged?.("rated");
      }
    } catch (e) {
      say(String(e));
    } finally {
      busy = false;
    }
  }
  async function tagCur() {
    if (!mark) return;
    const on = !mark.tagged;
    try {
      await setTagged([cur.id], on);
      mark = { ...mark, tagged: on };
      say(on ? "Tagged" : "Tag removed");
      onchanged?.("tagged");
    } catch (e) {
      say(String(e));
    }
  }
  const next = () => show(i + 1);
  const prev = () => show(i - 1);
  let closing = false;
  async function close(fromFullscreenExit = false) {
    if (closing) return;
    if (dirty && !fromFullscreenExit && !(await leaveEdits())) return;
    closing = true;
    onclose(cur?.id ?? null);
  }

  function fitScale() {
    if (!el || !natural.w) return 1;
    return Math.min(1, el.clientWidth / natural.w, el.clientHeight / natural.h);
  }
  // Crop, straighten and retouch place marks on the picture itself: show it without frame / watermark.
  let rawView = $derived(!!(edit?.has_effects && (crop || straight || rt)));
  // The real picture's size, upright like the shown image (an EXIF turn swaps the stored size).
  let real = $derived.by(() => {
    if (edit) return rawView ? { w: edit.width, h: edit.height } : { w: edit.out_width, h: edit.out_height };
    if (!fullSize || !natural.w) return null;
    const { w, h } = fullSize;
    return natural.w > natural.h !== w > h && natural.w !== natural.h ? { w: h, h: w } : { w, h };
  });
  // `zoom` scales the shown image in CSS pixels; 100 % means one SCREEN pixel per real
  // pixel. zoom 1.0 on a 1280 px preview of a 4000 px photo is only 32 % of the real
  // picture, and Windows display scaling (devicePixelRatio) enlarges every CSS pixel.
  let dpr = $state(window.devicePixelRatio || 1);
  let scaleF = $derived((real && natural.w ? real.w / natural.w : 1) / dpr);
  function setZoom(z) {
    zoom = Math.min(8 * scaleF, Math.max(0.02, z));
    // Past the preview's own pixels: load the original so the picture stays sharp.
    if (!edit && !hiRes && fullSize?.original && real && real.w > natural.w * 1.01 && zoom * dpr > 1.001) hiRes = true;
  }
  $effect(() => {
    const upd = () => (dpr = window.devicePixelRatio || 1); // another monitor / scaling change
    window.addEventListener("resize", upd);
    return () => window.removeEventListener("resize", upd);
  });
  const zoomBy = (f) => setZoom((zoom || fitScale()) * f);
  const actualSize = () => setZoom(scaleF);
  const fit = () => { zoom = 0; pan = { x: 0, y: 0 }; };
  // Percent of the REAL picture, not of the preview.
  let pct = $derived(natural.w ? Math.round((100 * (zoom || fitScale())) / scaleF) : 0);
  function imgLoaded(e) {
    const nw = e.currentTarget.naturalWidth, nh = e.currentTarget.naturalHeight;
    // Same picture at another resolution (preview -> original): keep the same view.
    if (zoom && natural.w && nw !== natural.w) zoom = (zoom * natural.w) / nw;
    natural = { w: nw, h: nh };
  }

  // ---------- edits ----------
  async function ensureEdit() {
    if (edit) return true;
    busy = true;
    try {
      edit = await editOpen(cur.id);
      return true;
    } catch (e) {
      say(String(e));
      return false;
    } finally {
      busy = false;
    }
  }
  async function doEdit(op) {
    if (busy || !(await ensureEdit())) return;
    busy = true;
    try {
      edit = await editApply(op);
      fit();
    } catch (e) {
      say(String(e));
    } finally {
      busy = false;
    }
  }
  async function stepBack() {
    if (!edit?.edits || busy) return;
    busy = true;
    try { edit = await editStepBack(); } catch (e) { say(String(e)); } finally { busy = false; }
  }

  // Crop: drag a rectangle over the picture (screen pixels), mapped to picture pixels on Apply.
  const RATIOS = [["free", "Free"], ["orig", "Original"], ["1:1", "1 : 1"], ["4:3", "4 : 3"], ["3:2", "3 : 2"], ["16:9", "16 : 9"], ["3:4", "3 : 4 (portrait)"], ["2:3", "2 : 3 (portrait)"]];
  async function startCrop() {
    if (!(await ensureEdit())) return;
    fit();
    pinned = "left";
    await tick();
    await new Promise((r) => setTimeout(r, 120)); // let the edit preview load
    const r = imgEl.getBoundingClientRect();
    crop = { ratio: "free", sel: { x: r.left + r.width * 0.1, y: r.top + r.height * 0.1, w: r.width * 0.8, h: r.height * 0.8 } };
  }
  function ratioOf(key) {
    if (key === "free") return 0;
    if (key === "orig") return edit.width / edit.height;
    const [a, b] = key.split(":").map(Number);
    return a / b;
  }
  function applyRatio() {
    const q = ratioOf(crop.ratio);
    if (!q) return;
    const r = imgEl.getBoundingClientRect();
    let { x, y, w, h } = crop.sel;
    if (w / h > q) w = h * q; else h = w / q;
    if (w > r.width) { w = r.width; h = w / q; }
    if (h > r.height) { h = r.height; w = h * q; }
    x = Math.min(Math.max(x, r.left), r.right - w);
    y = Math.min(Math.max(y, r.top), r.bottom - h);
    crop.sel = { x, y, w, h };
  }
  function cropDrag(ev, mode) {
    ev.preventDefault();
    ev.stopPropagation();
    const r = imgEl.getBoundingClientRect();
    const s0 = { ...crop.sel }, x0 = ev.clientX, y0 = ev.clientY;
    const q = ratioOf(crop.ratio);
    const clamp = (v, lo, hi) => Math.min(Math.max(v, lo), hi);
    const mv = (m) => {
      const dx = m.clientX - x0, dy = m.clientY - y0;
      let { x, y, w, h } = s0;
      if (mode === "move") {
        x = clamp(x + dx, r.left, r.right - w);
        y = clamp(y + dy, r.top, r.bottom - h);
      } else if (mode === "new") {
        x = clamp(Math.min(x0, m.clientX), r.left, r.right);
        y = clamp(Math.min(y0, m.clientY), r.top, r.bottom);
        w = Math.min(Math.abs(dx), r.right - x);
        h = Math.min(Math.abs(dy), r.bottom - y);
        if (q) { h = Math.min(w / q, r.bottom - y); w = h * q; }
      } else {
        if (mode.includes("e")) w = clamp(s0.w + dx, 16, r.right - x);
        if (mode.includes("s")) h = clamp(s0.h + dy, 16, r.bottom - y);
        if (mode.includes("w")) { const nx = clamp(s0.x + dx, r.left, s0.x + s0.w - 16); w = s0.x + s0.w - nx; x = nx; }
        if (mode.includes("n")) { const ny = clamp(s0.y + dy, r.top, s0.y + s0.h - 16); h = s0.y + s0.h - ny; y = ny; }
        if (q) { if (Math.abs(dy) > Math.abs(dx)) w = h * q; else h = w / q; }
      }
      crop.sel = { x, y, w, h };
    };
    const up = () => { window.removeEventListener("pointermove", mv); window.removeEventListener("pointerup", up); };
    window.addEventListener("pointermove", mv);
    window.addEventListener("pointerup", up);
  }
  let cropPx = $derived.by(() => {
    if (!crop || !imgEl || !edit) return null;
    const r = imgEl.getBoundingClientRect();
    const sx = edit.width / r.width, sy = edit.height / r.height;
    const x = Math.max(0, Math.round((crop.sel.x - r.left) * sx)), y = Math.max(0, Math.round((crop.sel.y - r.top) * sy));
    return { x, y, w: Math.min(edit.width - x, Math.round(crop.sel.w * sx)), h: Math.min(edit.height - y, Math.round(crop.sel.h * sy)) };
  });
  async function applyCrop() {
    const p = cropPx;
    crop = null;
    pinned = "";
    if (p && p.w > 0 && p.h > 0) await doEdit({ op: "crop", ...p });
  }

  async function startAdjust() {
    if (!(await ensureEdit())) return;
    crop = null;
    resize = null;
    saveDlg = null;
    effecting = false;
    adjusting = true;
    pinned = "left";
  }
  async function startEffects() {
    if (!(await ensureEdit())) return;
    crop = resize = saveDlg = straight = canvasDlg = dpiDlg = rt = null;
    adjusting = false;
    effecting = true;
    pinned = "left";
  }

  // ---- straighten: live preview by CSS, the real turn on Apply ----
  // The largest rectangle of the picture's shape inside it when turned (same as the backend).
  function straightScale(w, h, deg) {
    const r = (Math.abs(deg) * Math.PI) / 180, s = Math.sin(r), c = Math.cos(r);
    return Math.min(w / (w * c + h * s), h / (w * s + h * c));
  }
  async function startStraighten() {
    if (!(await ensureEdit())) return;
    crop = resize = saveDlg = canvasDlg = dpiDlg = null;
    adjusting = effecting = false;
    fit();
    await new Promise((r) => setTimeout(r, 60));
    const b = imgEl?.getBoundingClientRect();
    if (!b) return;
    straight = { angle: 0, frame: { x: b.left, y: b.top, w: b.width, h: b.height }, draw: false, line: null };
    pinned = "left";
  }
  function setAngle(v) {
    straight.angle = Math.max(-45, Math.min(45, Math.round((+v || 0) * 10) / 10));
  }
  // Draw a line along something that should be level (or upright): the angle follows from it.
  function lineDown(ev) {
    ev.preventDefault();
    const p = { x: ev.clientX, y: ev.clientY };
    straight.line = { x1: p.x, y1: p.y, x2: p.x, y2: p.y };
    const move = (e) => { straight.line.x2 = e.clientX; straight.line.y2 = e.clientY; };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      const l = straight.line;
      const dx = l.x2 - l.x1, dy = l.y2 - l.y1;
      if (Math.hypot(dx, dy) > 20) {
        let a = (Math.atan2(dy, dx) * 180) / Math.PI; // screen angle of the line
        if (a > 90) a -= 180;
        if (a <= -90) a += 180;
        // Nearer horizontal: make it level; nearer vertical: make it upright.
        const need = Math.abs(a) <= 45 ? -a : (a > 0 ? 90 - a : -90 - a);
        setAngle(straight.angle + need);
      }
      straight.line = null;
      straight.draw = false;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }
  async function applyStraighten() {
    const a = straight.angle;
    straight = null;
    pinned = "";
    if (a) await doEdit({ op: "straighten", angle100: Math.round(a * 100) });
  }
  let straightStyle = $derived(straight && natural.w
    ? `transform: translate(-50%, -50%) rotate(${straight.angle}deg) scale(${1 / straightScale(natural.w, natural.h, straight.angle)})`
    : "");

  // ---- retouch (red eye, heal, clone, draw) ----
  async function startRetouch() {
    if (!(await ensureEdit())) return;
    crop = resize = saveDlg = straight = canvasDlg = dpiDlg = null;
    adjusting = effecting = false;
    fit();
    await new Promise((r) => setTimeout(r, 80));
    const b = imgEl?.getBoundingClientRect();
    if (!b) return;
    rtRect = { x: b.left, y: b.top, w: b.width, h: b.height };
    rt = { tool: "heal", size: 18, hardness: 50, pupil: 50, darken: 50, src: null, offset: null, pickSource: false,
      shape: "pen", color: "#ffffff", width: 6, font: 48, text: "", layer: null, drew: false, busy: false, hint: "" };
    pinned = "left";
  }
  async function applyRetouch(op) {
    rt.busy = true;
    rt.hint = "";
    try {
      edit = await editRetouch(op);
      return true;
    } catch (e) {
      say(String(e));
      return false;
    } finally {
      if (rt) rt.busy = false;
    }
  }

  // ---- canvas size ----
  async function startCanvas() {
    if (!(await ensureEdit())) return;
    crop = resize = saveDlg = straight = dpiDlg = null;
    adjusting = effecting = false;
    canvasDlg = { w: edit.width, h: edit.height, anchor: 4, color: "#ffffff" };
    pinned = "left";
  }
  function border(pct) {
    const b = Math.round((Math.min(edit.width, edit.height) * pct) / 100);
    canvasDlg.w = edit.width + 2 * b;
    canvasDlg.h = edit.height + 2 * b;
    canvasDlg.anchor = 4;
  }
  async function applyCanvas() {
    const c = canvasDlg;
    const hex = c.color.replace("#", "");
    const color = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16) || 0);
    canvasDlg = null;
    pinned = "";
    await doEdit({ op: "canvas", w: Math.max(1, Math.round(+c.w)), h: Math.max(1, Math.round(+c.h)), anchor: c.anchor, color });
  }

  // ---- print size (DPI) ----
  async function startDpi() {
    if (!(await ensureEdit())) return;
    crop = resize = saveDlg = straight = canvasDlg = null;
    adjusting = effecting = false;
    dpiDlg = { dpi: edit.dpi ?? Math.round(edit.dpi_original ?? 300) };
    pinned = "left";
  }
  async function applyDpi(clear = false) {
    const v = clear ? null : Math.round(+dpiDlg.dpi);
    try {
      edit = await editSetDpi(v);
      dpiDlg = null;
      pinned = "";
      if (v) say(`The picture will be saved for printing at ${v} dots per inch.`);
    } catch (e) {
      say(String(e));
    }
  }
  const printSize = (px, dpi) => `${((px / dpi) * 2.54).toFixed(1)} cm (${(px / dpi).toFixed(1)} in)`;

  async function startResize() {
    if (!(await ensureEdit())) return;
    resize = { w: edit.width, h: edit.height, keep: true };
    pinned = "left";
  }
  function resizeW(v) {
    resize.w = Math.max(1, Math.round(+v || 1));
    if (resize.keep) resize.h = Math.max(1, Math.round((resize.w * edit.height) / edit.width));
  }
  function resizeH(v) {
    resize.h = Math.max(1, Math.round(+v || 1));
    if (resize.keep) resize.w = Math.max(1, Math.round((resize.h * edit.width) / edit.height));
  }
  async function applyResize() {
    const r = resize;
    resize = null;
    pinned = "";
    await doEdit({ op: "resize", w: r.w, h: r.h });
  }

  async function startSave() {
    if (!(await ensureEdit())) return;
    const same = { Jpeg: "JPEG", Png: "PNG", Tiff: "TIFF", WebP: "WebP" }[edit.format] ?? "JPEG";
    saveDlg = { mode: "copy", format: same, quality: 92 };
    pinned = "left";
  }
  async function doSave() {
    const d = saveDlg;
    busy = true;
    try {
      const r = await editSave(d.mode, d.format, d.quality);
      const name = r.path.split(/[\\/]/).pop();
      const how = r.lossless ? " without any quality loss (only the orientation tag changed)" : "";
      const msg = d.mode === "replace"
        ? `Saved over “${name}”${how}. The original is kept: Ctrl+Z in the browser brings it back.`
        : `Saved as “${name}”${how}.`;
      await dropEdits();
      say(msg + (r.warnings.length ? " Note: " + r.warnings.join("; ") : ""));
      onchanged?.(msg);
    } catch (e) {
      say(String(e));
    } finally {
      busy = false;
    }
  }

  // ---------- keys, wheel, pointer ----------
  function onKey(e) {
    if (e.target.closest?.("input, textarea, select")) {
      if (e.key === "Escape") { el.focus(); e.preventDefault(); e.stopPropagation(); }
      return;
    }
    const k = e.key, ctrl = e.ctrlKey || e.metaKey;
    if (k === "Escape") {
      if (crop || resize || saveDlg || adjusting || effecting || straight || canvasDlg || dpiDlg || rt) {
        crop = resize = saveDlg = straight = canvasDlg = dpiDlg = rt = null;
        adjusting = effecting = false;
        pinned = "";
      }
      else if (pinned) pinned = "";
      else close();
    } else if (k === "Enter" && crop) applyCrop();
    else if (k === "Enter" && straight) applyStraighten();
    else if (k === "Enter") close();
    else if (ctrl && (k === "r" || k === "R")) startResize();
    else if (ctrl && (k === "s" || k === "S")) startSave();
    else if (ctrl && (k === "z" || k === "Z")) stepBack();
    else if (ctrl && /^[0-5]$/.test(k)) rateCur(+k);
    else if (ctrl) return;
    else if (k === "q" || k === "Q") tagCur();
    else if (k === "ArrowRight" || k === "PageDown" || k === " ") next();
    else if (k === "ArrowLeft" || k === "PageUp" || k === "Backspace") prev();
    else if (k === "Home") show(0);
    else if (k === "End") show(items.length - 1);
    else if (k === "0" || k === "b" || k === "B") fit();
    else if (k === "1" || k === "a" || k === "A") actualSize();
    else if (k === "+" || k === "=") zoomBy(1.25);
    else if (k === "-") zoomBy(0.8);
    else if (k === "i" || k === "I") pinned = pinned === "right" ? "" : "right";
    else if (k === "l" || k === "L") doEdit({ op: "rotate_left" });
    else if (k === "r" || k === "R") doEdit({ op: "rotate_right" });
    else if (k === "h" || k === "H") doEdit({ op: "flip_horizontal" });
    else if (k === "v" || k === "V") doEdit({ op: "flip_vertical" });
    else if (k === "x" || k === "X") startCrop();
    else if (k === "c" || k === "C") startAdjust();
    else if (k === "t" || k === "T") startStraighten();
    else if (k === "e" || k === "E") startRetouch();
    else if (k === "f" || k === "F") startEffects();
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  function onWheel(e) {
    if (e.target.closest(".fly")) return;
    e.preventDefault();
    if (crop) return;
    if (e.ctrlKey) zoomBy(e.deltaY < 0 ? 1.15 : 0.87);
    else if (e.deltaY > 0) next();
    else if (e.deltaY < 0) prev();
  }

  // Fly-outs: open when the pointer reaches an edge, close when it leaves the panel.
  function onMove(e) {
    hideCursor = false;
    clearTimeout(cursorTimer);
    cursorTimer = setTimeout(() => (hideCursor = !edge && !pinned && !crop), 2000);
    if (e.target.closest(".fly") || e.buttons) return;
    const w = window.innerWidth, h = window.innerHeight;
    if (e.clientX <= 6) edge = "left";
    else if (e.clientX >= w - 7) edge = "right";
    else if (e.clientY <= 6) edge = "top";
    else if (e.clientY >= h - 7) edge = "bottom";
    else edge = "";
  }

  function onDown(e) {
    if (e.button !== 0 || !zoom || crop || e.target.closest(".fly")) return;
    const sx = e.clientX - pan.x, sy = e.clientY - pan.y;
    const mv = (m) => (pan = { x: m.clientX - sx, y: m.clientY - sy });
    const up = () => { window.removeEventListener("pointermove", mv); window.removeEventListener("pointerup", up); };
    window.addEventListener("pointermove", mv);
    window.addEventListener("pointerup", up);
  }

  // Navigator: where the visible window sits in the zoomed picture (0..1).
  let navBox = $derived.by(() => {
    if (!zoom || !natural.w || !el) return null;
    const W = natural.w * zoom, H = natural.h * zoom, vw = el.clientWidth, vh = el.clientHeight;
    const left = (W - vw) / 2 - pan.x, top = (H - vh) / 2 - pan.y;
    return { x: Math.max(0, left / W), y: Math.max(0, top / H), w: Math.min(1, vw / W), h: Math.min(1, vh / H) };
  });
  function navTo(ev) {
    ev.preventDefault();
    ev.stopPropagation();
    const r = ev.currentTarget.getBoundingClientRect();
    const go = (e) => {
      const fx = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
      const fy = Math.min(1, Math.max(0, (e.clientY - r.top) / r.height));
      pan = { x: -(fx - 0.5) * natural.w * zoom, y: -(fy - 0.5) * natural.h * zoom };
    };
    go(ev);
    // Keep following the pointer while the button is held: drag the view around.
    const up = () => { window.removeEventListener("pointermove", go); window.removeEventListener("pointerup", up); };
    window.addEventListener("pointermove", go);
    window.addEventListener("pointerup", up);
  }

  let src = $derived(edit ? editUrl(`${cur.id}-${edit.version}${rawView ? "-raw" : ""}`) : cur ? fthumbUrl(cur.id, hiRes ? "o" : "p") : "");
  let pre = $derived(items.length > 1 ? [items[(i + 1) % items.length], items[(i - 1 + items.length) % items.length]] : []);
  const ZOOMS = [10, 25, 50, 75, 100, 150, 200, 300, 400];
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="viewer" class:nocursor={hideCursor} bind:this={el} tabindex="0" role="dialog" aria-modal="true" aria-label="Photo viewer"
  onkeydown={onKey} onwheel={onWheel} onpointermove={onMove} onpointerdown={onDown}
  ondblclick={(e) => !e.target.closest(".fly, .cropbox") && !crop && close()}>
  {#if cur}
    {#if !failed}
      <img bind:this={imgEl} class="photo" class:fit={!zoom} {src} alt={cur.name} draggable="false"
        style={zoom ? `transform: translate(calc(-50% + ${pan.x}px), calc(-50% + ${pan.y}px)) scale(${zoom}); width:${natural.w}px; height:${natural.h}px` : straightStyle}
        onload={imgLoaded}
        onerror={() => (hiRes ? (hiRes = false) : (failed = true))} />
    {:else}
      <div class="nopic"><Icon name={cur.kind} size={96} /><p>No preview for this file yet.</p></div>
    {/if}
    <div class="info">
      {cur.name} &nbsp;( {real ? `${real.w} × ${real.h}` : natural.w ? `${natural.w} × ${natural.h} preview` : "…"}, {formatBytes(cur.size)} ) &nbsp;[ {i + 1} / {items.length} ] &nbsp;{pct ? pct + "%" : ""}
      {#if mark?.rating}<span class="vstars" title="{mark.rating} of 5 stars (Ctrl+0–5)">{"★".repeat(mark.rating)}</span>{/if}
      {#if mark?.tagged}<span class="vtag">● tagged</span>{/if}
      {#if dirty}<span class="unsaved">● edited, not saved yet</span>{/if}
    </div>
  {/if}
  {#each pre as p (p.id)}<link rel="prefetch" href={fthumbUrl(p.id, "p")} />{/each}
  <!-- The usual window buttons (the title bar is hidden in full screen). -->
  <div class="winbtns" class:hidden={hideCursor}>
    <button title="Minimise" aria-label="Minimise" onclick={minimize}><svg viewBox="0 0 12 12" width="12" height="12"><path d="M1 6.5h10" stroke="currentColor" stroke-width="1.2" /></svg></button>
    <button title={full ? "Restore: show the viewer in a window" : "Full screen"} aria-label={full ? "Restore" : "Full screen"} onclick={toggleFull}>
      {#if full}<svg viewBox="0 0 12 12" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.1"><rect x="1.5" y="3.5" width="7" height="7" /><path d="M3.5 3.5v-2h7v7h-2" /></svg>
      {:else}<svg viewBox="0 0 12 12" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1.1"><rect x="1.5" y="1.5" width="9" height="9" /></svg>{/if}
    </button>
    <button class="x" title="Close the viewer (Esc)" aria-label="Close the viewer" onclick={() => close()}><svg viewBox="0 0 12 12" width="12" height="12"><path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.2" /></svg></button>
  </div>
  {#if message}<div class="msg" role="status">{message}</div>{/if}
  {#if busy}<div class="busy"><span class="spin"></span> Working…</div>{/if}

  <!-- crop rectangle -->
  {#if crop}
    <div class="shade" role="presentation" onpointerdown={(e) => cropDrag(e, "new")}></div>
    <div class="cropbox" role="presentation" style="left:{crop.sel.x}px; top:{crop.sel.y}px; width:{crop.sel.w}px; height:{crop.sel.h}px"
      onpointerdown={(e) => cropDrag(e, "move")}>
      {#each ["nw", "ne", "sw", "se"] as c}<span class="hd {c}" role="presentation" onpointerdown={(e) => cropDrag(e, c)}></span>{/each}
      {#if cropPx}<span class="dims">{cropPx.w} × {cropPx.h}</span>{/if}
    </div>
  {/if}

  {#if rt && rtRect && edit}
    <RetouchOverlay {rt} rect={rtRect} imgW={edit.width} imgH={edit.height} onapply={applyRetouch} />
  {/if}

  <!-- straighten: the frame that will be kept, with a grid; drawing layer for the level line -->
  {#if straight}
    <div class="sframe" style="left:{straight.frame.x}px; top:{straight.frame.y}px; width:{straight.frame.w}px; height:{straight.frame.h}px"></div>
    {#if straight.draw}
      <div class="shade" role="presentation" onpointerdown={lineDown}></div>
    {/if}
    {#if straight.line}
      <svg class="sline"><line x1={straight.line.x1} y1={straight.line.y1} x2={straight.line.x2} y2={straight.line.y2} /></svg>
    {/if}
  {/if}

  <!-- TOP: thumbnail strip -->
  <div class="fly top" class:open={open("top")} onpointerleave={() => (edge = "")} role="group" aria-label="Photos in this folder">
    <div class="tophead">
      <strong>{cur?.name}</strong>
      <span class="muted">{i + 1} of {items.length} photos and videos in this folder · click one to show it · wheel scrolls the strip</span>
    </div>
    <div class="strip" bind:this={stripEl} onwheel={(e) => { e.stopPropagation(); e.preventDefault(); stripEl.scrollLeft += e.deltaY || e.deltaX; }}>
      {#each items as it, n (it.id)}
        <button class="st" class:on={n === i} data-n={n} onclick={() => show(n)} title={it.name}>
          <span class="sp"><img src={fthumbUrl(it.id, "s")} alt="" loading="lazy" onerror={(x) => (x.currentTarget.style.visibility = "hidden")} /></span>
          <span class="sn">{it.name}</span>
        </button>
      {/each}
    </div>
  </div>

  <!-- LEFT: picture actions + edit forms -->
  <div class="fly left" class:wide={adjusting || effecting || rt} class:open={open("left")} onpointerleave={() => (edge = "")} role="group" aria-label="Edit">
    {#if rt}
      <RetouchPanel {rt} onapply={applyRetouch} onclose={() => { rt = null; pinned = ""; }} />
    {:else if straight}
      <h3>Straighten</h3>
      <p class="hint">Turn the picture until it is level; the corners are cropped so nothing is left blank.</p>
      <label class="fld">Angle {straight.angle.toFixed(1)}°
        <input type="range" min="-45" max="45" step="0.1" value={straight.angle} oninput={(e) => setAngle(e.currentTarget.value)}
          ondblclick={() => setAngle(0)} />
      </label>
      <div class="pcts">
        {#each [-1, -0.1, 0.1, 1] as d}<button onclick={() => setAngle(straight.angle + d)}>{d > 0 ? "+" : "−"}{Math.abs(d)}°</button>{/each}
      </div>
      <button class="wide-btn" class:on={straight.draw} onclick={() => (straight.draw = !straight.draw)}>
        {straight.draw ? "Now drag along the horizon…" : "Draw a level line"}</button>
      <p class="hint">Drag along something that should be level (a horizon, a table edge) or upright (a wall, a door).</p>
      <div class="row2"><button onclick={() => { straight = null; pinned = ""; }}>Cancel</button><button class="primary" onclick={applyStraighten}>Straighten <kbd>Enter</kbd></button></div>
    {:else if canvasDlg}
      <h3>Canvas size</h3>
      <p class="hint">Now {edit.width} × {edit.height} pixels. A bigger canvas adds a border; a smaller one trims the picture.</p>
      <div class="pcts">{#each [2, 5, 10] as p}<button onclick={() => border(p)}>Border {p}%</button>{/each}</div>
      <div class="wh">
        <label class="fld">Width <input type="number" min="1" bind:value={canvasDlg.w} /></label>
        <label class="fld">Height <input type="number" min="1" bind:value={canvasDlg.h} /></label>
      </div>
      <span class="hint">Picture position</span>
      <div class="anchors">
        {#each Array(9) as _, n}<button class:on={canvasDlg.anchor === n} aria-label="Position {n + 1}" onclick={() => (canvasDlg.anchor = n)}></button>{/each}
      </div>
      <label class="fld">Border colour
        <span class="swatches">
          {#each ["#ffffff", "#000000", "#808080"] as c}<button class="sw" class:on={canvasDlg.color === c} style="background:{c}" aria-label={c} onclick={() => (canvasDlg.color = c)}></button>{/each}
          <input type="color" bind:value={canvasDlg.color} />
        </span>
      </label>
      <div class="row2"><button onclick={() => { canvasDlg = null; pinned = ""; }}>Cancel</button><button class="primary" onclick={applyCanvas}>Apply</button></div>
    {:else if dpiDlg}
      <h3>Print size</h3>
      <p class="hint">{edit.dpi_original ? `The file says ${Math.round(edit.dpi_original)} dots per inch.` : "The file does not say how big to print."} This only changes how big the picture prints; the pixels stay the same.</p>
      <label class="fld">Dots per inch (DPI) <input type="number" min="1" max="10000" bind:value={dpiDlg.dpi} /></label>
      <div class="pcts">{#each [72, 150, 300, 600] as v}<button class:on={+dpiDlg.dpi === v} onclick={() => (dpiDlg.dpi = v)}>{v}</button>{/each}</div>
      {#if +dpiDlg.dpi > 0}<p class="hint">Prints at {printSize(edit.width, +dpiDlg.dpi)} × {printSize(edit.height, +dpiDlg.dpi)}</p>{/if}
      <div class="row2">
        {#if edit.dpi != null}<button onclick={() => applyDpi(true)}>Keep the file's</button>{:else}<button onclick={() => { dpiDlg = null; pinned = ""; }}>Cancel</button>{/if}
        <button class="primary" onclick={() => applyDpi()}>Apply</button>
      </div>
    {:else if effecting}
      <EffectsPanel start={edit} onstate={(st) => (edit = st)} onerror={say}
        onclose={() => { effecting = false; pinned = ""; }} />
    {:else if adjusting}
      <AdjustPanel start={edit?.adjust} onstate={(st) => (edit = st)} onerror={say}
        onclose={() => { adjusting = effecting = false; pinned = ""; }} />
    {:else if crop}
      <h3>Crop</h3>
      <label class="fld">Shape
        <select bind:value={crop.ratio} onchange={applyRatio}>{#each RATIOS as [k, t]}<option value={k}>{t}</option>{/each}</select>
      </label>
      <p class="hint">Drag the box or its corners; drag outside it to draw a new one.{#if cropPx}<br />{cropPx.w} × {cropPx.h} pixels{/if}</p>
      <div class="row2"><button onclick={() => { crop = null; pinned = ""; }}>Cancel</button><button class="primary" onclick={applyCrop}>Crop <kbd>Enter</kbd></button></div>
    {:else if resize}
      <h3>Resize</h3>
      <p class="hint">Now {edit.width} × {edit.height} pixels</p>
      <div class="wh">
        <label class="fld">Width <input type="number" min="1" value={resize.w} oninput={(e) => resizeW(e.currentTarget.value)} /></label>
        <label class="fld">Height <input type="number" min="1" value={resize.h} oninput={(e) => resizeH(e.currentTarget.value)} /></label>
      </div>
      <label class="chk"><input type="checkbox" bind:checked={resize.keep} /> Keep the shape (aspect ratio)</label>
      <div class="pcts">{#each [25, 50, 75] as p}<button onclick={() => resizeW((edit.width * p) / 100)}>{p}%</button>{/each}</div>
      <div class="row2"><button onclick={() => { resize = null; pinned = ""; }}>Cancel</button><button class="primary" onclick={applyResize}>Resize</button></div>
    {:else if saveDlg}
      <h3>Save the edited picture</h3>
      <label class="radio"><input type="radio" bind:group={saveDlg.mode} value="copy" /> <span><strong>As a new copy</strong><br /><small>next to the original (recommended)</small></span></label>
      <label class="radio"><input type="radio" bind:group={saveDlg.mode} value="replace" /> <span><strong>Replace the original</strong><br /><small>the original is kept in a hidden backup folder, so Undo can bring it back</small></span></label>
      <label class="fld">Format
        <select bind:value={saveDlg.format}>{#each ["JPEG", "PNG", "TIFF", "WebP"] as f}<option>{f}</option>{/each}</select>
      </label>
      {#if (saveDlg.format === "JPEG" || saveDlg.format === "WebP") && !(edit?.lossless && saveDlg.format === "JPEG")}
        <label class="fld">Quality {saveDlg.quality}<input type="range" min="50" max="100" bind:value={saveDlg.quality} /></label>
      {/if}
      {#if edit?.lossless && saveDlg.format === "JPEG"}<p class="hint good">Only turned, flipped or given a print size: saved <strong>without any quality loss</strong>.</p>{/if}
      {#if edit?.dpi}<p class="hint">Print resolution: {edit.dpi} dots per inch.</p>{/if}
      <div class="row2"><button onclick={() => { saveDlg = null; pinned = ""; }}>Cancel</button><button class="primary" disabled={busy} onclick={doSave}>Save</button></div>
    {:else}
      <h3>Picture</h3>
      <button onclick={() => doEdit({ op: "rotate_left" })}><Icon name="rot-left" size={16} /> Rotate left <kbd>L</kbd></button>
      <button onclick={() => doEdit({ op: "rotate_right" })}><Icon name="rot-right" size={16} /> Rotate right <kbd>R</kbd></button>
      <button onclick={() => doEdit({ op: "flip_horizontal" })}><Icon name="flip-h" size={16} /> Flip left–right <kbd>H</kbd></button>
      <button onclick={() => doEdit({ op: "flip_vertical" })}><Icon name="flip-v" size={16} /> Flip upside down <kbd>V</kbd></button>
      <button onclick={startCrop}><Icon name="crop" size={16} /> Crop… <kbd>X</kbd></button>
      <button onclick={startResize}><Icon name="resize" size={16} /> Resize… <kbd>Ctrl+R</kbd></button>
      <button onclick={startStraighten}><Icon name="rot-right" size={16} /> Straighten… <kbd>T</kbd></button>
      <button onclick={startCanvas}><Icon name="crop" size={16} /> Canvas size / border…</button>
      <button onclick={startDpi}><Icon name="details" size={16} /> Print size (DPI)…</button>
      <button onclick={startAdjust}><Icon name="details" size={16} /> Colour and light… <kbd>C</kbd></button>
      <button onclick={startRetouch}><Icon name="details" size={16} /> Red eye, heal, clone, draw… <kbd>E</kbd></button>
      <button onclick={startEffects}><Icon name="details" size={16} /> Frame and watermark… <kbd>F</kbd></button>
      <h3>Save</h3>
      <button onclick={startSave} disabled={!dirty}><Icon name="save" size={16} /> Save as… <kbd>Ctrl+S</kbd></button>
      <button onclick={stepBack} disabled={!edit?.edits}><Icon name="undo" size={16} /> Undo last change <kbd>Ctrl+Z</kbd></button>
      <button onclick={dropEdits} disabled={!dirty}><Icon name="close" size={16} /> Throw away changes</button>
      <h3>File</h3>
      <button onclick={() => cur && revealItem(cur.id)}><Icon name="explorer" size={16} /> Show in Explorer</button>
      <button onclick={() => (pinned = "right")}><Icon name="details" size={16} /> Information (EXIF) <kbd>I</kbd></button>
      <button onclick={() => close()}><Icon name="close" size={16} /> Back to the browser <kbd>Esc</kbd></button>
    {/if}
  </div>

  <!-- RIGHT: file + EXIF information with edit mode -->
  <div class="fly right" class:open={open("right")} onpointerleave={() => (edge = "")} role="complementary" aria-label="Information">
    {#if pinned === "right"}<button class="unpin" onclick={() => (pinned = "")} title="Close (Esc)">✕</button>{/if}
    {#if cur}
      <InfoPanel item={cur} index={i} total={items.length} onchanged={(m) => { say(m); onchanged?.(m); }} onpin={(p) => (pinned = p ? "right" : "")} />
    {/if}
  </div>

  <!-- BOTTOM: navigation + zoom toolbar, with the navigator -->
  <div class="fly bottom" class:open={open("bottom")} onpointerleave={() => (edge = "")} role="toolbar" tabindex="-1" aria-label="Navigation and zoom">
    {#if navBox}
      <!-- Overview: its own frame attached to the toolbar's left end, so the bar keeps its height. -->
      <div class="navframe">
        <div class="nav" role="presentation" onpointerdown={navTo} title="Overview: click or drag to look at that part">
          <img {src} alt="" draggable="false" />
          <i style="left:{navBox.x * 100}%; top:{navBox.y * 100}%; width:{navBox.w * 100}%; height:{navBox.h * 100}%"></i>
        </div>
      </div>
    {/if}
    <div class="vbar">
      <button title="Previous (←, wheel up)" onclick={prev}><Icon name="back" /></button>
      <span class="pos">{i + 1} / {items.length}</span>
      <button title="Next (→, wheel down)" onclick={next}><Icon name="forward" /></button>
      <span class="sep"></span>
      <button title="Zoom out (−)" onclick={() => zoomBy(0.8)}><Icon name="zoom-out" /></button>
      <select title="Zoom" value={zoom ? "cur" : "fit"} onchange={(e) => (e.currentTarget.value === "fit" ? fit() : setZoom((+e.currentTarget.value / 100) * scaleF))}>
        <option value="fit">Fit{zoom ? "" : ` (${pct}%)`}</option>
        {#if zoom}<option value="cur">{pct}%</option>{/if}
        {#each ZOOMS as z}<option value={z}>{z}%</option>{/each}
      </select>
      <button title="Zoom in (+, Ctrl+wheel)" onclick={() => zoomBy(1.25)}><Icon name="zoom-in" /></button>
      <button title="Actual size (1)" onclick={actualSize}><span class="t">1:1</span></button>
      <button title="Fit to screen (0)" onclick={fit}><Icon name="fullscreen" /></button>
      <span class="sep"></span>
      <button title="Rotate left (L)" onclick={() => doEdit({ op: "rotate_left" })}><Icon name="rot-left" /></button>
      <button title="Rotate right (R)" onclick={() => doEdit({ op: "rotate_right" })}><Icon name="rot-right" /></button>
      <button title="Crop (X)" onclick={startCrop}><Icon name="crop" /></button>
      <button title="Resize (Ctrl+R)" onclick={startResize}><Icon name="resize" /></button>
      <button title="Save as (Ctrl+S)" disabled={!dirty} onclick={startSave}><Icon name="save" /></button>
      <span class="sep"></span>
      <button title="Information (I)" onclick={() => (pinned = pinned === "right" ? "" : "right")}><Icon name="details" /></button>
      <button title="Show in Explorer" onclick={() => cur && revealItem(cur.id)}><Icon name="explorer" /></button>
      <button class="exit" title="Back to the browser (Esc)" onclick={() => close()}><Icon name="close" /></button>
    </div>
  </div>
</div>

<style>
  .viewer { position: fixed; inset: 0; z-index: 900; background: #000; overflow: hidden; outline: none; user-select: none; }
  .viewer.nocursor { cursor: none; }
  .photo { position: absolute; left: 50%; top: 50%; transform: translate(-50%, -50%); transform-origin: center; }
  .photo.fit { max-width: 100%; max-height: 100%; }
  .nopic { position: absolute; inset: 0; display: grid; place-content: center; justify-items: center; color: #aaa; gap: 10px; }
  .vstars { color: #ffc83d; margin-left: 10px; letter-spacing: 1px; }
  .vtag { color: #8ab4ff; margin-left: 10px; }
  .info { position: absolute; top: 6px; left: 10px; font: 13px var(--mono); color: #7ee08a; text-shadow: 0 1px 2px #000; pointer-events: none; }
  .unsaved { color: #ffc46b; margin-left: 10px; }
  .msg { position: absolute; top: 34px; left: 50%; transform: translateX(-50%); background: rgba(28, 30, 34, 0.95); color: #e8eaee; padding: 8px 14px; border-radius: 8px; max-width: 70%; z-index: 5; font-size: 14px; }
  .winbtns { position: absolute; top: 0; right: 0; z-index: 6; display: flex; transition: opacity 0.3s; }
  .winbtns.hidden { opacity: 0; }
  .winbtns button { width: 46px; height: 32px; display: grid; place-items: center; background: rgba(28, 30, 34, 0.75); border: none; border-radius: 0; color: #e8eaee; }
  .winbtns button:hover { background: #3a3e45; }
  .winbtns .x:hover { background: #c42b1c; color: #fff; }
  .busy { position: absolute; top: 34px; right: 14px; color: #e8eaee; background: rgba(28, 30, 34, 0.9); padding: 5px 10px; border-radius: 6px; display: flex; gap: 6px; align-items: center; z-index: 5; }

  .fly { position: absolute; background: rgba(28, 30, 34, 0.96); color: #e8eaee; z-index: 4; transition: transform 0.16s ease-out; box-shadow: 0 0 24px rgba(0, 0, 0, 0.6); font-size: 13.5px; }
  .fly h3 { font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; color: #9aa1ab; margin: 10px 12px 4px; font-weight: 600; }
  .fly button { color: inherit; }
  .fly.left { left: 0; top: 50%; width: 260px; transform: translate(-105%, -50%); padding: 6px 6px 12px; border-radius: 0 10px 10px 0; max-height: 94%; overflow: auto; }
  .fly.left.open { transform: translate(0, -50%); }
  .fly.left.wide { width: 330px; overflow-x: hidden; }
  .fly.left > button { display: flex; align-items: center; gap: 9px; width: 100%; background: none; border: none; padding: 5px 10px; border-radius: 5px; font-weight: 400; text-align: left; }
  .fly.left > button:hover:not(:disabled) { background: #3a64c8; }
  .fly.left > button:disabled { opacity: 0.4; }
  .fly.left kbd { margin-left: auto; font: 11px var(--mono); color: #9aa1ab; }
  .fly.right { right: 0; top: 50%; width: 360px; max-height: 94%; overflow: auto; transform: translate(105%, -50%); padding: 6px 4px 12px; border-radius: 10px 0 0 10px; }
  .fly.right.open { transform: translate(0, -50%); }
  .unpin { position: absolute; top: 6px; right: 8px; background: none; border: none; color: #9aa1ab; font-size: 15px; }
  .fly.top { top: 0; left: 0; right: 0; transform: translateY(-105%); padding: 10px 14px 6px; }
  .tophead { display: flex; gap: 14px; align-items: baseline; margin-bottom: 8px; font-size: 14px; }
  .tophead .muted { color: #9aa1ab; font-size: 13px; }
  .fly.top.open { transform: translateY(0); }
  .strip { display: flex; gap: 10px; overflow-x: auto; padding-bottom: 8px; scrollbar-color: #4a4f58 transparent; }
  .st { flex: none; width: 168px; padding: 4px; border: 2px solid transparent; background: #15171a; border-radius: 8px; display: grid; gap: 4px; }
  .st:hover { border-color: #4a4f58; }
  .sp { height: 120px; display: grid; place-items: center; }
  .sn { font-size: 12px; color: #c9ced6; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .st.on { border-color: #6f9bff; background: #1d2a47; }
  .st img { max-width: 100%; max-height: 120px; object-fit: contain; }
  .fly.bottom { bottom: 0; left: 0; right: 0; transform: translateY(105%); padding: 10px 14px; display: flex; align-items: center; gap: 14px; height: 62px; box-sizing: border-box; }
  .fly.bottom.open { transform: translateY(0); }
  .vbar { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; justify-content: center; flex: 1; }
  .vbar button { display: grid; place-items: center; min-width: 42px; height: 42px; padding: 0 8px; background: none; border: 1px solid transparent; border-radius: 8px; }
  .vbar :global(svg) { width: 22px; height: 22px; }
  .vbar button:hover:not(:disabled) { background: #3a3e45; border-color: #4a4f58; }
  .vbar button:disabled { opacity: 0.35; }
  .vbar .exit:hover { background: #b3261e !important; }
  .vbar select { background: #15171a; color: #eee; border: 1px solid #3a3e45; border-radius: 6px; padding: 6px; font-size: 14px; }
  .pos { font: 15px var(--mono); padding: 0 8px; min-width: 76px; text-align: center; }
  .t { font: 600 14px var(--mono); }
  .sep { width: 1px; height: 30px; background: #3a3e45; margin: 0 6px; }
  .navframe { position: absolute; left: 0; bottom: 100%; padding: 8px 8px 6px; background: rgba(28, 30, 34, 0.96);
    border-radius: 0 10px 0 0; box-shadow: 0 -6px 18px rgba(0, 0, 0, 0.45); }
  .nav { position: relative; width: 180px; max-height: 40vh; border: 1px solid #4a4f58; cursor: crosshair; overflow: hidden; }
  .nav img { display: block; width: 100%; }
  .nav i { position: absolute; border: 2px solid #ffc46b; box-shadow: 0 0 0 999px rgba(0, 0, 0, 0.45); }

  .fld { display: grid; gap: 3px; margin: 6px 10px; font-size: 13px; color: #c9ced6; }
  .fld select, .fld input[type="number"] { background: #15171a; color: #eee; border: 1px solid #3a3e45; border-radius: 5px; padding: 5px; }
  .wh { display: grid; grid-template-columns: 1fr 1fr; }
  .chk, .radio { display: flex; gap: 8px; align-items: flex-start; margin: 6px 10px; font-size: 13px; }
  .radio small { color: #9aa1ab; }
  .pcts { display: flex; gap: 4px; margin: 4px 10px; }
  .pcts button, .row2 button { background: #2b2f36; border: 1px solid #3a3e45; border-radius: 6px; padding: 5px 10px; }
  .row2 { display: flex; justify-content: flex-end; gap: 6px; margin: 10px 10px 0; }
  .row2 .primary { background: #3a64c8; border-color: #3a64c8; color: #fff; }
  .row2 kbd { font: 11px var(--mono); opacity: 0.8; margin-left: 4px; }
  .hint { margin: 4px 10px; font-size: 12.5px; color: #9aa1ab; }
  .hint.good { color: #7ee08a; }

  .shade { position: absolute; inset: 0; z-index: 2; cursor: crosshair; }
  .sframe { position: fixed; z-index: 1; pointer-events: none; box-shadow: 0 0 0 9999px rgba(0, 0, 0, 0.6); outline: 1px solid #ffc46b;
    background-image: linear-gradient(rgba(255, 255, 255, 0.28) 1px, transparent 1px), linear-gradient(90deg, rgba(255, 255, 255, 0.28) 1px, transparent 1px);
    background-size: 12.5% 12.5%; }
  .sline { position: fixed; inset: 0; width: 100%; height: 100%; z-index: 3; pointer-events: none; }
  .sline line { stroke: #ffc46b; stroke-width: 2; }
  .fly.left > button.wide-btn { justify-content: center; margin: 6px 0; border: 1px solid #ffffff33; }
  .fly.left > button.wide-btn.on { background: #3a64c8; color: #fff; }
  .anchors { display: grid; grid-template-columns: repeat(3, 28px); gap: 4px; margin: 4px 10px 8px; }
  .anchors button { width: 28px; height: 22px; padding: 0; border-radius: 4px; }
  .anchors button.on { background: #ffc46b; border-color: #ffc46b; }
  .swatches { display: flex; gap: 6px; align-items: center; margin-top: 4px; }
  .sw { width: 24px; height: 24px; padding: 0; border-radius: 50%; border: 2px solid #555; }
  .sw.on { border-color: #ffc46b; }
  .swatches input[type="color"] { width: 34px; height: 26px; padding: 0; border: none; background: none; }
  .pcts button.on { background: #3a64c8; color: #fff; }
  .cropbox { position: fixed; z-index: 3; border: 2px solid #ffc46b; box-shadow: 0 0 0 9999px rgba(0, 0, 0, 0.55); cursor: move; }
  .cropbox::before, .cropbox::after { content: ""; position: absolute; border: 0 dashed rgba(255, 255, 255, 0.35); pointer-events: none; }
  .cropbox::before { left: 33.3%; right: 33.3%; top: 0; bottom: 0; border-left-width: 1px; border-right-width: 1px; }
  .cropbox::after { top: 33.3%; bottom: 33.3%; left: 0; right: 0; border-top-width: 1px; border-bottom-width: 1px; }
  .hd { position: absolute; width: 14px; height: 14px; background: #ffc46b; border-radius: 3px; }
  .hd.nw { left: -8px; top: -8px; cursor: nwse-resize; }
  .hd.ne { right: -8px; top: -8px; cursor: nesw-resize; }
  .hd.sw { left: -8px; bottom: -8px; cursor: nesw-resize; }
  .hd.se { right: -8px; bottom: -8px; cursor: nwse-resize; }
  .dims { position: absolute; bottom: -24px; left: 0; font: 12px var(--mono); color: #ffc46b; background: rgba(0, 0, 0, 0.7); padding: 1px 6px; border-radius: 4px; }
</style>
