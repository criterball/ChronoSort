<script>
  // Colour and light adjustments for the picture being edited. Every change is
  // sent to the backend (debounced, one request at a time), which renders the
  // preview; nothing is written to the file until Save.
  import { untrack } from "svelte";
  import { editAdjust, editAuto, editControls } from "../lib/api.js";

  // start: the adjustments when the panel opened; onstate(editState) after each update;
  // onclose(); onerror(message)
  let { start, onstate, onclose, onerror } = $props();

  const NEUTRAL = {
    exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0,
    temp: 0, tint: 0, vibrance: 0, saturation: 0, texture: 0, clarity: 0, dehaze: 0,
    inBlack: 0, inWhite: 1, gamma: 1, outBlack: 0, outWhite: 1,
    curveShadows: 0, curveDarks: 0, curveLights: 0, curveHighlights: 0, points: [],
    balanceShadows: [0, 0, 0], balanceMidtones: [0, 0, 0], balanceHighlights: [0, 0, 0],
    sharpenAmount: 0, sharpenRadius: 1, sharpenDetail: 25, sharpenMasking: 0,
    nrLuminance: 0, nrColor: 0, blur: 0, look: "none",
  };
  const copy = (a) => JSON.parse(JSON.stringify(a));
  const initial = untrack(() => copy({ ...NEUTRAL, ...(start ?? {}) }));
  let adj = $state(copy(initial));
  let controls = $state([]);
  let open = $state({ Light: true, Colour: true, Presence: false, Curve: false, Balance: false, Detail: false, Look: false });
  let range = $state("balanceMidtones");
  let comparing = $state(false);

  editControls().then((c) => (controls = c)).catch((e) => onerror?.(String(e)));
  const inSection = (s) => controls.filter((c) => c.section === s);

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
      onstate?.(await editAdjust(comparing ? NEUTRAL : $state.snapshot(adj)));
    } catch (e) {
      onerror?.(String(e));
    } finally {
      sending = false;
      if (again) { again = false; flush(); }
    }
  }
  function set(key, v) {
    adj[key] = v;
    send();
  }

  async function auto() {
    try {
      const a = await editAuto();
      // Auto sets tone and colour; detail, looks and the curve stay as the user set them.
      for (const k of ["inBlack", "inWhite", "gamma", "temp", "tint", "vibrance"]) adj[k] = a[k];
      open.Colour = open.Curve = true;
      send(true);
    } catch (e) {
      onerror?.(String(e));
    }
  }
  function reset() {
    adj = copy(NEUTRAL);
    send(true);
  }
  function cancel() {
    adj = copy(initial);
    flush().then(() => onclose?.());
  }
  function compare(on) {
    if (comparing === on) return;
    comparing = on;
    send(true);
  }

  const fmt = (c, v) => (c.step < 1 ? (+v).toFixed(c.step < 0.1 ? 2 : 1) : Math.round(v)) + "";
  const changed = $derived(JSON.stringify($state.snapshot(adj)) !== JSON.stringify(NEUTRAL));
  // Levels shown as 0..255 like every photo program; stored as 0..1.
  const LEVELS = [
    ["inBlack", "Input black", 0, 242], ["inWhite", "Input white", 13, 255],
    ["outBlack", "Output black", 0, 255], ["outWhite", "Output white", 0, 255],
  ];
  const BAL = [["Cyan", "Red"], ["Magenta", "Green"], ["Yellow", "Blue"]];
</script>

{#snippet slider(c)}
  <label class="sl" title="Double-click to reset">
    <span class="lab" role="presentation" ondblclick={() => set(c.key, c.default)}>{c.label}<b>{fmt(c, adj[c.key])}</b></span>
    <input type="range" min={c.min} max={c.max} step={c.step} value={adj[c.key]}
      class:centred={c.min < 0} oninput={(e) => set(c.key, +e.currentTarget.value)}
      ondblclick={() => set(c.key, c.default)} />
  </label>
{/snippet}

{#snippet section(id, title, body)}
  <button class="sec" aria-expanded={open[id]} onclick={() => (open[id] = !open[id])}>
    <span class="tw">{open[id] ? "▾" : "▸"}</span>{title}
  </button>
  {#if open[id]}<div class="body">{@render body()}</div>{/if}
{/snippet}

<div class="adjust">
  <h3>Colour and light</h3>
  <div class="top">
    <button onclick={auto} title="Let ChronoSort set brightness, contrast and colour; you can change it after">Auto</button>
    <button onclick={reset} disabled={!changed}>Reset</button>
    <button class="cmp" class:on={comparing} onpointerdown={() => compare(true)} onpointerup={() => compare(false)}
      onpointerleave={() => compare(false)} title="Hold to see the picture without these adjustments">Before</button>
  </div>

  {#snippet light()}{#each inSection("Light") as c (c.key)}{@render slider(c)}{/each}{/snippet}
  {@render section("Light", "Light", light)}

  {#snippet colour()}{#each inSection("Colour") as c (c.key)}{@render slider(c)}{/each}{/snippet}
  {@render section("Colour", "Colour", colour)}

  {#snippet presence()}{#each inSection("Presence") as c (c.key)}{@render slider(c)}{/each}{/snippet}
  {@render section("Presence", "Clarity and haze", presence)}

  {#snippet curve()}
    <p class="hint">Tone curve by region</p>
    {#each inSection("Curve") as c (c.key)}{@render slider(c)}{/each}
    <p class="hint">Levels</p>
    {#each LEVELS as [k, label, lo, hi]}
      <label class="sl"><span class="lab" role="presentation" ondblclick={() => set(k, NEUTRAL[k])}>{label}<b>{Math.round(adj[k] * 255)}</b></span>
        <input type="range" min={lo} max={hi} step="1" value={Math.round(adj[k] * 255)} oninput={(e) => set(k, +e.currentTarget.value / 255)} /></label>
    {/each}
    <label class="sl"><span class="lab" role="presentation" ondblclick={() => set("gamma", 1)}>Midtones (gamma)<b>{(+adj.gamma).toFixed(2)}</b></span>
      <input type="range" min="-1" max="1" step="0.01" value={Math.log10(adj.gamma)} class="centred"
        oninput={(e) => set("gamma", Math.round(10 ** +e.currentTarget.value * 100) / 100)} /></label>
  {/snippet}
  {@render section("Curve", "Curve and levels", curve)}

  {#snippet balance()}
    <div class="seg">
      {#each [["balanceShadows", "Shadows"], ["balanceMidtones", "Midtones"], ["balanceHighlights", "Highlights"]] as [k, t]}
        <button class:on={range === k} onclick={() => (range = k)}>{t}</button>
      {/each}
    </div>
    {#each BAL as [l, r], n}
      <label class="sl"><span class="lab bal" role="presentation" ondblclick={() => { adj[range][n] = 0; send(); }}><span>{l}</span><b>{Math.round(adj[range][n])}</b><span>{r}</span></span>
        <input type="range" min="-100" max="100" step="1" class="centred" value={adj[range][n]}
          oninput={(e) => { adj[range][n] = +e.currentTarget.value; send(); }} /></label>
    {/each}
  {/snippet}
  {@render section("Balance", "Colour balance", balance)}

  {#snippet detail()}{#each inSection("Detail") as c (c.key)}{@render slider(c)}{/each}
    <p class="hint">Sharpening is best judged at 100 % (key 1).</p>{/snippet}
  {@render section("Detail", "Sharpen, noise and blur", detail)}

  {#snippet look()}
    <div class="seg wrap">
      {#each [["none", "Colour"], ["grayscale", "Black & white"], ["sepia", "Sepia"], ["negative", "Negative"]] as [k, t]}
        <button class:on={adj.look === k} onclick={() => set("look", k)}>{t}</button>
      {/each}
    </div>
  {/snippet}
  {@render section("Look", "Black & white, sepia, negative", look)}

  <div class="row2">
    <button onclick={cancel}>Cancel</button>
    <button class="primary" onclick={() => onclose?.()}>Done</button>
  </div>
</div>

<style>
  .adjust { width: 100%; }
  h3 { margin: 6px 10px 8px; font-size: 14px; }
  .top { display: flex; gap: 6px; margin: 0 8px 8px; }
  .top button { flex: 1; padding: 5px 0; }
  .cmp.on { background: #3a64c8; }
  .sec { display: flex; align-items: center; gap: 6px; width: 100%; background: none; border: none; border-top: 1px solid #ffffff1a;
    padding: 7px 10px; font-weight: 600; text-align: left; border-radius: 0; }
  .sec:hover { background: #ffffff10; }
  .tw { width: 12px; color: #9aa1ab; }
  .body { padding: 0 10px 8px; }
  .sl { display: block; margin: 4px 0 6px; }
  .lab { display: flex; justify-content: space-between; font-size: 12.5px; color: #d6dae0; cursor: default; user-select: none; }
  .lab b { font-weight: 500; font-variant-numeric: tabular-nums; color: #fff; }
  .lab.bal span { color: #9aa1ab; width: 60px; }
  .lab.bal span:last-child { text-align: right; }
  input[type="range"] { width: 100%; margin: 2px 0 0; accent-color: #6f9bff; }
  .hint { margin: 8px 0 2px; font-size: 12px; color: #9aa1ab; }
  .seg { display: flex; gap: 4px; margin: 4px 0 8px; }
  .seg.wrap { flex-wrap: wrap; }
  .seg button { flex: 1; padding: 4px 6px; font-size: 12.5px; white-space: nowrap; }
  .seg button.on { background: #3a64c8; border-color: #3a64c8; color: #fff; }
  .row2 { display: flex; gap: 6px; margin: 10px 8px 0; }
  .row2 button { flex: 1; }
</style>
