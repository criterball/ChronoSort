<script>
  import { onMount, tick } from "svelte";
  import Icon from "./Icon.svelte";
  import { fthumbUrl, imageSize, revealItem, formatBytes } from "../lib/api.js";

  // items: 2–4 pictures { id (fthumb id), name, folder?, size, modified }.
  // onclose(): back. onpick(i) (optional): "Keep this one" (Find duplicates), then closes.
  let { items, onclose, onpick = null } = $props();

  // One view per picture. `z` is relative to "fit" (1 = whole picture fits its pane) and
  // (cx, cy) is the point of the picture in the middle of the pane, 0..1 of its width/height.
  // So "together" shows the SAME part of every version, even when one is a smaller copy.
  // svelte-ignore state_referenced_locally
  let views = $state(items.map(() => ({ z: 1, cx: 0.5, cy: 0.5 })));
  // svelte-ignore state_referenced_locally
  let panes = $state(items.map(() => ({ w: 0, h: 0 })));
  // svelte-ignore state_referenced_locally
  let nat = $state(items.map(() => ({ w: 0, h: 0 }))); // the loaded image's pixels (aspect)
  // svelte-ignore state_referenced_locally
  let full = $state(items.map(() => null)); // { w, h, original } from the file header
  // svelte-ignore state_referenced_locally
  let hi = $state(items.map(() => false)); // showing the original instead of the preview
  // svelte-ignore state_referenced_locally
  let failed = $state(items.map(() => false));
  let together = $state(true);
  let active = $state(0); // the pane under the pointer (keys 1 / + / − act on it)
  let el = $state();
  let dpr = $state(window.devicePixelRatio || 1);

  onMount(() => {
    tick().then(() => el?.focus());
    items.forEach((it, i) => imageSize(it.id).then((r) => { full[i] = r; }).catch(() => {}));
    const upd = () => (dpr = window.devicePixelRatio || 1);
    window.addEventListener("resize", upd);
    return () => window.removeEventListener("resize", upd);
  });

  // The real picture's size, upright like the shown image (an EXIF turn swaps the stored size).
  function real(i) {
    const f = full[i], n = nat[i];
    if (!f || !n.w) return null;
    return n.w > n.h !== f.w > f.h && n.w !== n.h ? { w: f.h, h: f.w } : { w: f.w, h: f.h };
  }
  const fitW = (i) => (nat[i].w && panes[i].w ? Math.min(panes[i].w / nat[i].w, panes[i].h / nat[i].h) * nat[i].w : 0);
  const dispW = (i) => fitW(i) * views[i].z;
  const dispH = (i) => (nat[i].w ? (dispW(i) * nat[i].h) / nat[i].w : 0);
  // The middle point, kept so the picture never leaves an empty edge when it is larger than the pane.
  function eff(c, disp, pane) {
    if (disp <= pane) return 0.5;
    const m = pane / 2 / disp;
    return Math.min(1 - m, Math.max(m, c));
  }
  function box(i) {
    const w = dispW(i), h = dispH(i), p = panes[i];
    const cx = eff(views[i].cx, w, p.w), cy = eff(views[i].cy, h, p.h);
    return { left: p.w / 2 - cx * w, top: p.h / 2 - cy * h, w, h, cx, cy };
  }
  // Percent of the REAL picture (one real pixel per screen pixel = 100 %).
  function pct(i) {
    const r = real(i);
    return r && dispW(i) ? Math.round((100 * dispW(i) * dpr) / r.w) : null;
  }

  // Load the original once the preview's own pixels are not enough.
  $effect(() => {
    items.forEach((_, i) => {
      const r = real(i);
      if (!hi[i] && full[i]?.original && r && r.w > nat[i].w * 1.01 && dispW(i) * dpr > nat[i].w * 1.01) hi[i] = true;
    });
  });

  const targets = (i) => (together ? items.map((_, k) => k) : [i]);
  function setView(i, v) {
    for (const k of targets(i)) views[k] = { ...v };
  }
  const clampZ = (z) => Math.min(80, Math.max(0.25, z));

  // Zoom around a point of pane i (pane pixels; default: its middle).
  function zoomAt(i, z, mx = panes[i].w / 2, my = panes[i].h / 2) {
    z = clampZ(z);
    const b = box(i);
    if (!b.w) return;
    const u = b.cx + (mx - panes[i].w / 2) / b.w, v = b.cy + (my - panes[i].h / 2) / b.h;
    const w2 = fitW(i) * z, h2 = (w2 * nat[i].h) / nat[i].w;
    setView(i, { z, cx: u - (mx - panes[i].w / 2) / w2, cy: v - (my - panes[i].h / 2) / h2 });
  }
  const zoomBy = (i, f) => zoomAt(i, views[i].z * f);
  const fitAll = () => (views = items.map(() => ({ z: 1, cx: 0.5, cy: 0.5 })));
  // 100 % = real pixels of pane i (the others follow with the same view when "together").
  function actual(i, mx, my) {
    const r = real(i);
    const f = fitW(i);
    if (!f) return;
    zoomAt(i, r ? r.w / dpr / f : nat[i].w / f, mx, my);
  }

  function onWheel(ev, i) {
    ev.preventDefault();
    const r = ev.currentTarget.getBoundingClientRect();
    zoomAt(i, views[i].z * (ev.deltaY < 0 ? 1.2 : 1 / 1.2), ev.clientX - r.left, ev.clientY - r.top);
  }
  function onDown(ev, i) {
    if (ev.button !== 0) return;
    const b = box(i);
    if (b.w <= panes[i].w && b.h <= panes[i].h) return; // nothing to move
    ev.preventDefault();
    const x0 = ev.clientX, y0 = ev.clientY, start = { ...views[i], cx: b.cx, cy: b.cy };
    const t = ev.currentTarget;
    t.setPointerCapture(ev.pointerId);
    const mv = (m) => {
      const cx = eff(start.cx - (m.clientX - x0) / b.w, b.w, panes[i].w);
      const cy = eff(start.cy - (m.clientY - y0) / b.h, b.h, panes[i].h);
      setView(i, { z: start.z, cx, cy });
    };
    const up = () => { t.removeEventListener("pointermove", mv); t.removeEventListener("pointerup", up); };
    t.addEventListener("pointermove", mv);
    t.addEventListener("pointerup", up);
  }
  function onDbl(ev, i) {
    const r = ev.currentTarget.getBoundingClientRect();
    if (views[i].z > 1.01) setView(i, { z: 1, cx: 0.5, cy: 0.5 });
    else actual(i, ev.clientX - r.left, ev.clientY - r.top);
  }
  function setTogether(on) {
    together = on;
    if (on) views = items.map(() => ({ ...views[active] })); // all follow the one under the pointer
  }

  function onKey(e) {
    const k = e.key;
    if (k === "Escape") onclose();
    else if (k === "0" || k === "b" || k === "B") fitAll();
    else if (k === "1" || k === "a" || k === "A") actual(active);
    else if (k === "+" || k === "=") zoomBy(active, 1.25);
    else if (k === "-") zoomBy(active, 0.8);
    else if (k === "s" || k === "S") setTogether(!together);
    else if (k === "Tab") active = (active + (e.shiftKey ? items.length - 1 : 1)) % items.length;
    else return;
    e.preventDefault();
  }

  function loaded(e, i) {
    const nw = e.currentTarget.naturalWidth, nh = e.currentTarget.naturalHeight;
    nat[i] = { w: nw, h: nh }; // same aspect for preview and original, so the view stays put
  }

  // Which version has the most pixels / the biggest file (a hint, like in Find duplicates).
  let most = $derived.by(() => {
    const px = items.map((_, i) => { const r = real(i); return r ? r.w * r.h : 0; });
    const best = Math.max(...px);
    return best > 0 && px.filter((p) => p === best).length === 1 ? px.indexOf(best) : -1;
  });
  let biggest = $derived.by(() => {
    const s = items.map((it) => it.size ?? 0);
    const best = Math.max(...s);
    return best > 0 && s.filter((x) => x === best).length === 1 ? s.indexOf(best) : -1;
  });
  function info(i) {
    const r = real(i), it = items[i];
    return [r ? `${r.w} × ${r.h}` : "", formatBytes(it.size ?? 0), it.modified ? `changed ${fmtDate(it.modified)}` : ""].filter(Boolean).join(" · ");
  }
  const fmtDate = (s) => (s ? new Date(s * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "");
  let cols = $derived(items.length === 4 ? 2 : items.length);
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="compare" bind:this={el} tabindex="0" role="dialog" aria-label="Compare pictures" onkeydown={onKey}>
  <header>
    <strong>Compare {items.length} pictures</strong>
    <span class="grp">
      <button class="ib" title="Zoom out (−)" onclick={() => zoomBy(active, 0.8)}><Icon name="zoom-out" /></button>
      <button class="ib" title="Zoom in (+)" onclick={() => zoomBy(active, 1.25)}><Icon name="zoom-in" /></button>
      <button onclick={fitAll} title="Whole pictures (0)">Fit</button>
      <button onclick={() => actual(active)} title="Real pixels of the highlighted picture (1)">100 %</button>
    </span>
    <label class="check" title="Zoom and move all pictures together (S)">
      <input type="checkbox" checked={together} onchange={(e) => setTogether(e.currentTarget.checked)} /> Zoom and move together
    </label>
    <span class="hint muted">Wheel = zoom at the pointer · drag = move · double-click = 100 % / fit</span>
    <span class="spacer"></span>
    <button class="close" onclick={onclose} title="Back (Esc)">Close <kbd>Esc</kbd></button>
  </header>

  <div class="grid" style="grid-template-columns: repeat({cols}, minmax(0, 1fr))">
    {#each items as it, i (it.id)}
      {@const b = box(i)}
      <section class="cell" class:act={active === i} onpointerenter={() => (active = i)} role="group" aria-label="Picture {i + 1}">
        <div class="pane" bind:clientWidth={panes[i].w} bind:clientHeight={panes[i].h}
          class:grab={b.w > panes[i].w || b.h > panes[i].h}
          onwheel={(e) => onWheel(e, i)} onpointerdown={(e) => onDown(e, i)} ondblclick={(e) => onDbl(e, i)} role="img" aria-label={it.name}>
          {#if failed[i]}
            <div class="nopic muted">No preview for this file</div>
          {:else}
            <img src={fthumbUrl(it.id, hi[i] ? "o" : "p")} alt="" draggable="false" onload={(e) => loaded(e, i)}
              onerror={() => { if (hi[i]) hi[i] = false; else failed[i] = true; }}
              style="left:{b.left}px; top:{b.top}px; width:{b.w}px; height:{b.h}px; visibility:{nat[i].w ? 'visible' : 'hidden'}" />
          {/if}
          {#if pct(i) != null}<span class="pct">{pct(i)} %</span>{/if}
        </div>
        <div class="cap">
          <div class="l1">
            <kbd>{i + 1}</kbd>
            <strong class="nm" title={it.name}>{it.name}</strong>
            <span class="spacer"></span>
            <button class="small" onclick={() => revealItem(it.id).catch(() => {})}>Show in Explorer</button>
            {#if onpick}<button class="small primary" onclick={() => onpick(i)}>Keep this one</button>{/if}
          </div>
          <div class="l2 muted">
            {#if most === i}<span class="chip green">most pixels</span>{/if}
            {#if biggest === i}<span class="chip accent">biggest file</span>{/if}
            {info(i)}
          </div>
          {#if it.folder}<div class="l2 mono dir" title={it.folder}><span dir="ltr">{it.folder}</span></div>{/if}
        </div>
      </section>
    {/each}
  </div>
</div>

<style>
  .compare { position: fixed; inset: 0; z-index: 60; display: flex; flex-direction: column; background: #111; color: #e8e8e8; outline: none; }
  header { display: flex; align-items: center; gap: 12px; padding: 8px 12px; background: #1c1c1e; border-bottom: 1px solid #333; flex-wrap: wrap; }
  header button { background: #2c2c30; color: #eee; border: 1px solid #444; }
  header button:hover { background: #3a3a40; }
  .grp { display: flex; gap: 4px; align-items: center; }
  .ib { display: grid; place-items: center; width: 34px; height: 30px; padding: 0; }
  .check { display: flex; align-items: center; gap: 6px; cursor: pointer; }
  .hint { font-size: 0.85em; color: #9a9a9f; }
  .spacer { flex: 1; }
  .close kbd { margin-left: 4px; }
  .grid { flex: 1; min-height: 0; display: grid; grid-auto-rows: minmax(0, 1fr); gap: 4px; padding: 4px; }
  .cell { display: flex; flex-direction: column; min-height: 0; min-width: 0; border: 2px solid transparent; border-radius: 6px; background: #000; }
  .cell.act { border-color: var(--accent); }
  .pane { position: relative; flex: 1; min-height: 0; overflow: hidden; touch-action: none; }
  .pane.grab { cursor: grab; }
  .pane.grab:active { cursor: grabbing; }
  .pane img { position: absolute; max-width: none; user-select: none; image-rendering: auto; }
  .nopic { display: grid; place-items: center; height: 100%; color: #999; }
  .pct { position: absolute; right: 8px; top: 8px; padding: 2px 8px; border-radius: 10px; background: rgba(0, 0, 0, 0.6); color: #fff; font-size: 0.85em; pointer-events: none; }
  .cap { padding: 6px 10px 8px; background: #1c1c1e; display: flex; flex-direction: column; gap: 2px; }
  .l1 { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .l1 button { flex: none; }
  .nm { flex: 0 1 auto; min-width: 4em; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .l2 { height: 22px; line-height: 22px; }
  .l2 .chip { margin-right: 6px; padding-top: 0; padding-bottom: 0; line-height: 18px; display: inline-block; }
  .l2 { font-size: 0.85em; color: #a8a8ad; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .dir { direction: rtl; text-align: left; }
  .l1 button { background: #2c2c30; color: #eee; border: 1px solid #444; }
  .l1 button.primary { background: var(--accent); color: var(--accent-text); border-color: var(--accent); }
  kbd { background: #333; color: #ddd; border: 1px solid #555; border-radius: 4px; padding: 0 5px; font-size: 0.8em; }
</style>
