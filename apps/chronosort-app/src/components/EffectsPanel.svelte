<script module>
  // The text watermark's wording and look, kept while the app runs (one per session, like a pen).
  const lastText = { text: `© ${new Date().getFullYear()}`, bold: true, color: "#ffffff", outline: true };
</script>

<script>
  // Frame and watermark for the picture being edited. Like the colour panel,
  // every change is sent to the backend (debounced, one request at a time) and
  // shows in the preview; the file only changes on Save. The effects are
  // applied last, so a white frame stays white whatever the colour look.
  //
  // A text watermark is drawn here on a canvas (big, so it stays sharp on large
  // photos) and handed over as a PNG; a logo is read by the backend from its file.
  import { untrack } from "svelte";
  import { editEffects, editWatermark, pickPicture } from "../lib/api.js";
  import { textWatermarkPng, lastEffects, effectsForBackend, EFFECTS_MEMORY } from "../lib/watermark.js";

  // start: the edit state when the panel opened; onstate(editState); onclose(); onerror(message)
  let { start, onstate, onclose, onerror } = $props();

  const FRAMES = [
    ["none", "None"], ["border", "Border"], ["matte", "Mat with line"], ["instant", "Instant photo"],
    ["rounded", "Round corners"], ["shadow", "Shadow"], ["vignette", "Soft edges"],
  ];
  const SIZE_LABEL = { rounded: "Corner size", vignette: "Strength" };
  const ACCENT_LABEL = { matte: "Line colour", shadow: "Shadow colour" };
  const COLOURS = ["#ffffff", "#f3ede2", "#000000", "#808080"];

  const hex = (c) => "#" + c.map((v) => v.toString(16).padStart(2, "0")).join("");

  // ---- settings ----
  const init = untrack(() => start?.effects);
  let fx = $state({
    frame: init?.frame ?? "none",
    size: init?.size ?? 5,
    color: hex(init?.color ?? [255, 255, 255]),
    accent: hex(init?.accent ?? [0, 0, 0]),
    wm: { ...(init?.watermark ?? { on: false, anchor: 8, width: 25, margin: 3, opacity: 70 }) },
  });
  // Watermark source: "text" or "logo"; the text and how it looks.
  let src = $state(untrack(() => (start?.watermark && start.watermark !== "text" ? "logo" : "text")));
  let logoName = $state(untrack(() => (start?.watermark && start.watermark !== "text" ? start.watermark : "")));
  let logoPath = $state(untrack(() => lastEffects()?.logoPath ?? ""));
  let txt = $state({ ...lastText });
  $effect(() => { Object.assign(lastText, $state.snapshot(txt)); });
  let saved = $state(lastEffects());

  function remember() {
    try {
      // The logo's place is kept too, so the batch converter can use the same watermark.
      const v = { fx: $state.snapshot(fx), txt: $state.snapshot(txt), src, logoPath };
      localStorage.setItem(EFFECTS_MEMORY, JSON.stringify(v));
      saved = v;
    } catch { /* private window: no memory, nothing else changes */ }
  }

  // ---- sending (debounced, never two at once) ----
  let timer, sending = false, again = false;
  function send(now = false) {
    clearTimeout(timer);
    timer = setTimeout(flush, now ? 0 : 60);
  }
  async function flush() {
    if (sending) { again = true; return; }
    sending = true;
    try {
      onstate?.(await editEffects(effectsForBackend($state.snapshot(fx))));
    } catch (e) {
      onerror?.(String(e));
    } finally {
      sending = false;
      if (again) { again = false; flush(); }
    }
  }

  // ---- the watermark picture ----
  const textPng = () => textWatermarkPng(txt);
  let textTimer;
  function textChanged() {
    clearTimeout(textTimer);
    textTimer = setTimeout(applyText, 250);
  }
  async function applyText() {
    if (src !== "text" || !fx.wm.on) return;
    const png = textPng();
    try {
      onstate?.(png ? await editWatermark(png, null) : await editWatermark(null, null));
    } catch (e) {
      onerror?.(String(e));
    }
  }
  async function chooseLogo() {
    const p = await pickPicture("Choose a logo picture (PNG with a transparent background works best)");
    if (!p) return;
    try {
      onstate?.(await editWatermark(null, p));
      logoPath = p;
      logoName = p.split(/[\\/]/).pop();
    } catch (e) {
      onerror?.(String(e));
    }
  }
  async function toggleWatermark(on) {
    fx.wm.on = on;
    if (on && src === "text") await applyText();
    send(true);
  }
  async function setSource(s) {
    src = s;
    if (s === "text") await applyText();
    else if (logoName) send(true);
    else {
      try { onstate?.(await editWatermark(null, null)); } catch (e) { onerror?.(String(e)); }
    }
  }

  function setFrame(f) {
    if (f !== fx.frame && f !== "none") {
      // Each style starts at its own sensible size and colour; change them afterwards.
      fx.size = { border: 4, matte: 10, instant: 5, rounded: 8, shadow: 4, vignette: 15 }[f] ?? 5;
      if (f === "vignette" && fx.color === "#ffffff") fx.color = "#000000";
      else if (f !== "vignette" && f !== "border" && fx.color === "#000000") fx.color = "#ffffff";
    }
    fx.frame = f;
    send(true);
  }
  async function useSaved() {
    if (!saved) return;
    fx = { ...saved.fx, wm: { ...saved.fx.wm } };
    txt = { ...saved.txt };
    src = saved.src;
    if (fx.wm.on && src === "text") await applyText();
    else if (fx.wm.on && src === "logo" && saved.logoPath) {
      try {
        onstate?.(await editWatermark(null, saved.logoPath));
        logoPath = saved.logoPath;
        logoName = saved.logoPath.split(/[\\/]/).pop();
      } catch (e) {
        onerror?.(String(e));
      }
    }
    send(true);
  }
  async function removeAll() {
    fx.frame = "none";
    fx.wm.on = false;
    send(true);
  }
  function done() {
    if (fx.frame !== "none" || fx.wm.on) remember();
    onclose?.();
  }
</script>

<div class="fx">
  <h3>Frame and watermark</h3>
  <p class="hint top">Added last, on top of colour changes. Nothing changes in the file until you save.</p>
  {#if saved}<div class="row1"><button onclick={useSaved} title="The frame and watermark you used last time">Use my last settings</button></div>{/if}

  <div class="body">
    <div class="lab">Frame</div>
    <div class="grid">
      {#each FRAMES as [k, t]}<button class:on={fx.frame === k} onclick={() => setFrame(k)}>{t}</button>{/each}
    </div>
    {#if fx.frame !== "none"}
      <label class="sl"><span class="lab">{SIZE_LABEL[fx.frame] ?? "Width"} <b>{(+fx.size).toFixed(1)} %</b></span>
        <input type="range" min="0.5" max="30" step="0.5" bind:value={fx.size} oninput={() => send()} /></label>
      <div class="lab">{fx.frame === "vignette" ? "Edge colour" : fx.frame === "rounded" ? "Corner colour" : fx.frame === "shadow" ? "Background" : "Colour"}</div>
      <div class="sws">
        {#each COLOURS as c}<button class="sw" class:on={fx.color === c} style="background:{c}" aria-label={c} onclick={() => { fx.color = c; send(true); }}></button>{/each}
        <input type="color" bind:value={fx.color} oninput={() => send()} aria-label="Other colour" />
      </div>
      {#if ACCENT_LABEL[fx.frame]}
        <div class="lab">{ACCENT_LABEL[fx.frame]}</div>
        <div class="sws">
          {#each ["#000000", "#808080", "#ffffff", "#8a6d3b"] as c}<button class="sw" class:on={fx.accent === c} style="background:{c}" aria-label={c} onclick={() => { fx.accent = c; send(true); }}></button>{/each}
          <input type="color" bind:value={fx.accent} oninput={() => send()} aria-label="Other colour" />
        </div>
      {/if}
    {/if}

    <label class="chk big"><input type="checkbox" checked={fx.wm.on} onchange={(e) => toggleWatermark(e.currentTarget.checked)} /> Add a watermark</label>
    {#if fx.wm.on}
      <div class="seg">
        <button class:on={src === "text"} onclick={() => setSource("text")}>Text</button>
        <button class:on={src === "logo"} onclick={() => setSource("logo")}>Logo picture</button>
      </div>
      {#if src === "text"}
        <textarea rows="2" bind:value={txt.text} oninput={textChanged} aria-label="Watermark text"></textarea>
        <div class="seg">
          <button onclick={() => { txt.text = txt.text.includes("©") ? txt.text : `© ${txt.text}`.trim(); textChanged(); }} title="Add the copyright sign">©</button>
          <button class:on={txt.bold} onclick={() => { txt.bold = !txt.bold; textChanged(); }}><b>Bold</b></button>
          <button class:on={txt.outline} onclick={() => { txt.outline = !txt.outline; textChanged(); }} title="A soft halo keeps the text readable">Halo</button>
        </div>
        <div class="sws">
          {#each ["#ffffff", "#000000", "#ffd23f", "#c8102e"] as c}<button class="sw" class:on={txt.color === c} style="background:{c}" aria-label={c} onclick={() => { txt.color = c; textChanged(); }}></button>{/each}
          <input type="color" bind:value={txt.color} oninput={textChanged} aria-label="Text colour" />
        </div>
      {:else}
        <button class="wide" onclick={chooseLogo}>{logoName ? "Choose another picture…" : "Choose a picture…"}</button>
        {#if logoName}<p class="hint">Logo: {logoName}</p>{:else}<p class="hint">A PNG with a transparent background works best.</p>{/if}
      {/if}
      <div class="lab">Position</div>
      <div class="anchor">
        {#each Array(9) as _, n}<button class:on={fx.wm.anchor === n} aria-label="Position {n + 1}" onclick={() => { fx.wm.anchor = n; send(true); }}></button>{/each}
      </div>
      <label class="sl"><span class="lab">Size <b>{Math.round(fx.wm.width)} %</b></span>
        <input type="range" min="2" max="90" step="1" bind:value={fx.wm.width} oninput={() => send()} /></label>
      <label class="sl"><span class="lab">Distance from the edge <b>{(+fx.wm.margin).toFixed(1)} %</b></span>
        <input type="range" min="0" max="20" step="0.5" bind:value={fx.wm.margin} oninput={() => send()} /></label>
      <label class="sl"><span class="lab">Opacity <b>{Math.round(fx.wm.opacity)} %</b></span>
        <input type="range" min="5" max="100" step="1" bind:value={fx.wm.opacity} oninput={() => send()} /></label>
    {/if}
  </div>

  <div class="row2">
    <button onclick={removeAll} disabled={fx.frame === "none" && !fx.wm.on}>Remove both</button>
    <button class="primary" onclick={done}>Done</button>
  </div>
</div>

<style>
  .fx { width: 100%; }
  h3 { margin: 6px 10px 4px; font-size: 14px; }
  .hint { margin: 6px 0 2px; font-size: 12px; color: #9aa1ab; }
  .hint.top { margin: 0 10px 8px; }
  .row1 { margin: 0 8px 8px; display: flex; }
  .row1 button, .wide { flex: 1; width: 100%; padding: 5px 0; }
  .body { padding: 0 10px 8px; }
  .lab { display: flex; justify-content: space-between; font-size: 12.5px; color: #d6dae0; margin: 8px 0 4px; user-select: none; }
  .lab b { font-weight: 500; font-variant-numeric: tabular-nums; color: #fff; }
  .sl { display: block; }
  .sl .lab { margin-top: 6px; }
  input[type="range"] { width: 100%; margin: 2px 0 0; accent-color: #6f9bff; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 4px; }
  .grid button, .seg button { padding: 4px 6px; font-size: 12.5px; white-space: nowrap; }
  .grid button.on, .seg button.on, .anchor button.on { background: #3a64c8; border-color: #3a64c8; color: #fff; }
  .seg { display: flex; gap: 4px; margin: 6px 0; }
  .seg button { flex: 1; }
  .sws { display: flex; align-items: center; gap: 6px; }
  .sw { width: 26px; height: 26px; padding: 0; border-radius: 6px; border: 2px solid #ffffff40; }
  .sw.on { outline: 2px solid #6f9bff; outline-offset: 1px; }
  input[type="color"] { width: 34px; height: 28px; padding: 0; border: none; background: none; }
  .chk { display: flex; align-items: center; gap: 8px; margin: 14px 0 4px; font-size: 13px; }
  .chk.big { font-weight: 600; border-top: 1px solid #ffffff1a; padding-top: 12px; }
  textarea { width: 100%; box-sizing: border-box; resize: vertical; font: inherit; font-size: 13px; padding: 5px 7px;
    background: #ffffff10; color: #fff; border: 1px solid #ffffff30; border-radius: 6px; }
  .anchor { display: grid; grid-template-columns: repeat(3, 26px); gap: 3px; }
  .anchor button { width: 26px; height: 20px; padding: 0; }
  .row2 { display: flex; gap: 6px; margin: 10px 8px 0; }
  .row2 button { flex: 1; }
</style>
