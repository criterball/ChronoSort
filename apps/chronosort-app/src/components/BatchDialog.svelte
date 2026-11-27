<script>
  // Batch tools for many files at once:
  //  * Convert / resize: copies in a new folder (originals are never changed).
  //  * Rename: a pattern with a live before → after list; refused while any
  //    name would clash; one Ctrl+Z puts every name back.
  // Long runs show live progress and can be stopped.
  import { onMount, tick } from "svelte";
  import Icon from "./Icon.svelte";
  import {
    batchConvert, batchRename, batchRenamePreview, batchCancel, onProgress, pickFolder,
  } from "../lib/api.js";
  import { app } from "../lib/state.svelte.js";
  import { textWatermarkPng, lastEffects, effectsForBackend } from "../lib/watermark.js";

  // items: browse entries { id, name, kind, dir }; tab: "convert" | "rename";
  // onclose(changed, openFolder?) — openFolder: a folder the browser should go to
  let { items, tab = "convert", onclose } = $props();

  const MEMORY = "chronosort.batch.v1";
  const DEF = {
    format: "JPEG", quality: 90, resize: "long", long: 1920, pct: 50, fitW: 1920, fitH: 1080, folder: "",
    effects: false,
    pattern: "{name}", start: 1, digits: 0, find: "", replace: "", extCase: "keep", sidecars: true,
  };
  function load() {
    try { return { ...DEF, ...JSON.parse(localStorage.getItem(MEMORY) ?? "{}") }; } catch { return { ...DEF }; }
  }
  let o = $state(load());
  function save() {
    try { localStorage.setItem(MEMORY, JSON.stringify($state.snapshot(o))); } catch { /* no memory: fine */ }
  }

  const pictures = $derived(items.filter((e) => e.kind === "image" || e.kind === "raw"));
  // A sidecar (IMG_1.xmp, IMG_1.CR2.xmp) whose photo is in the list goes with that photo,
  // so it is not numbered on its own.
  const stemOf = (n) => (n.lastIndexOf(".") > 0 ? n.slice(0, n.lastIndexOf(".")) : n).toLowerCase();
  const files = $derived.by(() => {
    const all = items.filter((e) => !e.dir);
    if (!o.sidecars) return all;
    const owners = new Set(all.filter((e) => e.kind !== "sidecar").flatMap((e) => [stemOf(e.name), e.name.toLowerCase()]));
    return all.filter((e) => !(e.kind === "sidecar" && owners.has(stemOf(e.name))));
  });
  const followers = $derived(items.filter((e) => !e.dir).length - files.length);
  const fx = lastEffects();
  const fxText = fx ? [fx.fx.frame !== "none" && "frame", fx.fx.wm?.on && "watermark"].filter(Boolean).join(" and ") : "";

  let el = $state();
  let busy = $state(false);
  let prog = $state(null); // { done, total, name }
  let result = $state(null); // text shown after a run
  let error = $state("");
  let rows = $state([]);
  let previewing = $state(false);

  onMount(() => {
    tick().then(() => el?.querySelector("input, select, button.primary")?.focus());
    const un = onProgress("batch-progress", (p) => (prog = p));
    return () => un.then((f) => f()).catch(() => {});
  });

  // ---------- convert ----------
  function resizeOpt() {
    if (o.resize === "long") return { kind: "long", px: Math.max(1, +o.long | 0) };
    if (o.resize === "pct") return { kind: "percent", pct: Math.min(99, Math.max(1, +o.pct | 0)) };
    if (o.resize === "fit") return { kind: "fit", w: Math.max(1, +o.fitW | 0), h: Math.max(1, +o.fitH | 0) };
    return { kind: "none" };
  }
  async function chooseFolder() {
    const f = await pickFolder("Where should the converted copies go?");
    if (f) o.folder = f;
  }
  async function runConvert() {
    if (!pictures.length) return (error = "There are no photos among these files.");
    save();
    error = "";
    result = null;
    busy = true;
    app.busy = true;
    prog = { done: 0, total: pictures.length, name: "" };
    try {
      const useFx = o.effects && fx;
      const opts = {
        format: o.format, quality: +o.quality, resize: resizeOpt(), folder: o.folder || null,
        effects: useFx ? effectsForBackend(fx.fx) : null,
        watermarkPng: useFx && fx.fx.wm?.on && fx.src !== "logo" ? textWatermarkPng(fx.txt) : null,
        watermarkPath: useFx && fx.fx.wm?.on && fx.src === "logo" ? fx.logoPath || null : null,
      };
      const r = await batchConvert(pictures.map((e) => e.id), opts);
      const parts = [`${r.done} ${r.done === 1 ? "copy" : "copies"} made in “${r.folder.split(/[\\/]/).pop()}”`];
      if (r.cancelled) parts.push("stopped before the end");
      if (r.failed.length) parts.push(`${r.failed.length} could not be converted: ${r.failed.slice(0, 5).map(([n, w]) => `${n} (${w})`).join(", ")}${r.failed.length > 5 ? " …" : ""}`);
      result = { text: parts.join(" · ") + (r.done ? ". Ctrl+Z in the browser removes them again." : "."), folder: r.done ? r.folder : null };
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
      app.busy = false;
      prog = null;
    }
  }

  // ---------- rename ----------
  const tz = -new Date().getTimezoneOffset();
  const renameOpts = () => ({
    pattern: o.pattern, start: +o.start | 0, step: 1, digits: +o.digits | 0, find: o.find, replace: o.replace,
    extCase: o.extCase, sidecars: o.sidecars, tzOffsetMin: tz,
  });
  let ptimer;
  $effect(() => {
    if (tab !== "rename") return;
    // Re-run the preview whenever an option changes.
    JSON.stringify([o.pattern, o.start, o.digits, o.find, o.replace, o.extCase, o.sidecars, files.length]);
    clearTimeout(ptimer);
    ptimer = setTimeout(preview, 250);
  });
  async function preview() {
    if (!files.length) return;
    previewing = true;
    try {
      rows = await batchRenamePreview(files.map((e) => e.id), renameOpts());
      error = "";
    } catch (e) {
      rows = [];
      error = String(e);
    } finally {
      previewing = false;
    }
  }
  const problems = $derived(rows.filter((r) => r.problem).length);
  const changes = $derived(rows.filter((r) => r.new !== r.old).length);
  function addToken(t) {
    o.pattern = (o.pattern + " " + t).trim();
  }
  const PRESETS = [
    ["{name}", "Keep the names"],
    ["{year}-{month}-{day} {hour}.{minute}.{second}", "Date and time taken"],
    ["{year}-{month}-{day} {num}", "Date taken + number"],
    ["Holiday {num}", "Text + number"],
    ["{folder} {num}", "Folder name + number"],
  ];
  async function runRename() {
    if (problems || !changes) return;
    save();
    error = "";
    result = null;
    busy = true;
    app.busy = true;
    prog = { done: 0, total: changes, name: "" };
    try {
      const r = await batchRename(files.map((e) => e.id), renameOpts());
      result = { text: `${r.renamed} ${r.renamed === 1 ? "file" : "files"} renamed${r.sidecars ? ` (and ${r.sidecars} sidecar ${r.sidecars === 1 ? "file" : "files"})` : ""}. Ctrl+Z in the browser puts the old names back.`, folder: null, renamed: true };
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
      app.busy = false;
      prog = null;
    }
  }

  function close() {
    if (busy) return;
    onclose?.(!!result);
  }
  function onKey(e) {
    if (e.key === "Escape") { e.stopPropagation(); close(); }
  }
</script>

<div class="back batch-dlg" role="presentation" onkeydown={onKey}>
  <div class="dlg" bind:this={el} role="dialog" aria-modal="true" aria-label="Batch">
    <header>
      <h2>Batch</h2>
      <div class="tabs" role="tablist">
        <button role="tab" aria-selected={tab === "convert"} class:on={tab === "convert"} disabled={busy} onclick={() => { tab = "convert"; result = null; error = ""; }}>Convert / resize <kbd>F3</kbd></button>
        <button role="tab" aria-selected={tab === "rename"} class:on={tab === "rename"} disabled={busy} onclick={() => { tab = "rename"; result = null; error = ""; }}>Rename <kbd>F4</kbd></button>
      </div>
      <button class="x" onclick={close} disabled={busy} aria-label="Close"><Icon name="close" /></button>
    </header>

    {#if tab === "convert"}
      <p class="muted">{pictures.length} {pictures.length === 1 ? "photo" : "photos"}{files.length > pictures.length ? ` (${files.length - pictures.length} other files are skipped)` : ""}. New copies are made; <strong>your originals are not changed</strong>.</p>
      <div class="grid">
        <label class="fld">Save as
          <select bind:value={o.format}>
            <option value="keep">Same type as the original</option>
            <option>JPEG</option><option>PNG</option><option>TIFF</option><option>WebP</option>
          </select>
        </label>
        {#if o.format === "JPEG" || o.format === "WebP" || o.format === "keep"}
          <label class="fld">Quality {o.quality}<input type="range" min="50" max="100" bind:value={o.quality} /></label>
        {/if}
      </div>
      <div class="lab">Size</div>
      <div class="opts">
        <label class="radio"><input type="radio" bind:group={o.resize} value="none" /> Keep the size</label>
        <label class="radio"><input type="radio" bind:group={o.resize} value="long" /> Longest side
          <input class="num" type="number" min="16" bind:value={o.long} onfocus={() => (o.resize = "long")} /> pixels
          <span class="chips">{#each [[1280, "e-mail"], [1920, "screen"], [3840, "4K"]] as [v, t]}<button onclick={() => { o.resize = "long"; o.long = v; }}>{t}</button>{/each}</span>
        </label>
        <label class="radio"><input type="radio" bind:group={o.resize} value="pct" /> <input class="num" type="number" min="1" max="99" bind:value={o.pct} onfocus={() => (o.resize = "pct")} /> % of the original</label>
        <label class="radio"><input type="radio" bind:group={o.resize} value="fit" /> Fit inside
          <input class="num" type="number" min="16" bind:value={o.fitW} onfocus={() => (o.resize = "fit")} /> ×
          <input class="num" type="number" min="16" bind:value={o.fitH} onfocus={() => (o.resize = "fit")} /> pixels</label>
      </div>
      <p class="hint">Pictures are only made smaller, never bigger. Dates and photo information are kept. RAW photos use the camera's own JPEG inside them.</p>
      <div class="lab">Where</div>
      <div class="where">
        <span class="path">{o.folder || "A new “Converted” folder next to the photos"}</span>
        <button onclick={chooseFolder}>Choose…</button>
        {#if o.folder}<button onclick={() => (o.folder = "")}>Use “Converted”</button>{/if}
      </div>
      {#if fx && fxText}
        <label class="chk"><input type="checkbox" bind:checked={o.effects} /> Add my {fxText} (as last used in the viewer)</label>
      {:else}
        <p class="hint">Tip: set up a frame or watermark once in the viewer (key F); it can then be added here to all photos.</p>
      {/if}
    {:else}
      <p class="muted">{files.length} {files.length === 1 ? "file" : "files"}, in the order shown in the browser{#if followers}, plus {followers} sidecar {followers === 1 ? "file that follows its photo" : "files that follow their photos"}{/if}.</p>
      <label class="fld">New name
        <input class="pat" type="text" bind:value={o.pattern} spellcheck="false" />
      </label>
      <div class="chips">
        {#each [["{name}", "old name"], ["{num}", "number"], ["{year}-{month}-{day}", "date taken"], ["{hour}.{minute}.{second}", "time"], ["{folder}", "folder name"]] as [t, l]}
          <button onclick={() => addToken(t)} title="Add {t}">+ {l}</button>
        {/each}
        <select onchange={(e) => { if (e.currentTarget.value) o.pattern = e.currentTarget.value; e.currentTarget.value = ""; }} aria-label="Ready-made patterns">
          <option value="">Ready-made…</option>
          {#each PRESETS as [v, t]}<option value={v}>{t}</option>{/each}
        </select>
      </div>
      <div class="grid4">
        <label class="fld">First number<input type="number" bind:value={o.start} /></label>
        <label class="fld">Digits
          <select bind:value={o.digits}><option value={0}>Automatic</option>{#each [1, 2, 3, 4, 5] as d}<option value={d}>{d} ({"0".repeat(d - 1)}1)</option>{/each}</select>
        </label>
        <label class="fld">Replace<input type="text" bind:value={o.find} placeholder="text" /></label>
        <label class="fld">with<input type="text" bind:value={o.replace} /></label>
      </div>
      <div class="row">
        <label class="fld inline">Extension
          <select bind:value={o.extCase}><option value="keep">as it is</option><option value="lower">small letters (.jpg)</option><option value="upper">CAPITALS (.JPG)</option></select>
        </label>
        <label class="chk"><input type="checkbox" bind:checked={o.sidecars} /> Also rename matching sidecar files (.xmp, .aae, .thm, .json)</label>
      </div>
      <div class="table" aria-live="polite">
        <div class="th"><span>Now</span><span></span><span>New name</span></div>
        {#each rows.slice(0, 400) as r (r.id)}
          <div class="tr" class:bad={r.problem} class:same={r.new === r.old}>
            <span title={r.old}>{r.old}</span><span class="arrow">→</span>
            <span title={r.new}>{r.new}{#if r.date_from === "file"}<em title="No date taken was found inside this file, so the file's own date was used."> (file date)</em>{/if}{#if r.problem}<b> — {r.problem}</b>{/if}</span>
          </div>
        {/each}
        {#if rows.length > 400}<div class="tr more">… and {rows.length - 400} more</div>{/if}
        {#if previewing && !rows.length}<div class="tr more">Working out the new names…</div>{/if}
      </div>
      <p class="hint">{#if problems}<b class="badtxt">{problems} {problems === 1 ? "name needs" : "names need"} a change before renaming.</b>{:else}{changes} {changes === 1 ? "file gets" : "files get"} a new name.{/if}</p>
    {/if}

    {#if prog}
      <div class="prog">
        <div class="bar"><div style="width:{prog.total ? (100 * prog.done) / prog.total : 0}%"></div></div>
        <span>{prog.done} of {prog.total}{prog.name ? ` · ${prog.name}` : ""}</span>
        {#if tab === "convert"}<button onclick={() => batchCancel()}>Stop</button>{/if}
      </div>
    {/if}
    {#if error}<p class="err">{error}</p>{/if}
    {#if result}<p class="ok">{result.text}{#if result.folder} <button class="link" onclick={() => onclose?.(true, result.folder)}>Open the folder</button>{/if}</p>{/if}

    <footer>
      <button onclick={close} disabled={busy}>{result ? "Close" : "Cancel"}</button>
      {#if tab === "convert"}
        <button class="primary" onclick={runConvert} disabled={busy || !pictures.length} data-continue>Convert {pictures.length} {pictures.length === 1 ? "photo" : "photos"}</button>
      {:else}
        <button class="primary" onclick={runRename} disabled={busy || !!problems || !changes || previewing} data-continue>Rename {changes} {changes === 1 ? "file" : "files"}</button>
      {/if}
    </footer>
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; z-index: 50; background: rgba(0, 0, 0, 0.55); display: grid; place-items: center; }
  .dlg { width: min(760px, calc(100vw - 32px)); max-height: calc(100vh - 32px); overflow: auto; background: var(--surface);
    color: var(--text); border: 1px solid var(--border); border-radius: 14px; padding: 16px 20px; box-shadow: 0 20px 60px #0008; }
  header { display: flex; align-items: center; gap: 14px; margin-bottom: 6px; }
  h2 { margin: 0; font-size: 20px; }
  .tabs { display: flex; gap: 4px; flex: 1; }
  .tabs button { padding: 6px 12px; }
  .tabs button.on { background: var(--accent); border-color: var(--accent); color: var(--accent-text); }
  .x { background: none; border: none; padding: 4px; }
  .muted { color: var(--muted); }
  .hint { color: var(--muted); font-size: 12.5px; margin: 6px 0; }
  .lab { font-weight: 600; margin: 12px 0 4px; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; }
  .grid4 { display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; margin-top: 8px; }
  .fld { display: flex; flex-direction: column; gap: 4px; font-size: 13px; }
  .fld.inline { flex-direction: row; align-items: center; gap: 8px; }
  .opts { display: flex; flex-direction: column; gap: 6px; }
  .radio { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .num { width: 80px; }
  .chips { display: inline-flex; gap: 4px; flex-wrap: wrap; align-items: center; margin-top: 6px; }
  .chips button { padding: 2px 8px; font-size: 12.5px; }
  .where { display: flex; align-items: center; gap: 8px; }
  .path { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted); }
  .chk { display: flex; align-items: center; gap: 8px; margin-top: 12px; }
  .row { display: flex; gap: 18px; align-items: center; flex-wrap: wrap; margin-top: 8px; }
  .row .chk { margin-top: 0; }
  .pat { font-family: Consolas, "Cascadia Mono", monospace; font-size: 14px; }
  .table { margin-top: 10px; border: 1px solid var(--border); border-radius: 8px; max-height: 260px; overflow: auto; font-size: 13px; }
  .th, .tr { display: grid; grid-template-columns: 1fr 24px 1.4fr; gap: 4px; padding: 3px 8px; }
  .th { position: sticky; top: 0; background: var(--surface-2); font-weight: 600; }
  .tr span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .tr.same { color: var(--muted); }
  .tr.bad { background: color-mix(in srgb, var(--red) 14%, transparent); }
  .tr b, .badtxt { color: var(--red); font-weight: 600; }
  .tr em { color: var(--muted); font-style: normal; }
  .arrow { color: var(--muted); text-align: center; }
  .more { color: var(--muted); display: block; }
  .prog { display: flex; align-items: center; gap: 10px; margin-top: 12px; }
  .prog .bar { flex: 1; }
  .err { color: var(--red); }
  .ok { color: var(--green); }
  .link { background: none; border: none; color: var(--accent); text-decoration: underline; padding: 0 4px; }
  footer { display: flex; justify-content: flex-end; gap: 8px; margin-top: 14px; }
  footer button { padding: 8px 16px; }
</style>
