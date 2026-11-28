<script>
  // Set a photo as the desktop wallpaper, with the same choices as Windows.
  // Ctrl+Z in the browser puts the previous wallpaper back.
  import { onMount, tick } from "svelte";
  import Icon from "./Icon.svelte";
  import { setWallpaper } from "../lib/api.js";

  // item: { id, name }; onclose(message?)
  let { item, onclose } = $props();
  const MEMORY = "chronosort.wallpaper.v1";
  let style = $state((() => { try { return localStorage.getItem(MEMORY) || "fill"; } catch { return "fill"; } })());
  let busy = $state(false);
  let error = $state("");
  let el = $state();
  onMount(() => { tick().then(() => el?.querySelector("button.primary")?.focus()); });

  const STYLES = [
    ["fill", "Fill", "Fills the screen; edges may be cut"],
    ["fit", "Fit", "The whole photo, with bars at the sides"],
    ["stretch", "Stretch", "Fills the screen; the photo is pulled out of shape"],
    ["center", "Centre", "Real size in the middle"],
    ["tile", "Tile", "Repeated like tiles"],
    ["span", "Span", "One photo across all screens"],
  ];
  async function set() {
    busy = true;
    error = "";
    try { localStorage.setItem(MEMORY, style); } catch { /* fine */ }
    try {
      const name = await setWallpaper(item.id, style);
      onclose?.(`“${name}” is now your desktop wallpaper. Ctrl+Z in the browser puts the previous one back.`);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="back wall-dlg" role="presentation" onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); onclose?.(); } }}>
  <div class="dlg" bind:this={el} role="dialog" aria-modal="true" aria-label="Desktop wallpaper">
    <h2><Icon name="wallpaper" size={20} /> Set as desktop wallpaper</h2>
    <p class="muted">{item.name}</p>
    <div class="styles">
      {#each STYLES as [k, t, hint]}
        <label class="st" class:on={style === k} title={hint}>
          <input type="radio" bind:group={style} value={k} />
          <span class="pic {k}"><i></i></span>
          <span><strong>{t}</strong><small>{hint}</small></span>
        </label>
      {/each}
    </div>
    {#if error}<p class="err">{error}</p>{/if}
    <footer>
      <button onclick={() => onclose?.()} disabled={busy}>Cancel</button>
      <button class="primary" onclick={set} disabled={busy} data-continue>Set as wallpaper</button>
    </footer>
  </div>
</div>

<style>
  .back { position: fixed; inset: 0; z-index: 1100; background: rgba(0, 0, 0, 0.55); display: grid; place-items: center; }
  .dlg { width: min(560px, calc(100vw - 32px)); background: var(--surface); color: var(--text); border: 1px solid var(--border);
    border-radius: 14px; padding: 16px 20px; box-shadow: 0 20px 60px #0008; }
  h2 { margin: 0 0 2px; font-size: 20px; display: flex; align-items: center; gap: 8px; }
  .muted { color: var(--muted); margin: 0 0 10px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .styles { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .st { display: flex; align-items: center; gap: 10px; padding: 8px; border: 1px solid var(--border); border-radius: 10px; cursor: pointer; }
  .st.on { border-color: var(--accent); background: var(--accent-soft); }
  .st input { position: absolute; opacity: 0; pointer-events: none; }
  .st strong { display: block; }
  .st small { color: var(--muted); font-size: 12px; }
  /* Tiny screen pictures showing each style. */
  .pic { flex: none; width: 54px; height: 34px; border: 2px solid var(--muted); border-radius: 3px; background: #223; position: relative; overflow: hidden; }
  .pic i { position: absolute; background: linear-gradient(135deg, #f0b35a, #5a8ff0); }
  .pic.fill i { inset: -6px 0; }
  .pic.fit i { top: 0; bottom: 0; left: 14px; right: 14px; }
  .pic.stretch i { inset: 0; }
  .pic.center i { width: 18px; height: 14px; left: 16px; top: 8px; }
  .pic.tile { background: repeating-linear-gradient(90deg, #f0b35a 0 12px, #5a8ff0 12px 14px); }
  .pic.tile i { display: none; }
  .pic.span i { inset: 0 -20px 0 0; }
  .err { color: var(--red); }
  footer { display: flex; justify-content: flex-end; gap: 8px; margin-top: 14px; }
  footer button { padding: 8px 16px; }
</style>
