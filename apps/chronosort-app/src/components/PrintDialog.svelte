<script>
  // Print photos: choose a layout here, then the normal Windows print dialog
  // opens (printer, copies, paper). The pages are built in this window, so
  // nothing is sent anywhere but to the printer.
  import { onMount, tick } from "svelte";
  import Icon from "./Icon.svelte";
  import { fthumbUrl } from "../lib/api.js";

  // items: entries { id, name, kind, modified }; onclose()
  let { items, onclose } = $props();

  const MEMORY = "chronosort.print.v1";
  const DEF = { layout: "one", paper: "a4", size: "10x15", fit: "fit", caption: "none", margin: 10 };
  function load() {
    try { return { ...DEF, ...JSON.parse(localStorage.getItem(MEMORY) ?? "{}") }; } catch { return { ...DEF }; }
  }
  let o = $state(load());
  const save = () => { try { localStorage.setItem(MEMORY, JSON.stringify($state.snapshot(o))); } catch { /* fine */ } };

  const photos = $derived(items.filter((e) => e.kind === "image" || e.kind === "raw" || e.kind === "video"));
  const PAPER = { a4: [210, 297], letter: [215.9, 279.4] };
  const SIZES = { "9x13": [9, 13], "10x15": [10, 15], "13x18": [13, 18], "15x20": [15, 20], "20x25": [20, 25], "20x30": [20, 30] };

  let el = $state();
  let printing = $state(false);
  let pages = $state([]); // [{ kind, photos: [{ e, land }] }]
  let ratios = {}; // id → width / height, filled while loading
  let status = $state("");

  onMount(() => { tick().then(() => el?.querySelector("button.primary")?.focus()); });

  const dateOf = (e) => (e.modified ? new Date(e.modified * 1000).toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" }) : "");
  const srcOf = (e) => fthumbUrl(e.id, e.kind === "image" ? "o" : "p");

  // Load every picture first (its shape decides portrait or landscape pages), then lay out and print.
  async function print() {
    save();
    status = "Getting the pictures ready…";
    await Promise.all(photos.map((e) => new Promise((res) => {
      const im = new Image();
      im.onload = () => { ratios[e.id] = im.naturalWidth / Math.max(1, im.naturalHeight); res(); };
      im.onerror = () => {
        if (!im.dataset.fb) { im.dataset.fb = "1"; im.src = fthumbUrl(e.id, "p"); } else { ratios[e.id] = 1.5; res(); }
      };
      im.src = srcOf(e);
    })));
    const per = { one: 1, two: 2, four: 4 }[o.layout];
    const list = photos.map((e) => ({ e, land: (ratios[e.id] ?? 1) > 1 }));
    if (per) {
      pages = [];
      for (let i = 0; i < list.length; i += per) pages.push({ kind: o.layout, photos: list.slice(i, i + per) });
    } else {
      pages = [{ kind: "size", photos: list }];
    }
    printing = true;
    await tick();
    const imgs = [...document.querySelectorAll(".print-portal img")];
    await Promise.all(imgs.map((i) => (i.complete ? Promise.resolve() : new Promise((r) => { i.onload = i.onerror = r; }))));
    status = "";
    // The page stays until the print dialog is finished with it.
    window.addEventListener("afterprint", () => { printing = false; onclose?.(); }, { once: true });
    window.print();
  }

  // Page boxes in millimetres for the chosen paper (portrait or landscape).
  function box(land) {
    let [w, h] = PAPER[o.paper];
    if (land) [w, h] = [h, w];
    const m = +o.margin;
    return { w: w - 2 * m, h: h - 2 * m };
  }
  const capH = $derived(o.caption === "none" ? 0 : 8);
  function exact(e) {
    const [a, b] = SIZES[o.size];
    const land = (ratios[e.id] ?? 1) > 1;
    return land ? [b * 10, a * 10] : [a * 10, b * 10]; // mm, turned to the photo
  }
  const pageCss = $derived(() => {
    const [w, h] = PAPER[o.paper];
    return `@page { size: ${w}mm ${h}mm; margin: ${o.margin}mm; }
      @page land { size: ${h}mm ${w}mm; margin: ${o.margin}mm; }
      @media screen { .print-portal { display: none !important; } }
      @media print {
        body > *:not(.print-portal) { display: none !important; }
        html, body { background: #fff !important; height: auto !important; overflow: visible !important; }
        .print-portal { display: block !important; }
      }`;
  });
  function portal(node) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }
</script>

<svelte:head>{@html `<style id="cs-print-css">${pageCss()}</style>`}</svelte:head>

<div class="back print-dlg" role="presentation" onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); onclose?.(); } }}>
  <div class="dlg" bind:this={el} role="dialog" aria-modal="true" aria-label="Print">
    <header>
      <h2><Icon name="print" size={20} /> Print {photos.length} {photos.length === 1 ? "photo" : "photos"}</h2>
      <button class="x" onclick={() => onclose?.()} aria-label="Close"><Icon name="close" /></button>
    </header>

    <div class="lab">How many on a page</div>
    <div class="seg">
      {#each [["one", "One, as big as possible"], ["two", "Two"], ["four", "Four"], ["size", "Exact photo size"]] as [k, t]}
        <button class:on={o.layout === k} onclick={() => (o.layout = k)}>{t}</button>
      {/each}
    </div>
    {#if o.layout === "size"}
      <div class="seg">
        {#each Object.keys(SIZES) as s}<button class:on={o.size === s} onclick={() => (o.size = s)}>{s.replace("x", " × ")} cm</button>{/each}
      </div>
      <p class="hint">As many as fit go on each page; cut them out along the edges.</p>
    {:else if o.layout === "one"}
      <p class="hint">Wide photos get a sideways page, tall ones an upright page.</p>
    {/if}

    <div class="grid">
      <label class="fld">Paper<select bind:value={o.paper}><option value="a4">A4</option><option value="letter">Letter</option></select></label>
      <label class="fld">White edge
        <select bind:value={o.margin}><option value={5}>Small (5 mm)</option><option value={10}>Normal (1 cm)</option><option value={20}>Wide (2 cm)</option></select></label>
      <label class="fld">Photo in its space
        <select bind:value={o.fit}><option value="fit">Whole photo (may leave white space)</option><option value="fill">Fill the space (edges may be cut)</option></select></label>
      <label class="fld">Text under each photo
        <select bind:value={o.caption}><option value="none">None</option><option value="name">File name</option><option value="date">Name and date</option></select></label>
    </div>
    <p class="hint">Next, the Windows print window opens: choose the printer, the number of copies and print quality there. Nothing leaves this PC except to your printer.</p>
    {#if status}<p class="hint"><span class="spin"></span> {status}</p>{/if}

    <footer>
      <button onclick={() => onclose?.()}>Cancel</button>
      <button class="primary" disabled={!photos.length || !!status} onclick={print} data-continue><Icon name="print" size={16} /> Print…</button>
    </footer>
  </div>
</div>

{#if printing}
  <div class="print-portal" use:portal>
    {#each pages as pg, pi (pi)}
      {#if pg.kind === "size"}
        <div class="flow">
          {#each pg.photos as p (p.e.id)}
            {@const [w, h] = exact(p.e)}
            <figure class="exact" style="width:{w}mm">
              <div class="frame" style="width:{w}mm; height:{h}mm"><img src={srcOf(p.e)} alt="" class:fill={o.fit === "fill"} onerror={(x) => { if (!x.currentTarget.dataset.fb) { x.currentTarget.dataset.fb = "1"; x.currentTarget.src = fthumbUrl(p.e.id, "p"); } }} /></div>
              {#if o.caption !== "none"}<figcaption>{p.e.name}{#if o.caption === "date"}&nbsp;·&nbsp;{dateOf(p.e)}{/if}</figcaption>{/if}
            </figure>
          {/each}
        </div>
      {:else}
        {@const land = pg.kind === "one" && pg.photos[0]?.land}
        {@const b = box(land)}
        {@const n = pg.photos.length}
        <section class="page {pg.kind}" class:land style="width:{b.w}mm; height:{b.h - 1}mm">
          {#each pg.photos as p (p.e.id)}
            <figure>
              <div class="frame"><img src={srcOf(p.e)} alt="" class:fill={o.fit === "fill"} onerror={(x) => { if (!x.currentTarget.dataset.fb) { x.currentTarget.dataset.fb = "1"; x.currentTarget.src = fthumbUrl(p.e.id, "p"); } }} /></div>
              {#if o.caption !== "none"}<figcaption style="height:{capH}mm">{p.e.name}{#if o.caption === "date"}&nbsp;·&nbsp;{dateOf(p.e)}{/if}</figcaption>{/if}
            </figure>
          {/each}
          {#if n < { one: 1, two: 2, four: 4 }[pg.kind]}{#each Array({ one: 1, two: 2, four: 4 }[pg.kind] - n) as _}<figure class="empty"></figure>{/each}{/if}
        </section>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .back { position: fixed; inset: 0; z-index: 1100; background: rgba(0, 0, 0, 0.55); display: grid; place-items: center; }
  .dlg { width: min(640px, calc(100vw - 32px)); max-height: calc(100vh - 32px); overflow: auto; background: var(--surface);
    color: var(--text); border: 1px solid var(--border); border-radius: 14px; padding: 16px 20px; box-shadow: 0 20px 60px #0008; }
  header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 6px; }
  h2 { margin: 0; font-size: 20px; display: flex; align-items: center; gap: 8px; }
  .x { background: none; border: none; padding: 4px; }
  .lab { font-weight: 600; margin: 10px 0 6px; }
  .seg { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 6px; }
  .seg button { flex: 1; padding: 6px 8px; white-space: nowrap; }
  .seg button.on { background: var(--accent); border-color: var(--accent); color: var(--accent-text); }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 10px 14px; margin-top: 10px; }
  .fld { display: flex; flex-direction: column; gap: 4px; font-size: 13px; }
  .hint { color: var(--muted); font-size: 12.5px; margin: 6px 0; }
  footer { display: flex; justify-content: flex-end; gap: 8px; margin-top: 14px; }
  footer button { padding: 8px 16px; }
  .spin { display: inline-block; width: 10px; height: 10px; border: 2px solid var(--muted); border-top-color: transparent; border-radius: 50%; animation: spin 0.8s linear infinite; vertical-align: -1px; }
  @keyframes spin { to { transform: rotate(360deg); } }

  /* ---- the printed pages ---- */
  .print-portal { color: #000; background: #fff; font: 10pt "Segoe UI", system-ui, sans-serif; }
  .page { display: grid; gap: 6mm; break-after: page; page-break-after: always; overflow: hidden; }
  .page:last-child { break-after: auto; page-break-after: auto; }
  .page.land { page: land; }
  .page.one { grid-template: 1fr / 1fr; }
  .page.two { grid-template: 1fr 1fr / 1fr; }
  .page.four { grid-template: 1fr 1fr / 1fr 1fr; }
  .page figure { margin: 0; display: flex; flex-direction: column; min-height: 0; min-width: 0; }
  .page .frame { flex: 1; min-height: 0; display: flex; align-items: center; justify-content: center; overflow: hidden; }
  .frame img { max-width: 100%; max-height: 100%; object-fit: contain; display: block; }
  .page .frame img { width: 100%; height: 100%; }
  .frame img.fill { width: 100%; height: 100%; object-fit: cover; }
  figcaption { text-align: center; padding-top: 2mm; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
  /* Inline blocks never split across pages: a photo that does not fit moves to the next page whole. */
  .flow { display: block; font-size: 0; }
  .exact { display: inline-block; vertical-align: top; margin: 0 4mm 4mm 0; font-size: 10pt; break-inside: avoid; page-break-inside: avoid; }
  .exact .frame { display: flex; align-items: center; justify-content: center; overflow: hidden; box-sizing: border-box; border: 0.2mm dashed #bbb; }
  .exact .frame img { width: 100%; height: 100%; object-fit: contain; }
  .exact .frame img.fill { object-fit: cover; }
</style>
