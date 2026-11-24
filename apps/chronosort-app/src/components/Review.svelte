<script>
  import { thumbUrl, SOURCE_LABELS, aiDate, aiCancel, aiStop, onProgress, plural, saveReview } from "../lib/api.js";
  import { app, sort } from "../lib/state.svelte.js";
  import Lightbox from "./Lightbox.svelte";

  const analysis = $derived(sort.analysis);

  let filter = $state("open"); // open | done | all
  let broken = $state({}); // hash -> true when no thumbnail is available

  const folderOf = (rel) => (rel.includes("/") ? rel.slice(0, rel.lastIndexOf("/")) : "(top folder)");
  const nameOf = (rel) => rel.slice(rel.lastIndexOf("/") + 1);
  // Remember decisions as they are made (a long check survives closing the app).
  let saveTimer;
  $effect(() => {
    const res = sort.resolutions;
    const ai = Object.fromEntries(Object.entries(sort.ai).filter(([, v]) => v.guess).map(([k, v]) => [k, v.guess]));
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => saveReview(res, ai).catch(() => {}), 600);
  });

  const aiGuess = (it) => sort.ai[it.rel_path]?.guess ?? null;
  // The date engine's suggestion first; the AI's estimate only when there is none.
  const suggested = (it) => it.decision.suggested ?? aiGuess(it)?.best_year ?? null;

  // ---- Ask the AI (local vision model; a suggestion only, the user decides)
  let ai = $state({ phase: "idle", done: 0, total: 0, error: "" }); // idle | starting | running | done
  async function askAi() {
    const rels = analysis.review
      .filter((it) => !sort.resolutions[it.rel_path] && !sort.ai[it.rel_path]?.guess && it.kind === "image")
      .map((it) => it.rel_path);
    if (!rels.length) return;
    ai = { phase: "starting", done: 0, total: rels.length, error: "" };
    app.busy = true;
    const stops = [
      await onProgress("ai-status", (s) => (ai.phase = s)),
      await onProgress("ai-date", (e) => {
        sort.ai = { ...sort.ai, [e.rel]: e.guess ? { guess: { ...e.guess, people: e.people } } : { error: e.error } };
        ai.done = e.done;
      }),
    ];
    try {
      await aiDate(rels);
      ai.phase = "done";
    } catch (e) {
      ai = { ...ai, phase: "idle", error: String(e) };
    } finally {
      stops.forEach((s) => s());
      app.busy = false;
    }
  }
  async function turnOff() {
    await aiStop().catch(() => {});
    ai.phase = "idle";
  }
  let aiOpen = $derived(analysis.review.filter((it) => !sort.resolutions[it.rel_path] && !sort.ai[it.rel_path]?.guess && it.kind === "image").length);

  // Review items grouped by source folder: one decision often fits a whole box of scans.
  let groups = $derived.by(() => {
    const m = new Map();
    for (const it of analysis.review) {
      const f = folderOf(it.rel_path);
      if (!m.has(f)) m.set(f, []);
      m.get(f).push(it);
    }
    return [...m.entries()].map(([folder, items]) => {
      const counts = {};
      for (const it of items) if (suggested(it)) counts[suggested(it)] = (counts[suggested(it)] ?? 0) + 1;
      const common = Object.entries(counts).sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
      return { folder, items, common: common ? Number(common) : null };
    });
  });

  let confirmed = $derived(analysis.review.filter((it) => sort.resolutions[it.rel_path]).length);

  const visible = (it) =>
    filter === "all" || (filter === "open" ? !sort.resolutions[it.rel_path] : !!sort.resolutions[it.rel_path]);

  function setYear(it, v) {
    const y = Number(v);
    const next = { ...sort.resolutions };
    if (Number.isInteger(y) && y >= 1826 && y <= new Date().getFullYear()) next[it.rel_path] = y;
    else delete next[it.rel_path];
    sort.resolutions = next;
  }
  function acceptGroup(g, year) {
    const next = { ...sort.resolutions };
    for (const it of g.items) {
      const y = year ?? suggested(it);
      if (y) next[it.rel_path] = Number(y);
    }
    sort.resolutions = next;
  }
  let groupYear = $state({});

  // One chip per (source, year): EXIF original + digitized usually say the same thing.
  const evidence = (it, max = 4) => {
    const seen = new Set();
    const out = [];
    const g = aiGuess(it);
    for (const p of g?.people ?? []) {
      out.push({ label: `${p.name} (born ${p.born}) is in this photo: not before`, year: p.born, strong: true,
        note: "Recognised from the people you named. A photo cannot be older than the people in it." });
    }
    if (g?.date_stamp) out.push({ label: "Printed date", year: g.date_stamp, strong: false, note: "The AI read a date printed in the picture. Check it on the large preview." });
    if (g) {
      const range = g.earliest_year === g.latest_year ? "" : ` (${g.earliest_year}–${g.latest_year})`;
      out.push({ label: "AI ≈", year: `${g.best_year}${range}`, ai: true, note: `The local AI's estimate (${g.confidence} confidence): ${g.clues.join("; ")}` });
    }
    for (const e of it.evidence) {
      if (e.source.startsWith("fs_")) continue;
      const label = SOURCE_LABELS[e.source] ?? e.source;
      const key = label + e.date.year;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({ label, year: e.date.year, strong: e.weight >= 0.75, distrusted: e.weight <= 0.1, note: e.note });
    }
    return out.slice(0, max);
  };

  // Large preview: the list is frozen when it opens, so confirming a file
  // (which hides it from "Open") does not change what ← / → step through.
  let lbItems = $state(null);
  let lbIndex = $state(0);
  function openPreview(it) {
    const list = groups.flatMap((g) => g.items.filter(visible));
    lbItems = list;
    lbIndex = Math.max(0, list.indexOf(it));
  }
  const isVideo = (it) => it.kind === "video";
</script>

<section class="wrap">
  <div class="head row">
    <div>
      <h1>Check {analysis.review.length} uncertain {analysis.review.length === 1 ? "file" : "files"}</h1>
      <p class="muted">Confirm a year, or leave a file open: open files are not copied this time.</p>
    </div>
    <span class="spacer"></span>
    <div class="tabs">
      <button class:on={filter === "open"} onclick={() => (filter = "open")}>Open {analysis.review.length - confirmed}</button>
      <button class:on={filter === "done"} onclick={() => (filter = "done")}>Confirmed {confirmed}</button>
      <button class:on={filter === "all"} onclick={() => (filter = "all")}>All</button>
    </div>
    <button onclick={() => (sort.step = 1)}>← Overview</button>
    <button class="primary" data-continue onclick={() => (sort.step = 3)}>Continue to file names →</button>
  </div>

  {#if sort.restored}<p class="good restored">✓ {sort.restored}</p>{/if}
  {#if analysis.review.length}
    <div class="card aibar">
      <div class="aitxt">
        <strong>Not sure? Ask the AI.</strong>
        <span class="muted">The AI on this PC looks at each open photo (a printed date, the kind of print, clothes, cars …) and suggests a year. Your photos never leave this PC. You still decide.</span>
        {#if ai.phase === "starting"}
          <div class="progress"><div class="bar indeterminate"><div></div></div>
            <span class="muted">Starting the AI (loading the model into memory) … this can take up to a minute.</span></div>
        {:else if ai.phase === "running"}
          <div class="progress"><div class="bar" class:indeterminate={!ai.done}><div style="width: {ai.total ? (100 * ai.done) / ai.total : 3}%"></div></div>
            <span class="muted">Looked at {ai.done} of {ai.total} photos</span></div>
        {:else if ai.phase === "done"}<span class="good">✓ Done. The AI's years are shown as “AI ≈” on each photo.</span>{/if}
        {#if ai.error}<p class="problem">{ai.error}</p>{/if}
      </div>
      <div class="aibtn">
        {#if ai.phase === "starting" || ai.phase === "running"}
          <button onclick={() => aiCancel()}>Stop</button>
        {:else}
          <button class="primary" disabled={!aiOpen || app.busy} onclick={askAi}>Ask the AI ({plural(aiOpen, "photo", "photos")})</button>
          {#if ai.phase === "done"}<button class="ghost small" onclick={turnOff} title="Frees the graphics memory the AI uses">Turn the AI off</button>{/if}
        {/if}
      </div>
    </div>
  {/if}

  {#if analysis.review.length === 0}
    <div class="card empty"><h2>Nothing to check</h2><p class="muted">Every file has a reliable date.</p></div>
  {/if}

  <div class="groups">
  {#each groups as g (g.folder)}
    {#if g.items.some(visible)}
      {@const n = g.items.filter(visible).length}
      <div class="group" class:wide={n > 3} style={n > 3 ? "" : `--cols:${n}`}>
        <div class="ghead row">
          <span class="folder mono">{g.folder}</span>
          <span class="muted">{g.items.length} {g.items.length === 1 ? "file" : "files"}</span>
          <span class="spacer"></span>
          {#if g.common && g.items.length > 1}
            <button class="small" onclick={() => acceptGroup(g, null)}>Accept suggestions ({g.common})</button>
          {/if}
          {#if g.items.length > 1}
            <input
              type="number"
              class="yearbox"
              placeholder="Year"
              bind:value={groupYear[g.folder]}
              onkeydown={(e) => e.key === "Enter" && groupYear[g.folder] && acceptGroup(g, groupYear[g.folder])}
            />
            <button class="small" disabled={!groupYear[g.folder]} onclick={() => acceptGroup(g, groupYear[g.folder])}>
              Set all
            </button>
          {/if}
        </div>
        <div class="grid">
          {#each g.items.filter(visible) as it (it.rel_path)}
            {@const done = sort.resolutions[it.rel_path]}
            <article class="card item" class:done>
              <button class="thumb" onclick={() => openPreview(it)} title="Show large (click)">
                {#if broken[it.hash] || !it.hash}
                  <div class="placeholder" title={isVideo(it) ? "No preview for this kind of video; the optional FFmpeg pack in Settings → AI models reads more kinds" : ""}>{isVideo(it) ? "▶ video" : it.format.toUpperCase()}</div>
                {:else}
                  <img
                    src={thumbUrl(it.hash)}
                    alt={nameOf(it.rel_path)}
                    loading="lazy"
                    onerror={() => (broken = { ...broken, [it.hash]: true })}
                  />
                {/if}
                {#if isVideo(it) && !broken[it.hash] && it.hash}<span class="play" aria-hidden="true">▶</span>{/if}
                {#if it.flags.some((f) => f.startsWith("damaged"))}<span class="chip red badge">damaged</span>{/if}
                <span class="zoom" aria-hidden="true">⤢</span>
              </button>
              <div class="body">
                <div class="name" title={it.rel_path}>{nameOf(it.rel_path)}</div>
                <p class="reason">{it.decision.reason}</p>
                <div class="chips">
                  {#each evidence(it) as e}
                    <span class="chip" class:accent={e.strong} class:ai={e.ai} class:distrusted={e.distrusted} title={e.note}>{e.label} {e.year}</span>
                  {/each}
                </div>
                <div class="row act">
                  <input
                    type="number"
                    class="yearbox"
                    placeholder={suggested(it) ?? "Year"}
                    value={done ?? ""}
                    onchange={(e) => setYear(it, e.currentTarget.value)}
                    onkeydown={(e) => e.key === "Enter" && setYear(it, e.currentTarget.value || suggested(it))}
                  />
                  {#if done}
                    <span class="chip green">✓ {done}</span>
                    <button class="ghost small" onclick={() => setYear(it, "")}>Undo</button>
                  {:else if suggested(it)}
                    <button class="small" onclick={() => setYear(it, suggested(it))}>✓ {suggested(it)}</button>
                  {/if}
                </div>
              </div>
            </article>
          {/each}
        </div>
      </div>
    {/if}
  {/each}
  </div>
</section>

{#if lbItems}
  <Lightbox
    items={lbItems}
    bind:index={lbIndex}
    resolutions={sort.resolutions}
    {suggested}
    {evidence}
    onYear={setYear}
    onClose={() => (lbItems = null)}
  />
{/if}

<style>
  .wrap { max-width: 1240px; margin: 0 auto; padding: 24px 24px 64px; display: grid; gap: 20px; }
  .head { gap: 16px; flex-wrap: wrap; }
  .tabs { display: flex; background: var(--surface-2); border-radius: var(--radius-sm); padding: 3px; }
  .tabs button { border: none; background: transparent; padding: 5px 12px; color: var(--muted); }
  .tabs button.on { background: var(--surface); color: var(--text); box-shadow: var(--shadow); }
  .empty { padding: 32px; text-align: center; display: grid; gap: 6px; }
  .groups { display: flex; flex-wrap: wrap; gap: 22px 18px; align-items: flex-start; }
  .group { display: grid; gap: 10px; flex: 0 0 auto; }
  .group:not(.wide) .grid { grid-template-columns: repeat(var(--cols), 250px); }
  .group.wide { flex: 1 1 100%; }
  .ghead { padding: 0 4px; min-height: 30px; flex-wrap: wrap; }
  .folder { font-weight: 600; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(250px, 1fr)); gap: 14px; }
  .item { overflow: hidden; display: grid; grid-template-rows: 170px auto; border-width: 1.5px; }
  .item.done { border-color: var(--green); }
  .thumb {
    position: relative; background: var(--surface-2); display: grid; place-items: center; overflow: hidden;
    padding: 0; border: none; border-radius: 0; cursor: zoom-in; width: 100%; height: 100%;
  }
  .thumb:active:not(:disabled) { transform: none; }
  .zoom {
    position: absolute; right: 8px; bottom: 8px; width: 30px; height: 30px; border-radius: 8px; display: grid; place-items: center;
    background: rgba(0, 0, 0, 0.55); color: #fff; font-size: 16px; opacity: 0; transition: opacity 0.12s;
  }
  .thumb:hover .zoom, .thumb:focus-visible .zoom { opacity: 1; }
  .thumb img { width: 100%; height: 100%; object-fit: cover; }
  .placeholder { color: var(--muted); font-weight: 600; letter-spacing: 0.04em; }
  .badge { position: absolute; top: 8px; left: 8px; }
  .play { position: absolute; left: 50%; top: 50%; transform: translate(-50%, -50%); width: 44px; height: 44px; border-radius: 50%;
    display: grid; place-items: center; background: rgba(0, 0, 0, 0.55); color: #fff; font-size: 18px; pointer-events: none; }
  .body { padding: 10px 12px 12px; display: grid; gap: 6px; }
  .name { font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .reason { font-size: 12.5px; color: var(--muted); display: -webkit-box; -webkit-line-clamp: 3; line-clamp: 3; -webkit-box-orient: vertical; overflow: hidden; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; }
  .act { margin-top: 2px; }
  .chip.distrusted { text-decoration: line-through; opacity: 0.7; }
  .chip.ai { background: var(--amber-soft); color: var(--amber); }
  .aibar { display: flex; gap: 18px; align-items: center; padding: 14px 18px; }
  .aitxt { display: grid; gap: 6px; flex: 1; }
  .aibtn { display: flex; gap: 6px; align-items: center; }
  .progress { display: grid; gap: 4px; max-width: 420px; }
  .problem { color: var(--red); background: var(--red-soft); padding: 8px 12px; border-radius: var(--radius-sm); }
  .good { color: var(--green); font-weight: 600; }
  .yearbox { width: 84px; padding: 5px 8px; }
</style>
