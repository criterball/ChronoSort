<script>
  import { run, undo, onProgress, formatBytes, openFolder, confirmBox } from "../lib/api.js";
  import { app, sort, resetSort } from "../lib/state.svelte.js";

  const settings = sort.settings;
  let progress = $state({ done: 0, total: sort.plan?.files ?? 0, bytes: 0 });
  let report = $state(null);
  let undone = $state(null);
  let error = $state("");
  let started = false;

  async function start() {
    if (started) return;
    started = true;
    app.busy = true;
    const stop = await onProgress("run-progress", (p) => (progress = p));
    try {
      report = await run(settings.mode, 4);
    } catch (e) {
      error = String(e);
    } finally {
      stop();
      app.busy = false;
    }
  }
  $effect(() => {
    start();
  });

  async function doUndo() {
    const ok = await confirmBox(
      "Undo this run? Copies that you have not changed will be removed" + (settings.mode === "move" ? " and the originals put back." : "."),
      "Undo",
      "Undo",
      "Cancel",
    );
    if (!ok) return;
    app.busy = true;
    try {
      undone = await undo(settings.dest);
    } catch (e) {
      error = String(e);
    } finally {
      app.busy = false;
    }
  }
  let pct = $derived(progress.total ? Math.round((100 * progress.done) / progress.total) : 0);
</script>

<section class="wrap">
  {#if !report && !error}
    <div class="card running">
      <h1>{settings.mode === "move" ? "Moving" : "Copying"} and verifying…</h1>
      <div class="bar big"><div style="width:{pct}%"></div></div>
      <p class="muted">{progress.done} of {progress.total} files · {formatBytes(progress.bytes)} · {pct}%</p>
      <p class="muted small">Each file is hashed while copying, re-read and compared, and gets its original dates. Safe to interrupt: the next run resumes.</p>
    </div>
  {/if}

  {#if error}<div class="card problem"><h2>Something went wrong</h2><p>{error}</p><p class="muted">Your originals are untouched.</p>
    {#if !report}<div class="row"><span class="spacer"></span><button onclick={() => (sort.step = 4)}>← Back to final check</button></div>{/if}</div>{/if}

  {#if report}
    <div class="card done">
      <div class="tick">✓</div>
      <div>
        <h1>{report.copied} files {settings.mode === "move" ? "moved" : "copied"} and verified</h1>
        <p class="muted">{formatBytes(report.bytes)} into <span class="mono">{settings.dest}</span></p>
      </div>
    </div>
    <div class="cards">
      {#if report.trashed}<div class="card stat"><span class="big">{report.trashed}</span>originals in the Recycle Bin</div>{/if}
      {#if report.already_present}<div class="card stat"><span class="big">{report.already_present}</span>already present</div>{/if}
      <div class="card stat"><span class="big" class:bad={report.failed.length}>{report.failed.length}</span>failed</div>
    </div>
    {#each report.warnings as w}<p class="warn">⚠ {w}</p>{/each}
    {#if report.failed.length}
      <div class="card list">
        <h3>Not copied (originals untouched)</h3>
        {#each report.failed as [p, e]}<div class="mono"><strong>{p}</strong> — {e}</div>{/each}
      </div>
    {/if}

    {#if undone}
      <div class="card list">
        <h3>Run undone</h3>
        <p>{undone.removed} copies removed{undone.restored ? `, ${undone.restored} originals restored` : ""}, {undone.dirs_removed} folders removed.</p>
        {#each undone.kept_modified as k}<div class="mono">Kept (changed since it was copied): {k}</div>{/each}
      </div>
    {/if}

    <div class="row">
      <button class="danger" disabled={app.busy || !!undone} onclick={doUndo}>Undo this run</button>
      <span class="spacer"></span>
      <button disabled={app.busy} onclick={() => { resetSort(); app.view = "home"; }}>⌂ Back to menu</button>
      <button disabled={app.busy} onclick={() => resetSort()}>Sort another folder</button>
      <button class="primary go" data-continue disabled={app.busy || !!undone} onclick={() => openFolder(settings.dest).catch((e) => (error = String(e)))}>
        Open the sorted folder
      </button>
    </div>
  {/if}
</section>

<style>
  .wrap { max-width: 820px; margin: 0 auto; padding: 40px 24px; display: grid; gap: 16px; }
  .running { padding: 28px; display: grid; gap: 14px; }
  .bar.big { height: 12px; }
  .small { font-size: 12.5px; }
  .problem { padding: 22px; display: grid; gap: 6px; border-color: var(--red); }
  .done { padding: 24px; display: flex; gap: 18px; align-items: center; }
  .tick { width: 48px; height: 48px; border-radius: 50%; display: grid; place-items: center; background: var(--green-soft); color: var(--green); font-size: 24px; font-weight: 700; }
  .cards { display: flex; gap: 12px; }
  .stat { padding: 12px 16px; display: grid; gap: 2px; min-width: 150px; }
  .big { font-size: 22px; font-weight: 650; }
  .bad { color: var(--red); }
  .warn { color: var(--amber); background: var(--amber-soft); padding: 8px 12px; border-radius: var(--radius-sm); }
  .go { padding: 10px 20px; font-size: 15px; }
  .list { padding: 16px 18px; display: grid; gap: 6px; }
</style>
