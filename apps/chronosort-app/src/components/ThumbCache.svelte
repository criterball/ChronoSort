<script>
  // Settings → Thumbnails: where the small preview pictures are kept (next to
  // ChronoSort, so they travel with it), how much room they take, a size limit,
  // emptying the cache, and making thumbnails for a folder in advance.
  import { onMount } from "svelte";
  import {
    thumbCacheInfo, thumbCacheClear, thumbCacheTrim, thumbCacheClearOld, thumbCachePrescan, thumbCacheCancel,
    pickFolder, onProgress, formatBytes, openFolder, plural, confirmBox,
  } from "../lib/api.js";
  import { app } from "../lib/state.svelte.js";
  import { cacheLimit, setCacheLimit, LIMITS } from "../lib/thumbcache.js";

  let info = $state(null);
  let limit = $state(cacheLimit());
  let note = $state("");
  let error = $state("");
  let folder = $state("");
  let deep = $state(true);
  let previews = $state(false);
  let prog = $state(null);

  async function refresh() {
    try { info = await thumbCacheInfo(); } catch (e) { error = String(e); }
  }
  onMount(refresh);

  async function changeLimit() {
    setCacheLimit(+limit);
    if (+limit > 0) await tidy();
  }
  async function tidy() {
    try {
      const r = await thumbCacheTrim(+limit || 1e9);
      note = r.files ? `${plural(r.files, "old thumbnail", "old thumbnails")} removed (${formatBytes(r.bytes)}).` : "The cache is within the limit; nothing to remove.";
      await refresh();
    } catch (e) { error = String(e); }
  }
  async function clearAll() {
    const ok = await confirmBox("Remove all thumbnails? Your photos are not touched; the thumbnails are made again when you look at a folder (that takes a moment the first time).", "Empty the thumbnail cache", "Remove them", "Keep them");
    if (!ok) return;
    try {
      const r = await thumbCacheClear();
      note = `${plural(r.files, "thumbnail", "thumbnails")} removed (${formatBytes(r.bytes)}).`;
      await refresh();
    } catch (e) { error = String(e); }
  }
  async function clearOld() {
    try {
      const r = await thumbCacheClearOld();
      note = `The old cache was removed (${formatBytes(r.bytes)}).`;
      await refresh();
    } catch (e) { error = String(e); }
  }
  async function chooseFolder() {
    const p = await pickFolder("Which folder should get its thumbnails now?");
    if (p) folder = p;
  }
  async function prescan() {
    error = ""; note = "";
    app.busy = true;
    prog = { done: 0, total: 0, name: "" };
    const stop = await onProgress("cache-progress", (p) => (prog = p));
    try {
      const sizes = previews ? [320, 640, 1280] : [320, 640];
      const r = await thumbCachePrescan(folder, deep, sizes);
      note = r.photos
        ? `${r.cancelled ? "Stopped. " : ""}Thumbnails ready for ${plural(r.made, "photo or video", "photos and videos")}${r.failed ? `; ${r.failed} could not be read` : ""}.`
        : "There are no photos or videos in this folder.";
      await refresh();
    } catch (e) {
      error = String(e);
    } finally {
      stop();
      app.busy = false;
      prog = null;
    }
  }
</script>

<div class="card block">
  <h2>Thumbnails</h2>
  <p class="muted">ChronoSort keeps small copies of your photos so folders open quickly. They are only copies — removing them never touches a photo.</p>
  {#if info}
    <div class="row wrap">
      <span><strong>{plural(info.files, "thumbnail", "thumbnails")}</strong> · {formatBytes(info.bytes)}</span>
      <span class="spacer"></span>
      <button onclick={() => openFolder(info.dir)}>Show the folder</button>
    </div>
    <p class="small mono path" title={info.dir}>{info.dir}</p>
    {#if info.beside_program}
      <p class="good small">✓ Kept next to ChronoSort, so they travel with it (for example on a USB stick).</p>
    {:else}
      <p class="muted small">ChronoSort's own folder cannot be written to here, so the PC's cache folder is used instead.</p>
    {/if}
    {#if info.old_dir}
      <div class="row wrap">
        <span class="muted small">An older ChronoSort left thumbnails in the PC's folders ({formatBytes(info.old_bytes)}).</span>
        <span class="spacer"></span>
        <button onclick={clearOld} disabled={app.busy}>Remove the old cache</button>
      </div>
    {/if}
  {/if}

  <div class="row wrap">
    <label class="lim">Keep the cache under
      <select bind:value={limit} onchange={changeLimit}>
        {#each LIMITS as [v, t]}<option value={v}>{t}</option>{/each}
      </select>
    </label>
    <span class="muted small">When it grows past this, the thumbnails you have not looked at for the longest time are removed.</span>
    <span class="spacer"></span>
    <button onclick={tidy} disabled={app.busy}>Tidy up now</button>
    <button onclick={clearAll} disabled={app.busy}>Empty the cache…</button>
  </div>

  <h3>Make thumbnails in advance</h3>
  <p class="muted small">For a big photo collection: make all thumbnails now (for example overnight), so every folder opens instantly later. You can also right-click a folder → <em>Make thumbnails now</em>.</p>
  <div class="row">
    <input type="text" class="mono grow" placeholder="Choose a folder, e.g. D:\Photos" bind:value={folder} disabled={app.busy} />
    <button onclick={chooseFolder} disabled={app.busy}>Browse…</button>
  </div>
  <label class="check"><input type="checkbox" bind:checked={deep} disabled={app.busy} /> Include all sub-folders</label>
  <label class="check"><input type="checkbox" bind:checked={previews} disabled={app.busy} /> Also the large previews for full-screen viewing <span class="muted small">(uses more room)</span></label>
  {#if prog}
    <div class="progress">
      <div class="bar" class:indeterminate={!prog.total}><div style="width:{prog.total ? (100 * prog.done) / prog.total : 0}%"></div></div>
      <span class="muted small">{prog.total ? `${prog.done} of ${prog.total} · ${prog.name}` : prog.name || "Looking for photos…"}</span>
    </div>
  {/if}
  {#if error}<p class="problem">{error}</p>{/if}
  {#if note}<p class="good small">{note}</p>{/if}
  <div class="row">
    <span class="spacer"></span>
    {#if prog}<button onclick={() => thumbCacheCancel()}>Stop</button>{/if}
    <button class="primary" disabled={app.busy || !folder.trim()} onclick={prescan}>Make thumbnails now</button>
  </div>
</div>

<style>
  .block { padding: 18px 22px; display: grid; gap: 10px; }
  h3 { margin: 8px 0 0; font-size: 15px; }
  .wrap { flex-wrap: wrap; }
  .grow { flex: 1; }
  .path { margin: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted); }
  .lim { display: flex; align-items: center; gap: 8px; }
  .check { display: flex; gap: 8px; align-items: baseline; cursor: pointer; }
  .check input { accent-color: var(--accent); }
  .small { font-size: 12.5px; }
  .progress { display: grid; gap: 6px; }
  .problem { color: var(--red); background: var(--red-soft); padding: 8px 12px; border-radius: var(--radius-sm); }
  .good { color: var(--green); }
</style>
