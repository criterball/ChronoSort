<script>
  // The retouch tools: red eye, heal, clone, draw. Works with RetouchOverlay
  // (which does the work on the picture) through the shared state `rt`.
  // onapply(op) sends a step; onclose() ends retouching.
  let { rt, onapply, onclose } = $props();

  const TOOLS = [
    ["redeye", "Red eye"],
    ["heal", "Heal"],
    ["clone", "Clone"],
    ["draw", "Draw / text"],
  ];
  const SHAPES = [["pen", "Pen"], ["line", "Line"], ["arrow", "Arrow"], ["rect", "Box"], ["ellipse", "Oval"], ["text", "Text"]];
  const COLOURS = ["#ffffff", "#000000", "#e53935", "#fdd835", "#43a047", "#1e88e5"];

  function clearDrawing() {
    rt.layer?.getContext("2d").clearRect(0, 0, rt.layer.width, rt.layer.height);
    rt.drew = false;
  }
  async function applyDrawing() {
    if (!rt.layer || !rt.drew) return;
    const png = rt.layer.toDataURL("image/png");
    if (await onapply({ kind: "draw", png })) clearDrawing();
  }
  async function done() {
    if (rt.drew) await applyDrawing();
    onclose?.();
  }
</script>

<div class="rt">
  <h3>Retouch</h3>
  <div class="seg wrap">
    {#each TOOLS as [k, t]}<button class:on={rt.tool === k} onclick={() => { rt.tool = k; rt.hint = ""; }}>{t}</button>{/each}
  </div>

  {#if rt.tool === "redeye"}
    <p class="hint">Drag a box around each red eye. Only the red in the pupil changes.</p>
    <label class="fld">Pupil size {rt.pupil}<input type="range" min="0" max="100" bind:value={rt.pupil} /></label>
    <label class="fld">Darken {rt.darken}<input type="range" min="0" max="100" bind:value={rt.darken} /></label>
  {:else if rt.tool === "heal"}
    <p class="hint">Paint over a spot, scratch or speck of dust. When you let go, it is filled from the area around it.</p>
    <label class="fld">Brush size {rt.size}<input type="range" min="3" max="120" bind:value={rt.size} /></label>
  {:else if rt.tool === "clone"}
    <p class="hint">{rt.src ? "Paint to copy from the blue circle. Alt+click (or the button) picks another source." : "First click the place to copy FROM (or Alt+click), then paint where it should go."}</p>
    <button class="wide" class:on={rt.pickSource} onclick={() => (rt.pickSource = true)}>Set the source…</button>
    <label class="fld">Brush size {rt.size}<input type="range" min="3" max="120" bind:value={rt.size} /></label>
    <label class="fld">Hardness {rt.hardness}<input type="range" min="0" max="100" bind:value={rt.hardness} /></label>
  {:else}
    <div class="seg wrap">
      {#each SHAPES as [k, t]}<button class:on={rt.shape === k} onclick={() => (rt.shape = k)}>{t}</button>{/each}
    </div>
    <div class="swatches">
      {#each COLOURS as c}<button class="sw" class:on={rt.color === c} style="background:{c}" aria-label={c} onclick={() => (rt.color = c)}></button>{/each}
      <input type="color" bind:value={rt.color} aria-label="Other colour" />
    </div>
    {#if rt.shape === "text"}
      <label class="fld">Text<textarea rows="2" bind:value={rt.text} placeholder="e.g. Summer 1987"></textarea></label>
      <label class="fld">Size {rt.font}<input type="range" min="10" max="200" bind:value={rt.font} /></label>
      <p class="hint">Then click where the text should start.</p>
    {:else}
      <label class="fld">Line width {rt.width}<input type="range" min="1" max="40" bind:value={rt.width} /></label>
    {/if}
    <div class="row2">
      <button onclick={clearDrawing} disabled={!rt.drew}>Clear</button>
      <button class="primary" onclick={applyDrawing} disabled={!rt.drew || rt.busy}>Put on the picture</button>
    </div>
  {/if}
  {#if rt.hint}<p class="hint warn">{rt.hint}</p>{/if}
  <p class="hint">Each change can be taken back with Ctrl+Z.</p>
  <div class="row2"><button class="primary" onclick={done}>Done</button></div>
</div>

<style>
  .rt { width: 100%; }
  h3 { margin: 6px 10px 8px; font-size: 14px; }
  .seg { display: flex; gap: 4px; margin: 4px 8px 8px; }
  .seg.wrap { flex-wrap: wrap; }
  .seg button { flex: 1 0 auto; padding: 4px 8px; font-size: 12.5px; }
  .seg button.on, .wide.on { background: #3a64c8; border-color: #3a64c8; color: #fff; }
  .fld { display: block; margin: 6px 10px; font-size: 12.5px; }
  .fld input[type="range"] { width: 100%; accent-color: #6f9bff; }
  .fld textarea { width: 100%; box-sizing: border-box; margin-top: 4px; font: inherit; }
  .hint { margin: 6px 10px; font-size: 12px; color: #9aa1ab; }
  .hint.warn { color: #ffc46b; }
  .wide { display: block; width: calc(100% - 20px); margin: 6px 10px; }
  .swatches { display: flex; gap: 6px; align-items: center; margin: 6px 10px; }
  .sw { width: 22px; height: 22px; padding: 0; border-radius: 50%; border: 2px solid #555; }
  .sw.on { border-color: #ffc46b; }
  .swatches input[type="color"] { width: 32px; height: 24px; padding: 0; border: none; background: none; }
  .row2 { display: flex; gap: 6px; margin: 8px; }
  .row2 button { flex: 1; }
</style>
