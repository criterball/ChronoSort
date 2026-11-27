<script>
  // "Create" from many files: a contact sheet (thumbnails with names on a page),
  // one picture joined from several (a strip), a PDF with a photo per page, or a
  // ZIP to send. The new file goes next to the first file; nothing is replaced,
  // and Ctrl+Z in the browser removes it again.
  //
  // Sheets and strips are laid out here; their text is drawn here on a canvas and
  // sent as a transparent layer, so the result uses the same fonts as the app.
  import { onMount, tick, untrack } from "svelte";
  import Icon from "./Icon.svelte";
  import { createSheet, createPdf, createZip, createSizes, onProgress } from "../lib/api.js";
  import { app } from "../lib/state.svelte.js";

  // items: entries { id, name, kind, modified }; folder: the folder's name; tab; onclose(changed, selectName?)
  let { items, folder = "", tab = "sheet", onclose } = $props();

  const MEMORY = "chronosort.create.v1";
  const DEF = {
    paper: "a4", landscape: false, dpi: 150, cols: 5, caption: "name", title: true,
    dir: "row", height: 1200, gap: 12, bg: "#ffffff", fmt: "JPEG",
    pdfPage: "a4", margin: 10,
  };
  function load() {
    try { return { ...DEF, ...JSON.parse(localStorage.getItem(MEMORY) ?? "{}") }; } catch { return { ...DEF }; }
  }
  let o = $state(load());
  const save = () => { try { localStorage.setItem(MEMORY, JSON.stringify($state.snapshot(o))); } catch { /* fine */ } };

  const media = $derived(items.filter((e) => e.kind === "image" || e.kind === "raw" || e.kind === "video"));
  // Starting names from the folder name (the user can change them).
  let names = $state(untrack(() => ({ sheet: `Contact sheet ${folder}`.trim(), strip: `${folder || "Pictures"} joined`, pdf: folder || "Photos", zip: folder || "Photos" })));

  let el = $state();
  let busy = $state(false);
  let prog = $state(null);
  let result = $state(null);
  let error = $state("");

  onMount(() => {
    tick().then(() => el?.querySelector("button.primary")?.focus());
    const un = onProgress("create-progress", (p) => (prog = p));
    return () => un.then((f) => f()).catch(() => {});
  });

  const rgb = (h) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  const PAPER = { a4: [210, 297], letter: [215.9, 279.4] };
  const dateOf = (e) => (e.modified ? new Date(e.modified * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" }) : "");

  // ---------- contact sheet layout ----------
  // Page size in pixels, cell boxes (4:3) with a caption under each, a title on top.
  function sheetLayout() {
    let [wmm, hmm] = PAPER[o.paper];
    if (o.landscape) [wmm, hmm] = [hmm, wmm];
    const W = Math.round((wmm / 25.4) * o.dpi), H = Math.round((hmm / 25.4) * o.dpi);
    const m = Math.round(W * 0.045), g = Math.round(W * 0.018);
    const font = Math.max(10, Math.round(W * 0.0125));
    const titleH = o.title ? Math.round(font * 3) : 0;
    const capLines = o.caption === "none" ? 0 : o.caption === "date" ? 2 : 1;
    const capH = capLines ? Math.round(font * 1.35 * capLines + font * 0.4) : 0;
    const cols = Math.max(1, Math.min(12, +o.cols | 0));
    const cw = Math.floor((W - 2 * m - (cols - 1) * g) / cols);
    const bh = Math.round(cw * 0.75);
    const rowH = bh + capH;
    const rows = Math.max(1, Math.floor((H - 2 * m - titleH + g) / (rowH + g)));
    return { W, H, m, g, font, titleH, capH, cols, cw, bh, rowH, rows, perPage: cols * rows };
  }
  let lay = $derived(sheetLayout());
  let sheetPages = $derived(Math.max(1, Math.ceil(media.length / lay.perPage)));

  function ellipsis(x, text, max) {
    if (x.measureText(text).width <= max) return text;
    let lo = 0, hi = text.length;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (x.measureText(text.slice(0, mid) + "…").width <= max) lo = mid; else hi = mid - 1;
    }
    return text.slice(0, lo) + "…";
  }
  function sheetOverlay(L, page, list, pages) {
    const c = document.createElement("canvas");
    c.width = L.W; c.height = L.H;
    const x = c.getContext("2d");
    const ink = rgb(o.bg).reduce((a, b) => a + b, 0) > 382 ? "#1d1f23" : "#f2f3f5";
    x.fillStyle = ink;
    x.textBaseline = "top";
    if (o.title) {
      x.font = `600 ${Math.round(L.font * 1.6)}px "Segoe UI", system-ui, sans-serif`;
      const t = folder + (pages > 1 ? `  ·  page ${page + 1} of ${pages}` : "");
      x.fillText(ellipsis(x, t, L.W - 2 * L.m), L.m, L.m);
    }
    x.font = `${L.font}px "Segoe UI", system-ui, sans-serif`;
    x.textAlign = "center";
    list.forEach((e, i) => {
      if (o.caption === "none") return;
      const col = i % L.cols, row = Math.floor(i / L.cols);
      const cx = L.m + col * (L.cw + L.g) + L.cw / 2;
      const y = L.m + L.titleH + row * (L.rowH + L.g) + L.bh + L.font * 0.3;
      x.fillText(ellipsis(x, e.name, L.cw), cx, y);
      if (o.caption === "date") {
        x.globalAlpha = 0.7;
        x.fillText(dateOf(e), cx, y + L.font * 1.35);
        x.globalAlpha = 1;
      }
    });
    return c.toDataURL("image/png");
  }

  // ---------- strip layout ----------
  async function stripLayout() {
    const sizes = await createSizes(media.map((e) => e.id));
    const ok = media.map((e, i) => ({ e, s: sizes[i] })).filter((x) => x.s);
    if (!ok.length) throw new Error("The sizes of these pictures could not be read.");
    const gap = Math.max(0, +o.gap | 0);
    const side = Math.max(100, +o.height | 0); // height of a row strip, or width of a column strip
    let pos = gap, cells = [];
    for (const { s } of ok) {
      const [w, h] = s;
      if (o.dir === "row") {
        const cw = Math.max(1, Math.round((w / h) * side));
        cells.push({ x: pos, y: gap, w: cw, h: side });
        pos += cw + gap;
      } else {
        const ch = Math.max(1, Math.round((h / w) * side));
        cells.push({ x: gap, y: pos, w: side, h: ch });
        pos += ch + gap;
      }
    }
    let W = o.dir === "row" ? pos : side + 2 * gap;
    let H = o.dir === "row" ? side + 2 * gap : pos;
    // Keep it under 120 megapixels: shrink everything evenly if needed.
    const k = Math.min(1, Math.sqrt(118e6 / (W * H)));
    if (k < 1) {
      cells = cells.map((c) => ({ x: Math.floor(c.x * k), y: Math.floor(c.y * k), w: Math.max(1, Math.floor(c.w * k)), h: Math.max(1, Math.floor(c.h * k)) }));
      W = Math.floor(W * k); H = Math.floor(H * k);
    }
    return { ids: ok.map((x) => x.e.id), cells, W, H };
  }

  // ---------- run ----------
  async function run() {
    if (!media.length && tab !== "zip") return (error = "There are no photos or videos among these files.");
    save();
    error = "";
    result = null;
    busy = true;
    app.busy = true;
    try {
      if (tab === "sheet") {
        const L = sheetLayout();
        const made = [];
        for (let p = 0; p < sheetPages; p++) {
          const list = media.slice(p * L.perPage, (p + 1) * L.perPage);
          prog = { done: 0, total: list.length, name: `page ${p + 1} of ${sheetPages}` };
          const cells = list.map((_, i) => ({
            x: L.m + (i % L.cols) * (L.cw + L.g), y: L.m + L.titleH + Math.floor(i / L.cols) * (L.rowH + L.g), w: L.cw, h: L.bh,
          }));
          const name = sheetPages > 1 ? `${names.sheet} - page ${p + 1}` : names.sheet;
          const r = await createSheet(list.map((e) => e.id), {
            width: L.W, height: L.H, background: rgb(o.bg), cells, fit: "contain",
            overlay: o.title || o.caption !== "none" ? sheetOverlay(L, p, list, sheetPages) : null,
            format: o.fmt, quality: 90, name,
          });
          made.push(r);
        }
        result = { text: made.length === 1 ? `“${made[0].name}” made.` : `${made.length} sheets made.`, name: made[0].name, skipped: made.flatMap((m) => m.skipped) };
      } else if (tab === "strip") {
        prog = { done: 0, total: media.length, name: "" };
        const L = await stripLayout();
        const r = await createSheet(L.ids, { width: L.W, height: L.H, background: rgb(o.bg), cells: L.cells, fit: "contain", overlay: null, format: o.fmt, quality: 92, name: names.strip });
        result = { text: `“${r.name}” made (${L.W} × ${L.H} pixels).`, name: r.name, skipped: r.skipped };
      } else if (tab === "pdf") {
        prog = { done: 0, total: media.length, name: "" };
        const r = await createPdf(media.map((e) => e.id), { page: o.pdfPage, marginMm: +o.margin, name: names.pdf });
        result = { text: `“${r.name}” made, ${media.length - r.skipped.length} ${media.length - r.skipped.length === 1 ? "page" : "pages"}.`, name: r.name, skipped: r.skipped };
      } else {
        const files = items.filter((e) => !e.dir);
        prog = { done: 0, total: files.length, name: "" };
        const r = await createZip(files.map((e) => e.id), names.zip);
        result = { text: `“${r.name}” made with ${files.length} ${files.length === 1 ? "file" : "files"}.`, name: r.name, skipped: r.skipped };
      }
    } catch (e) {
      error = String(e?.message ?? e);
    } finally {
      busy = false;
      app.busy = false;
      prog = null;
    }
  }
  function close() {
    if (!busy) onclose?.(!!result, result?.name);
  }
  const TABS = [["sheet", "Contact sheet"], ["strip", "Join into one picture"], ["pdf", "PDF"], ["zip", "ZIP"]];
</script>

<div class="back create-dlg" role="presentation" onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); close(); } }}>
  <div class="dlg" bind:this={el} role="dialog" aria-modal="true" aria-label="Create">
    <header>
      <h2>Create</h2>
      <div class="tabs" role="tablist">
        {#each TABS as [k, t]}
          <button role="tab" aria-selected={tab === k} class:on={tab === k} disabled={busy} onclick={() => { tab = k; result = null; error = ""; }}>{t}</button>
        {/each}
      </div>
      <button class="x" onclick={close} disabled={busy} aria-label="Close"><Icon name="close" /></button>
    </header>

    {#if tab === "sheet"}
      <p class="muted">An overview page of {media.length} {media.length === 1 ? "picture" : "pictures"} with their names — to print, or to keep as a picture.{#if sheetPages > 1} They need <strong>{sheetPages} sheets</strong> ({lay.perPage} per sheet).{/if}</p>
      <div class="grid">
        <label class="fld">Paper
          <select bind:value={o.paper}><option value="a4">A4</option><option value="letter">Letter</option></select></label>
        <label class="fld">Turn
          <select bind:value={o.landscape}><option value={false}>Upright (portrait)</option><option value={true}>Sideways (landscape)</option></select></label>
        <label class="fld">Sharpness
          <select bind:value={o.dpi}><option value={150}>For the screen (150 dpi)</option><option value={300}>For printing (300 dpi)</option></select></label>
        <label class="fld">Pictures per row {o.cols}<input type="range" min="2" max="10" bind:value={o.cols} /></label>
      </div>
      <div class="seg">
        {#each [["none", "No names"], ["name", "File names"], ["date", "Names and dates"]] as [k, t]}<button class:on={o.caption === k} onclick={() => (o.caption = k)}>{t}</button>{/each}
      </div>
      <label class="chk"><input type="checkbox" bind:checked={o.title} /> Folder name as a title</label>
    {:else if tab === "strip"}
      <p class="muted">Puts {media.length} {media.length === 1 ? "picture" : "pictures"} side by side (or one below the other) in one new picture.</p>
      <div class="seg">
        <button class:on={o.dir === "row"} onclick={() => (o.dir = "row")}>Side by side</button>
        <button class:on={o.dir === "col"} onclick={() => (o.dir = "col")}>One below the other</button>
      </div>
      <div class="grid">
        <label class="fld">{o.dir === "row" ? "Height" : "Width"} in pixels<input type="number" min="100" max="20000" bind:value={o.height} /></label>
        <label class="fld">Space between them (pixels)<input type="number" min="0" max="500" bind:value={o.gap} /></label>
      </div>
    {:else if tab === "pdf"}
      <p class="muted">A PDF document with one photo per page ({media.length} {media.length === 1 ? "page" : "pages"}) — easy to e-mail or print. Upright photos get upright pages, wide photos sideways pages.</p>
      <div class="grid">
        <label class="fld">Paper<select bind:value={o.pdfPage}><option value="a4">A4</option><option value="letter">Letter</option></select></label>
        <label class="fld">White edge
          <select bind:value={o.margin}><option value={0}>None</option><option value={10}>Narrow (1 cm)</option><option value={20}>Wide (2 cm)</option></select></label>
      </div>
    {:else}
      <p class="muted">Packs {items.filter((e) => !e.dir).length} files into one ZIP file — handy for sending. The files themselves stay where they are.</p>
    {/if}

    {#if tab === "sheet" || tab === "strip"}
      <div class="grid">
        <label class="fld">Background
          <span class="sws">{#each ["#ffffff", "#000000", "#2b2d33", "#f3ede2"] as c}<button class="sw" class:on={o.bg === c} style="background:{c}" aria-label={c} onclick={() => (o.bg = c)}></button>{/each}<input type="color" bind:value={o.bg} aria-label="Other colour" /></span>
        </label>
        <label class="fld">Save as<select bind:value={o.fmt}><option>JPEG</option><option>PNG</option></select></label>
      </div>
    {/if}
    <label class="fld name">Name of the new file
      <span class="nm"><input type="text" bind:value={names[tab]} /><em>.{tab === "pdf" ? "pdf" : tab === "zip" ? "zip" : o.fmt === "PNG" ? "png" : "jpg"}</em></span>
    </label>
    <p class="hint">It is saved in this folder, next to the {tab === "zip" ? "files" : "pictures"}. Nothing is replaced; Ctrl+Z in the browser removes it again.</p>

    {#if prog}
      <div class="prog">
        <div class="bar" class:indeterminate={!prog.total}><div style="width:{prog.total ? (100 * prog.done) / prog.total : 35}%"></div></div>
        <span>{prog.total ? `${prog.done} of ${prog.total}` : "Working…"}{prog.name ? ` · ${prog.name}` : ""}</span>
      </div>
    {/if}
    {#if error}<p class="err">{error}</p>{/if}
    {#if result}
      <p class="ok">{result.text}{#if result.skipped?.length} {result.skipped.length} could not be used: {result.skipped.slice(0, 4).map(([n, w]) => `${n} (${w})`).join(", ")}.{/if}</p>
    {/if}

    <footer>
      <button onclick={close} disabled={busy}>{result ? "Close and show it" : "Cancel"}</button>
      <button class="primary" onclick={run} disabled={busy || (tab !== "zip" && !media.length)} data-continue>
        {tab === "sheet" ? (sheetPages > 1 ? `Make ${sheetPages} sheets` : "Make the contact sheet") : tab === "strip" ? "Join them" : tab === "pdf" ? "Make the PDF" : "Make the ZIP"}
      </button>
    </footer>
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; z-index: 50; background: rgba(0, 0, 0, 0.55); display: grid; place-items: center; }
  .dlg { width: min(720px, calc(100vw - 32px)); max-height: calc(100vh - 32px); overflow: auto; background: var(--surface);
    color: var(--text); border: 1px solid var(--border); border-radius: 14px; padding: 16px 20px; box-shadow: 0 20px 60px #0008; }
  header { display: flex; align-items: center; gap: 14px; margin-bottom: 6px; }
  h2 { margin: 0; font-size: 20px; }
  .tabs { display: flex; gap: 4px; flex: 1; flex-wrap: wrap; }
  .tabs button { padding: 6px 10px; }
  .tabs button.on, .seg button.on { background: var(--accent); border-color: var(--accent); color: var(--accent-text); }
  .x { background: none; border: none; padding: 4px; }
  .muted { color: var(--muted); }
  .hint { color: var(--muted); font-size: 12.5px; margin: 6px 0; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 10px 14px; margin-top: 10px; }
  .fld { display: flex; flex-direction: column; gap: 4px; font-size: 13px; }
  .fld.name { margin-top: 12px; }
  .nm { display: flex; align-items: center; gap: 4px; }
  .nm input { flex: 1; }
  .nm em { color: var(--muted); font-style: normal; }
  .seg { display: flex; gap: 6px; margin-top: 10px; }
  .seg button { flex: 1; padding: 6px 8px; }
  .chk { display: flex; align-items: center; gap: 8px; margin-top: 10px; }
  .sws { display: flex; align-items: center; gap: 6px; }
  .sw { width: 26px; height: 26px; padding: 0; border-radius: 6px; border: 2px solid var(--border); }
  .sw.on { outline: 2px solid var(--accent); outline-offset: 1px; }
  input[type="color"] { width: 34px; height: 28px; padding: 0; border: none; background: none; }
  .prog { display: flex; align-items: center; gap: 10px; margin-top: 12px; }
  .prog .bar { flex: 1; }
  .err { color: var(--red); }
  .ok { color: var(--green); }
  footer { display: flex; justify-content: flex-end; gap: 8px; margin-top: 14px; }
  footer button { padding: 8px 16px; }
</style>
