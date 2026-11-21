<script>
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { pickFolder, onProgress, formatBytes, openFolder, plural } from "../lib/api.js";
  import { app } from "../lib/state.svelte.js";

  let info = $state(null);
  let target = $state("");
  let models = $state(true);
  let people = $state(false);
  let progress = $state(null);
  let report = $state(null);
  let error = $state("");

  onMount(async () => {
    try { info = await invoke("portable_info"); } catch (e) { error = String(e); }
  });

  async function browse() {
    const p = await pickFolder("Choose the USB stick (or a folder on it)");
    if (p) target = p;
  }

  async function copy() {
    error = ""; report = null;
    app.busy = true;
    progress = { done: 0, total: 0, file: "" };
    const stop = await onProgress("portable-progress", (p) => (progress = p));
    try {
      report = await invoke("make_portable", { target, models, people });
    } catch (e) {
      error = String(e);
    } finally {
      stop();
      app.busy = false;
      progress = null;
    }
  }
  const pct = (p) => (p?.total ? Math.floor((100 * p.done) / p.total) : 0);
</script>

<div class="card block">
  <h2>Take ChronoSort with you (USB stick)</h2>
  {#if info?.portable}
    <p class="good">✓ ChronoSort is running in portable mode. Everything it keeps (AI models, people, settings) is in
      <span class="mono">{info.data_dir}</span> and travels with this folder.</p>
  {/if}
  <p class="muted">Puts a copy of ChronoSort on a USB stick that runs on other Windows 10/11 PCs without installing anything. On each PC it checks the memory and graphics card again and uses the AI only if the PC can handle it.</p>

  <div class="row">
    <input type="text" class="mono grow" placeholder="Choose the USB stick, e.g. E:\" bind:value={target} disabled={app.busy} />
    <button disabled={app.busy} onclick={browse}>Browse…</button>
  </div>
  <label class="check"><input type="checkbox" bind:checked={models} disabled={app.busy} />
    Take the AI models along {#if info}<span class="muted">({formatBytes(info.models_bytes)})</span>{/if}
    <span class="muted small">— otherwise download them again on the stick when needed</span></label>
  <label class="check"><input type="checkbox" bind:checked={people} disabled={app.busy || !info?.people} />
    Take the people I named along {#if info}<span class="muted">({plural(info.people, "person", "people")})</span>{/if}
    <span class="muted small">— their names and face fingerprints; keep the stick safe</span></label>

  {#if progress}
    <div class="progress">
      <div class="bar" class:indeterminate={!progress.total}><div style="width:{pct(progress)}%"></div></div>
      <span class="muted small">{progress.total ? `Copying ${progress.file} … ${formatBytes(progress.done)} of ${formatBytes(progress.total)} (${pct(progress)}%)` : "Getting ready …"}</span>
    </div>
  {/if}
  {#if error}<p class="problem">{error}</p>{/if}
  {#if report}
    <div class="good">✓ Done: ChronoSort is in <span class="mono">{report.folder}</span>. On another PC, open that folder and double-click <strong>ChronoSort.exe</strong>.</div>
    {#each report.notes as n}<p class="muted small">{n}</p>{/each}
  {/if}
  <div class="row">
    <span class="muted small">USB 3 sticks are much faster; the large AI model then loads in under a minute.</span>
    <span class="spacer"></span>
    {#if report}<button onclick={() => openFolder(report.folder)}>Open the folder</button>{/if}
    <button class="primary" disabled={app.busy || !target.trim()} onclick={copy}>Copy to the stick</button>
  </div>
</div>

<style>
  .block { padding: 18px 22px; display: grid; gap: 10px; }
  .grow { flex: 1; }
  .check { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; cursor: pointer; }
  .check input { accent-color: var(--accent); }
  .small { font-size: 12.5px; }
  .progress { display: grid; gap: 6px; }
  .problem { color: var(--red); background: var(--red-soft); padding: 8px 12px; border-radius: var(--radius-sm); }
  .good { color: var(--green); font-weight: 600; }
</style>
