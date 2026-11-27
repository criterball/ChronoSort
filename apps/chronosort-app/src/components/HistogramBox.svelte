<script>
  // Histogram of the shown picture (viewer, Ctrl+H): red, green and blue drawn
  // over each other, brightness as a white outline, plus how much is pure black
  // or pure white (clipped). Worked out by the backend from the shown preview.
  import { imageHistogram } from "../lib/api.js";

  // id: the photo; edit: true while it is being edited; version: changes with every edit
  let { id, edit = false, version = 0, onclose } = $props();

  let h = $state(null);
  let error = $state("");
  let timer;
  $effect(() => {
    const args = [id, edit, version];
    clearTimeout(timer);
    // Edits arrive quickly (sliders): wait until they settle.
    timer = setTimeout(async () => {
      try {
        const r = await imageHistogram(args[0], args[1]);
        if (id === args[0]) { h = r; error = ""; }
      } catch (e) {
        h = null;
        error = "No histogram for this file.";
      }
    }, edit ? 250 : 0);
    return () => clearTimeout(timer);
  });

  const W = 256, H = 100;
  // Square-root scale so small counts still show; the top 0.5 % ignored so one spike does not flatten the rest.
  function path(bins, max) {
    let d = `M0 ${H}`;
    for (let i = 0; i < 256; i++) d += ` L${i} ${(H - Math.min(1, Math.sqrt(bins[i] / max)) * H).toFixed(1)}`;
    return d + ` L${W} ${H} Z`;
  }
  let max = $derived.by(() => {
    if (!h) return 1;
    const all = [...h.r, ...h.g, ...h.b].slice().sort((a, b) => a - b);
    return Math.max(1, all[Math.floor(all.length * 0.995)]);
  });
  const pct = (v) => (v * 100 < 0.1 ? "0" : (v * 100).toFixed(1));
</script>

<div class="histo" role="figure" aria-label="Histogram">
  <div class="top">
    <span>Histogram</span>
    <button onclick={onclose} title="Hide the histogram (Ctrl+H)" aria-label="Hide the histogram">×</button>
  </div>
  {#if h}
    <svg viewBox="0 0 {W} {H}" preserveAspectRatio="none" aria-hidden="true">
      <g style="mix-blend-mode: screen">
        <path d={path(h.r, max)} fill="#ff3b3b" opacity="0.75" />
        <path d={path(h.g, max)} fill="#3bdc5a" opacity="0.75" />
        <path d={path(h.b, max)} fill="#4a7dff" opacity="0.75" />
      </g>
      <path d={path(h.l, max)} fill="none" stroke="#fff" stroke-width="1" vector-effect="non-scaling-stroke" opacity="0.9" />
    </svg>
    <div class="clip">
      <span class:warn={h.dark > 0.02} title="Share of the picture that is pure black in at least one colour">Black {pct(h.dark)} %</span>
      <span class:warn={h.light > 0.02} title="Share of the picture that is pure white in at least one colour (burnt out)">White {pct(h.light)} %</span>
    </div>
  {:else}
    <p>{error || "…"}</p>
  {/if}
</div>

<style>
  .histo { position: absolute; top: 34px; right: 14px; width: 280px; background: rgba(15, 16, 20, 0.82); border: 1px solid #ffffff26;
    border-radius: 10px; padding: 6px 10px 8px; color: #e8eaed; font-size: 12px; z-index: 5; pointer-events: auto; }
  .top { display: flex; justify-content: space-between; align-items: center; margin-bottom: 4px; font-weight: 600; }
  .top button { background: none; border: none; color: #c9ced6; font-size: 16px; line-height: 1; padding: 0 4px; }
  svg { display: block; width: 100%; height: 100px; background: #0008; border-radius: 4px; }
  .clip { display: flex; justify-content: space-between; margin-top: 4px; color: #b9bfc8; }
  .warn { color: #ffb347; font-weight: 600; }
  p { margin: 8px 0; color: #b9bfc8; }
</style>
