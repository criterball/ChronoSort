<script>
  import { invoke } from "@tauri-apps/api/core";
  import { pickFolder, confirmBox, onProgress, thumbUrl, plural, openFolder } from "../lib/api.js";
  import { app, places as st, resetPlaces } from "../lib/state.svelte.js";

  let { active } = $props();

  let progress = $state(null);
  let error = $state("");
  let broken = $state({});
  let query = $state("");
  let results = $state(null); // { places, countries }
  let searching = $state(false);
  let searchTimer;

  async function browse() {
    const p = await pickFolder("Folder with the photos that need a place");
    if (p) st.folder = p;
  }

  async function look() {
    error = "";
    app.busy = true;
    progress = { done: 0, total: 0 };
    const stop = await onProgress("places-progress", (p) => (progress = p));
    try {
      st.scan = await invoke("places_scan", { folder: st.folder });
      // Photos that already have a place are not ticked (never overwritten by accident).
      st.chosen = Object.fromEntries(st.scan.files.map((f) => [f.rel, !f.has_place]));
      st.phase = "choose";
    } catch (e) {
      error = String(e);
    } finally {
      stop();
      app.busy = false;
      progress = null;
    }
  }

  // Place search (offline list; the first search loads it, a few seconds).
  $effect(() => {
    const q = query;
    clearTimeout(searchTimer);
    if (q.trim().length < 2) { results = null; return; }
    searchTimer = setTimeout(async () => {
      searching = true;
      try { results = await invoke("places_search", { query: q }); } catch (e) { error = String(e); }
      finally { searching = false; }
    }, 250);
  });

  let groups = $derived.by(() => {
    if (!st.scan) return [];
    const m = new Map();
    for (const f of st.scan.files) {
      const d = f.rel.includes("/") ? f.rel.slice(0, f.rel.lastIndexOf("/")) : "(top folder)";
      if (!m.has(d)) m.set(d, []);
      m.get(d).push(f);
    }
    return [...m.entries()];
  });
  let count = $derived(st.scan ? st.scan.files.filter((f) => st.chosen[f.rel]).length : 0);
  let withPlace = $derived(st.scan ? st.scan.files.filter((f) => f.has_place).length : 0);
  const setGroup = (files, on) => { for (const f of files) st.chosen[f.rel] = on; st.chosen = { ...st.chosen }; };

  const labelOf = (t) => t.kind === "city" ? [t.place.city, t.place.region, t.place.country].filter((x, i, a) => x && a.indexOf(x) === i).join(", ") : `${t.name} (country only)`;

  async function apply() {
    const rels = st.scan.files.filter((f) => st.chosen[f.rel]).map((f) => f.rel);
    const replacing = st.scan.files.filter((f) => st.chosen[f.rel] && f.has_place).length;
    const ok = await confirmBox(
      `Add “${labelOf(st.target)}” to ${plural(rels.length, "photo", "photos")}?\n\n` +
      (st.target.kind === "city" ? "The town's map position and its name are written into each photo." : "Only the country name is written; no map position.") +
      (replacing ? `\n${plural(replacing, "photo already has", "photos already have")} a place; it will be replaced.` : "") +
      "\n\nThis changes these files. A copy of each photo is kept first, so you can undo it.",
      "Add a place", "Add the place", "Cancel",
    );
    if (!ok) return;
    error = "";
    app.busy = true;
    progress = { done: 0, total: rels.length };
    const stop = await onProgress("places-progress", (p) => (progress = p));
    try {
      st.report = await invoke("places_apply", { folder: st.scan.folder, rels, target: st.target });
      st.undone = null;
      st.phase = "done";
    } catch (e) {
      error = String(e);
    } finally {
      stop();
      app.busy = false;
      progress = null;
    }
  }

  async function undo(run) {
    const ok = await confirmBox("Put these photos back the way they were before the place was added?", "Undo", "Undo", "Cancel");
    if (!ok) return;
    error = "";
    app.busy = true;
    try {
      const u = await invoke("places_undo", { folder: st.scan.folder, run });
      // Look again so the pins, ticks and the list of earlier runs match the files now
      // (a run with files that could not be put back stays in the list).
      if (st.phase === "choose") await look();
      st.undone = u;
    } catch (e) {
      error = String(e);
    } finally {
      app.busy = false;
    }
  }
</script>

{#snippet undoneNote()}
  {#if st.undone}
    {@const u = st.undone}
    <p class="good">✓ Undone: {plural(u.restored, "photo", "photos")} put back.{u.kept.length ? ` ${plural(u.kept.length, "was", "were")} changed again since and left alone.` : ""}</p>
    {#if u.failed.length}
      <div class="card list"><h3>Could not be put back ({u.failed.length})</h3>
        <p class="muted small">The copies are still kept in <span class="mono">.chronosort\place-backups</span>, so you can try Undo again.</p>
        {#each u.failed as [p, why]}<div class="mono small"><strong>{p}</strong> — {why}</div>{/each}</div>
    {/if}
  {/if}
{/snippet}

<section class="wrap">
  <div class="head">
    <h1>Add a place to photos</h1>
    <p class="muted">Write where photos were taken, for example the town of a holiday, into many photos at once. Photo apps can then show them on a map or find them by place. The list of places is built in, so no internet is needed.</p>
  </div>

  {#if error}<p class="problem">{error}</p>{/if}

  {#if st.phase === "pick"}
    <div class="card form">
      <span class="label">Folder with the photos</span>
      <div class="row">
        <input type="text" class="mono grow" placeholder="Choose a folder, e.g. a sorted year folder" bind:value={st.folder} disabled={app.busy}
          onkeydown={(e) => e.key === "Enter" && st.folder.trim() && look()} />
        <button disabled={app.busy} onclick={browse}>Browse…</button>
      </div>
      <p class="muted small">Tip: use this on your sorted copies (for example <span class="mono">Sorted\2005</span>), not on your only originals. A copy of every photo is kept before it is changed, so it can be undone.</p>
      <div class="row actions">
        {#if progress}<div class="progress"><div class="bar indeterminate"><div></div></div><span class="muted small">Looking at the photos …</span></div>{/if}
        <span class="spacer"></span>
        <button class="primary go" data-continue disabled={!st.folder.trim() || app.busy} onclick={look}>Look at the photos</button>
      </div>
    </div>
  {:else if st.phase === "choose"}
    <div class="row head2">
      <div>
        <h2>{plural(st.scan.files.length, "photo or video", "photos and videos")}</h2>
        <p class="muted">{withPlace ? `${plural(withPlace, "already has", "already have")} a place (not ticked). ` : ""}{plural(count, "is", "are")} ticked.</p>
      </div>
      <span class="spacer"></span>
      <button class="ghost" disabled={app.busy} onclick={resetPlaces}>← Another folder</button>
    </div>

    {@render undoneNote()}

    {#if st.scan.runs.length}
      <div class="card runs">
        <strong>Earlier in this folder</strong>
        {#each st.scan.runs as r (r.run)}
          <div class="row"><span>“{r.place}” added to {plural(r.changed.length, "photo", "photos")}</span><span class="spacer"></span>
            <button class="small" disabled={app.busy} onclick={() => undo(r.run)}>Undo</button></div>
        {/each}
      </div>
    {/if}

    <div class="card where">
      <span class="label">Which place?</span>
      <input type="text" class="big" placeholder="Type a town or country, e.g. Adelaide, Paris, Italy" bind:value={query} disabled={app.busy} />
      {#if searching && !results}<div class="progress"><div class="bar indeterminate"><div></div></div><span class="muted small">Loading the list of places (first time only) …</span></div>{/if}
      {#if results}
        <div class="choices">
          {#each results.places as p}
            <button class="choice" class:on={st.target?.kind === "city" && st.target.place.lat === p.lat && st.target.place.lon === p.lon}
              onclick={() => (st.target = { kind: "city", place: p })}>
              <strong>{p.city}</strong> <span class="muted">{[p.region, p.country].filter(Boolean).join(", ")}</span>
              <span class="muted small">{p.population >= 1000 ? `${Math.round(p.population / 1000).toLocaleString()} thousand people` : ""}</span>
            </button>
          {/each}
          {#each results.countries as c}
            <button class="choice" class:on={st.target?.kind === "country" && st.target.code === c.code}
              onclick={() => (st.target = { kind: "country", name: c.name, code: c.code })}>
              <strong>{c.name}</strong> <span class="muted">country only (no map position)</span>
            </button>
          {/each}
          {#if !results.places.length && !results.countries.length}<p class="muted">No place found. Try another spelling or a bigger town nearby.</p>{/if}
        </div>
      {/if}
    </div>

    {#each groups as [dir, files] (dir)}
      <div class="card group">
        <div class="row">
          <span class="mono folder">{dir}</span><span class="muted small">{files.length}</span><span class="spacer"></span>
          <button class="small ghost" onclick={() => setGroup(files, true)}>Tick all</button>
          <button class="small ghost" onclick={() => setGroup(files, false)}>Untick all</button>
        </div>
        <div class="grid">
          {#each files as f (f.rel)}
            <label class="pic" class:on={st.chosen[f.rel]} title={f.rel}>
              <input type="checkbox" bind:checked={st.chosen[f.rel]} />
              {#if broken[f.key]}<span class="ph">{f.kind === "video" ? "▶" : f.rel.split(".").pop().toUpperCase()}</span>
              {:else}<img src={thumbUrl(f.key)} alt="" loading="lazy" onerror={() => (broken = { ...broken, [f.key]: true })} />{/if}
              {#if f.has_place}<span class="pin" title="Already has a place">📍</span>{/if}
            </label>
          {/each}
        </div>
      </div>
    {/each}

    <div class="card foot">
      {#if progress}
        <div class="progress grow"><div class="bar"><div style="width:{progress.total ? (100 * progress.done) / progress.total : 0}%"></div></div>
          <span class="muted small">Adding the place: {progress.done} of {progress.total} …</span></div>
        <button onclick={() => invoke("places_cancel")}>Stop</button>
      {:else}
        <span class="muted">{st.target ? `Place: ${labelOf(st.target)}` : "Choose a place above."}</span>
        <span class="spacer"></span>
        <button class="primary go" data-continue disabled={!st.target || !count || app.busy} onclick={apply}>
          Add the place to {plural(count, "photo", "photos")}
        </button>
      {/if}
    </div>
  {:else if st.phase === "done"}
    {@const r = st.report}
    <div class="card done">
      <div class="tick">✓</div>
      <div>
        <h2>“{labelOf(st.target)}” was added to {plural(r.changed, "photo", "photos")}</h2>
        <p class="muted">A copy of each photo was kept first, in <span class="mono">.chronosort\place-backups</span> inside the folder.</p>
      </div>
    </div>
    {#if r.failed.length}
      <div class="card list"><h3>Not changed ({r.failed.length})</h3>
        {#each r.failed as [p, why]}<div class="mono small"><strong>{p}</strong> — {why}</div>{/each}</div>
    {/if}
    {@render undoneNote()}
    <div class="row">
      {#if (!st.undone || st.undone.failed.length) && r.changed}<button class="danger" disabled={app.busy} onclick={() => undo(r.run)}>Undo</button>{/if}
      <span class="spacer"></span>
      <button onclick={() => openFolder(st.scan.folder)}>Open the folder</button>
      <button class="primary" data-continue onclick={() => { st.phase = "choose"; st.report = null; st.undone = null; look(); }}>Add another place</button>
    </div>
  {/if}
</section>

<style>
  .wrap { max-width: 1240px; margin: 0 auto; padding: 24px 24px 48px; display: grid; gap: 16px; }
  .head { display: grid; gap: 4px; }
  .row.head2 { display: flex; }
  .form, .where, .runs, .foot, .list { padding: 16px 20px; display: grid; gap: 10px; }
  .foot { display: flex; align-items: center; gap: 12px; position: sticky; bottom: 8px; z-index: 2; border-color: var(--accent); }
  .label { font-weight: 600; font-size: 15px; }
  .grow { flex: 1; }
  .small { font-size: 12.5px; }
  .actions { border-top: 1px solid var(--border); padding-top: 12px; }
  .progress { display: grid; gap: 6px; max-width: 460px; }
  .problem { color: var(--red); background: var(--red-soft); padding: 8px 12px; border-radius: var(--radius-sm); }
  .good { color: var(--green); font-weight: 600; }
  .go { font-size: 15px; padding: 10px 20px; }
  input.big { font-size: 17px; padding: 10px 12px; }
  .choices { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 6px; }
  .choice { text-align: left; display: grid; gap: 2px; padding: 8px 12px; }
  .choice.on { border-color: var(--accent); background: var(--accent-soft); }
  .group { padding: 12px 16px; display: grid; gap: 8px; }
  .folder { font-weight: 600; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(96px, 1fr)); gap: 6px; }
  .pic { position: relative; aspect-ratio: 1; border-radius: 8px; overflow: hidden; border: 3px solid transparent; background: var(--surface-2); cursor: pointer; display: grid; place-items: center; opacity: 0.55; }
  .pic.on { border-color: var(--accent); opacity: 1; }
  .pic img { width: 100%; height: 100%; object-fit: cover; }
  .pic input { position: absolute; top: 5px; left: 5px; width: 18px; height: 18px; accent-color: var(--accent); }
  .pin { position: absolute; bottom: 3px; right: 5px; font-size: 15px; }
  .ph { color: var(--muted); font-weight: 600; }
  .done { padding: 20px; display: flex; gap: 16px; align-items: center; }
  .tick { width: 46px; height: 46px; border-radius: 50%; display: grid; place-items: center; background: var(--green-soft); color: var(--green); font-size: 22px; font-weight: 700; flex: none; }
</style>
