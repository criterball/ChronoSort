<script>
  import { pickFolder, analyze, onProgress, loadReview, plural } from "../lib/api.js";
  import { app, sort } from "../lib/state.svelte.js";

  let { active } = $props();

  let progress = $state(null);
  let error = $state("");

  // Started from the command line with --analyze: run once the screen is visible.
  $effect(() => {
    if (active && sort.autostart && ready) {
      sort.autostart = false;
      start();
    }
  });

  async function browse(which) {
    const p = await pickFolder(which === "source" ? "Folder with your photos" : "Where the sorted copy goes");
    if (p) sort.settings[which] = p;
  }

  const sameOrNested = (a, b) => {
    const n = (x) => x.replace(/\\/g, "/").replace(/\/+$/, "").toLowerCase();
    const [x, y] = [n(a), n(b)];
    return x === y || x.startsWith(y + "/") || y.startsWith(x + "/");
  };
  let pathProblem = $derived(
    sort.settings.source && sort.settings.dest && sameOrNested(sort.settings.source, sort.settings.dest)
      ? "The destination must not be inside the source folder (or the other way round)."
      : "",
  );
  let ready = $derived(sort.settings.source && sort.settings.dest && !pathProblem && !app.busy);

  async function start() {
    error = "";
    app.busy = true;
    progress = { done: 0, total: 0 };
    const stop = await onProgress("analyze-progress", (p) => (progress = p));
    try {
      const a = await analyze(sort.settings.source);
      sort.analysis = a;
      sort.resolutions = {};
      sort.ai = {};
      sort.plan = null;
      sort.restored = "";
      // Pick up an earlier, unfinished check of this folder.
      try {
        const r = await loadReview();
        const n = Object.keys(r.resolutions).length;
        if (n || Object.keys(r.ai).length) {
          sort.resolutions = r.resolutions;
          sort.ai = Object.fromEntries(Object.entries(r.ai).map(([k, g]) => [k, { guess: g }]));
          const when = new Date(r.saved_at * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
          sort.restored = `Your earlier check of this folder (${when}) was picked up: ${plural(n, "year", "years")} you confirmed${r.dropped ? `; ${r.dropped} no longer fit because the file changed or is gone` : ""}.`;
        }
      } catch { /* nothing remembered */ }
      sort.step = sort.settings.goto === "plan" ? 4 : 1;
    } catch (e) {
      error = String(e);
    } finally {
      stop();
      app.busy = false;
      progress = null;
    }
  }
</script>

<section class="wrap">
  <div class="intro">
    <h1>Choose your folders</h1>
    <p class="muted">
      ChronoSort reads dates from EXIF, video data, sidecars, file and folder names, and asks you when it is unsure.
      Your originals are never changed in copy mode.
    </p>
  </div>

  <div class="card form">
    <div class="field">
      <label for="src">1. Where are your photos now?</label>
      <div class="row">
        <input id="src" type="text" placeholder="e.g. D:\Old_Photos" bind:value={sort.settings.source} />
        <button onclick={() => browse("source")}>Browse…</button>
      </div>
    </div>

    <div class="field">
      <label for="dst">2. Where should the sorted copy go?</label>
      <div class="row">
        <input id="dst" type="text" placeholder="e.g. D:\Sorted_Photos" bind:value={sort.settings.dest} />
        <button onclick={() => browse("dest")}>Browse…</button>
      </div>
      <p class="hint">Year folders like <span class="mono">2005</span> are created here. Running again later only adds what is new.</p>
    </div>

    <div class="field">
      <span class="label">3. Copy or move?</span>
      <div class="seg">
        <label class:on={sort.settings.mode === "copy"}>
          <input type="radio" bind:group={sort.settings.mode} value="copy" />
          <strong>Copy</strong><span class="muted">Originals stay exactly as they are (recommended)</span>
        </label>
        <label class:on={sort.settings.mode === "move"}>
          <input type="radio" bind:group={sort.settings.mode} value="move" />
          <strong>Move</strong><span class="muted">Originals go to the Recycle Bin only after a verified copy</span>
        </label>
      </div>
    </div>

    {#if pathProblem}<p class="problem">{pathProblem}</p>{/if}
    {#if error}<p class="problem">{error}</p>{/if}

    <div class="row actions">
      {#if progress}
        <div class="progress">
          <div class="bar" class:indeterminate={!progress.total}><div style="width: {progress.total ? (100 * progress.done) / progress.total : 3}%"></div></div>
          <span class="muted">Reading {progress.done} of {progress.total || "…"} files (read-only)</span>
        </div>
      {:else}
        <span class="muted">This step only reads your files. Nothing is written yet.</span>
      {/if}
      <span class="spacer"></span>
      <button class="primary big" data-continue disabled={!ready} onclick={start}>Look at my photos →</button>
    </div>
  </div>
</section>

<style>
  .wrap { max-width: 820px; margin: 0 auto; padding: 24px 24px 32px; display: grid; gap: 16px; }
  .intro { display: grid; gap: 6px; }
  .form { padding: 20px 24px; display: grid; gap: 18px; }
  .field { display: grid; gap: 8px; }
  label[for], .label { font-weight: 600; font-size: 15px; }
  .field input[type="text"] { flex: 1; padding: 9px 12px; }
  .hint { font-size: 12.5px; color: var(--muted); }
  .seg { display: grid; gap: 8px; }
  .seg label {
    display: grid; grid-template-columns: auto 90px 1fr; align-items: center; gap: 10px;
    padding: 10px 12px; border: 1px solid var(--border); border-radius: var(--radius-sm); cursor: pointer;
  }
  .seg label.on { border-color: var(--accent); background: var(--accent-soft); }
  .seg input { accent-color: var(--accent); }
  .problem { color: var(--red); background: var(--red-soft); padding: 8px 12px; border-radius: var(--radius-sm); }
  .actions { border-top: 1px solid var(--border); padding-top: 16px; }
  .progress { display: grid; gap: 6px; flex: 1; max-width: 420px; }
  .big { padding: 10px 20px; font-size: 15px; }
</style>
