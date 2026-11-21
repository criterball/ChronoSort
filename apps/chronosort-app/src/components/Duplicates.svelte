<script>
  import {
    pickFolder, pickSaveFile, confirmBox, findDuplicates, resolveDuplicates, saveDuplicateReport,
    undoDuplicates, openFolder, showDuplicate, onProgress, thumbUrl, previewUrl, formatBytes, plural,
    baseName, parentDir, joinPath,
  } from "../lib/api.js";
  import { app, dups, resetDups } from "../lib/state.svelte.js";

  let { active } = $props();

  let progress = $state(null);
  let error = $state("");
  let shown = $state(60); // sets listed so far (big scans can have thousands)
  let broken = $state({}); // small thumbnails that failed
  let bigBroken = $state({}); // large previews that failed
  let skip = $state({}); // group hash -> true: leave this set alone
  let saved = $state("");
  let sel = $state(0); // selected set, shown in the preview pane
  let typed = $state("");

  let cur = $derived(dups.scan?.groups[sel] ?? null);

  const defaultTarget = () =>
    dups.folders.length ? joinPath(parentDir(dups.folders[0]), baseName(dups.folders[0]) + " duplicates") : "";

  async function addFolder() {
    const p = await pickFolder("Folder to check for duplicates");
    if (p && !dups.folders.includes(p)) dups.folders = [...dups.folders, p];
  }
  function addTyped() {
    const p = typed.trim();
    if (p && !dups.folders.includes(p)) dups.folders = [...dups.folders, p];
    typed = "";
  }
  const removeFolder = (p) => (dups.folders = dups.folders.filter((f) => f !== p));

  async function scan() {
    error = "";
    app.busy = true;
    dups.phase = "scanning";
    progress = { done: 0, total: 0 };
    const stop = await onProgress("dup-progress", (p) => (progress = p));
    try {
      const s = await findDuplicates(dups.folders, dups.includeAll);
      dups.scan = s;
      dups.keep = Object.fromEntries(s.groups.map((g) => [g.hash, g.keep]));
      dups.target = defaultTarget();
      dups.phase = "results";
      skip = {};
      shown = 60;
      sel = 0;
    } catch (e) {
      error = String(e);
      dups.phase = "pick";
    } finally {
      stop();
      app.busy = false;
      progress = null;
    }
  }

  let chosen = $derived(dups.scan ? dups.scan.groups.filter((g) => !skip[g.hash]) : []);
  let extras = $derived(chosen.reduce((n, g) => n + g.files.length - 1, 0));
  let freed = $derived(chosen.reduce((n, g) => n + g.size * (g.files.length - 1), 0));

  // Keyboard on the results: arrows move between sets, 1-9 keep that copy, Space includes/leaves alone.
  function onKey(e) {
    if (!active || dups.phase !== "results" || !dups.scan?.groups.length) return;
    const t = e.target;
    if (t instanceof HTMLInputElement && t.type === "text") return;
    if (e.ctrlKey || e.altKey || e.metaKey) return;
    const n = Math.min(shown, dups.scan.groups.length);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      sel = Math.max(0, Math.min(n - 1, sel + (e.key === "ArrowDown" ? 1 : -1)));
      document.querySelector(`.grow[data-gi="${sel}"]`)?.scrollIntoView({ block: "nearest" });
    } else if (/^[1-9]$/.test(e.key) && cur && Number(e.key) <= cur.files.length && !skip[cur.hash]) {
      dups.keep[cur.hash] = Number(e.key) - 1;
    } else if (e.key === " " && cur && (t === document.body || t.classList?.contains("grow"))) {
      skip = { ...skip, [cur.hash]: !skip[cur.hash] };
    } else return;
    e.preventDefault();
  }

  async function browseTarget() {
    const p = await pickFolder("Where should the extra copies go?");
    if (p) dups.target = p;
  }

  async function act() {
    error = "";
    saved = "";
    if (dups.action === "report") {
      const p = await pickSaveFile("Save the duplicate list", "duplicates.csv");
      if (!p) return;
      try {
        await saveDuplicateReport(p);
        saved = p;
      } catch (e) {
        error = String(e);
      }
      return;
    }
    if (dups.action === "trash") {
      const ok = await confirmBox(
        `Put ${plural(extras, "extra copy", "extra copies")} in the Recycle Bin?\n\nThe copy you chose to keep is never touched. You can get files back from the Recycle Bin.`,
        "Recycle Bin", "Put in Recycle Bin", "Cancel",
      );
      if (!ok) return;
    }
    const keep = Object.fromEntries(chosen.map((g) => [g.hash, dups.keep[g.hash]]));
    app.busy = true;
    dups.phase = "working";
    try {
      dups.report = await resolveDuplicates(keep, dups.action, dups.action === "move" ? dups.target : null);
      dups.phase = "done";
    } catch (e) {
      error = String(e);
      dups.phase = "results";
    } finally {
      app.busy = false;
    }
  }

  async function undoMove() {
    const ok = await confirmBox("Put the moved copies back where they were?", "Undo", "Undo", "Cancel");
    if (!ok) return;
    app.busy = true;
    try {
      dups.undone = await undoDuplicates(dups.target);
    } catch (e) {
      error = String(e);
    } finally {
      app.busy = false;
    }
  }

  function reveal(e, hash, i) {
    e.preventDefault(); // the button sits inside the copy's <label>
    showDuplicate(hash, i).catch((x) => (error = String(x)));
  }

  const fmtDate = (t) => (t == null ? "unknown" : new Date(t * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }));
  const nameOf = (rel) => rel.slice(rel.lastIndexOf("/") + 1);
</script>

<svelte:window onkeydown={onKey} />

<section class="wrap">
  {#if dups.phase === "pick" || dups.phase === "scanning"}
    <div class="head">
      <h1>Find duplicates</h1>
      <p class="muted">Finds files that are exactly the same, byte for byte, even if their names differ. Checking only reads files.</p>
    </div>

    <div class="card form">
      <span class="label">Folders to check</span>
      {#if dups.folders.length === 0}
        <p class="muted">No folder chosen yet.</p>
      {/if}
      {#each dups.folders as f}
        <div class="row folder">
          <span class="mono">{f}</span>
          <span class="spacer"></span>
          <button class="ghost small" disabled={app.busy} onclick={() => removeFolder(f)}>Remove</button>
        </div>
      {/each}
      <div class="row">
        <button disabled={app.busy} onclick={addFolder}>+ Add a folder…</button>
        <input id="dupdir" type="text" class="typed" placeholder="or type / paste a folder path and press Enter" bind:value={typed}
          disabled={app.busy} onkeydown={(e) => e.key === "Enter" && addTyped()} />
        <button disabled={app.busy || !typed.trim()} onclick={addTyped}>Add</button>
      </div>
      <span class="muted small">You can add several, e.g. an old backup and your picture folder.</span>
      <label class="check">
        <input type="checkbox" bind:checked={dups.includeAll} disabled={app.busy} />
        Also check other file types (documents, music …), not only photos and videos
      </label>

      {#if error}<p class="problem">{error}</p>{/if}

      <div class="row actions">
        {#if progress}
          <div class="progress">
            <div class="bar" class:indeterminate={!progress.total}><div style="width: {progress.total ? (100 * progress.done) / progress.total : 3}%"></div></div>
            <span class="muted">Checked {progress.done} of {progress.total || "…"} files</span>
          </div>
        {/if}
        <span class="spacer"></span>
        <button class="primary go" data-continue disabled={!dups.folders.length || app.busy} onclick={scan}>Find duplicates</button>
      </div>
    </div>
  {:else if dups.phase === "results" || dups.phase === "working"}
    {@const s = dups.scan}
    {#if s.groups.length === 0}
      <div class="card empty">
        <div class="tick">✓</div>
        <h2>No duplicates found</h2>
        <p class="muted">{plural(s.files_scanned, "file", "files")} checked.</p>
        <div class="row center"><button class="primary go" data-continue onclick={resetDups}>Check other folders</button></div>
      </div>
    {:else}
      <div class="row head">
        <div>
          <h1>{plural(s.groups.length, "file has", "files have")} identical copies</h1>
          <p class="muted">{plural(s.files_scanned, "file", "files")} checked · {formatBytes(s.bytes_reclaimable)} could be freed. The suggested copy to keep is already selected.</p>
        </div>
        <span class="spacer"></span>
        <button class="ghost" disabled={app.busy} onclick={resetDups}>← Start over</button>
      </div>

      <div class="split">
        <div class="glist" role="listbox" aria-label="Duplicate sets">
          {#each s.groups.slice(0, shown) as g, gi (g.hash)}
            <button class="grow" class:sel={sel === gi} class:off={skip[g.hash]} role="option" aria-selected={sel === gi}
              data-gi={gi} onclick={() => (sel = gi)}>
              <span class="mini">
                {#if (g.files[0].kind !== "image" && g.files[0].kind !== "video") || broken[g.hash]}
                  <span class="placeholder">{g.files[0].kind === "video" ? "▶" : g.files[0].format.toUpperCase()}</span>
                {:else}
                  <img src={thumbUrl(g.hash)} alt="" loading="lazy" onerror={() => (broken = { ...broken, [g.hash]: true })} />
                {/if}
              </span>
              <span class="gtxt">
                <strong>{nameOf(g.files[dups.keep[g.hash]].rel)}</strong>
                <span class="muted">{plural(g.files.length, "copy", "copies")} · {formatBytes(g.size)}{skip[g.hash] ? " · left alone" : ""}</span>
              </span>
            </button>
          {/each}
          {#if shown < s.groups.length}
            <button class="more" onclick={() => (shown += 100)}>Show more ({s.groups.length - shown} left)</button>
          {/if}
        </div>

        {#if cur}
          <div class="card pane">
            <div class="big">
              {#if (cur.files[0].kind !== "image" && cur.files[0].kind !== "video") || bigBroken[cur.hash]}
                <div class="placeholder">
                  {cur.files[0].kind === "video" ? "▶ Video: no preview (get FFmpeg in Settings → AI models). Use “Show in Explorer” to play it." : "No preview for this file type. Use “Show in Explorer” to open it."}
                </div>
              {:else}
                {#key cur.hash}
                  <img src={previewUrl(cur.hash)} alt={nameOf(cur.files[0].rel)} onerror={() => (bigBroken = { ...bigBroken, [cur.hash]: true })} />
                {/key}
              {/if}
            </div>
            <div class="pinfo">
              <div class="row">
                <strong>Set {sel + 1} of {s.groups.length}</strong>
                <span class="muted">{formatBytes(cur.size)} each</span>
                <span class="spacer"></span>
                <label class="check small">
                  <input type="checkbox" checked={!skip[cur.hash]} onchange={(e) => (skip = { ...skip, [cur.hash]: !e.currentTarget.checked })} />
                  include this set
                </label>
              </div>
              <p class="muted small">These copies are the same picture, byte for byte. Choose <em>where</em> to keep it:</p>
              {#each cur.files as f, i}
                <label class="copy" class:kept={dups.keep[cur.hash] === i} class:off={skip[cur.hash]}>
                  <input type="radio" name="keep-{cur.hash}" checked={dups.keep[cur.hash] === i} disabled={skip[cur.hash]}
                    onchange={() => (dups.keep[cur.hash] = i)} />
                  <kbd>{i + 1}</kbd>
                  <span class="where">
                    <span class="fname">{nameOf(f.rel)}</span>
                    <span class="mono dir" title={f.abs}><span dir="ltr">{parentDir(f.abs)}</span></span>
                    <span class="muted small">changed {fmtDate(f.mtime)}{i === cur.keep ? " · suggested" : ""}</span>
                  </span>
                  {#if skip[cur.hash]}<span class="chip">left alone</span>
                  {:else if dups.keep[cur.hash] === i}<span class="chip green">keep</span>
                  {:else}<span class="chip amber">extra</span>{/if}
                  <button class="small" onclick={(e) => reveal(e, cur.hash, i)}>Show in Explorer</button>
                </label>
              {/each}
              <p class="muted small keys"><kbd>↑</kbd> <kbd>↓</kbd> previous / next set · <kbd>1</kbd>–<kbd>9</kbd> keep that copy · <kbd>Space</kbd> include or leave alone</p>
            </div>
          </div>
        {/if}
      </div>

      <div class="card decide">
        <h2>What should happen to the {plural(extras, "extra copy", "extra copies")}?</h2>
        <label class="opt" class:on={dups.action === "move"}>
          <input type="radio" bind:group={dups.action} value="move" />
          <div>
            <strong>Move them to a "duplicates" folder</strong> <span class="chip green">recommended · can be undone</span>
            <div class="row target">
              <input type="text" class="mono" bind:value={dups.target} disabled={dups.action !== "move"} />
              <button disabled={dups.action !== "move"} onclick={browseTarget}>Browse…</button>
            </div>
          </div>
        </label>
        <label class="opt" class:on={dups.action === "trash"}>
          <input type="radio" bind:group={dups.action} value="trash" />
          <div><strong>Put them in the Recycle Bin</strong> <span class="muted">frees {formatBytes(freed)}</span></div>
        </label>
        <label class="opt" class:on={dups.action === "report"}>
          <input type="radio" bind:group={dups.action} value="report" />
          <div><strong>Change nothing, just save a list</strong> <span class="muted">a CSV file that opens in Excel</span></div>
        </label>
        {#if error}<p class="problem">{error}</p>{/if}
        {#if saved}<p class="good">✓ List saved to <span class="mono">{saved}</span></p>{/if}
        <div class="row">
          <span class="muted">Before anything moves, each file is checked again. The copy you keep is never touched.</span>
          <span class="spacer"></span>
          <button class="primary go" data-continue
            disabled={app.busy || (dups.action !== "report" && !extras) || (dups.action === "move" && !dups.target.trim())}
            onclick={act}>
            {dups.phase === "working" ? "Working…" : dups.action === "move" ? `Move ${plural(extras, "copy", "copies")}` : dups.action === "trash" ? `Recycle ${plural(extras, "copy", "copies")}` : "Save list…"}
          </button>
        </div>
      </div>
    {/if}
  {:else if dups.phase === "done"}
    {@const r = dups.report}
    <div class="card done">
      <div class="tick">✓</div>
      <div>
        <h1>{r.moved ? `${plural(r.moved, "extra copy", "extra copies")} moved` : `${plural(r.trashed, "extra copy", "extra copies")} put in the Recycle Bin`}</h1>
        <p class="muted">{formatBytes(r.bytes_freed)} freed in the checked folders{#if r.moved} · now in <span class="mono">{dups.target}</span>{/if}</p>
      </div>
    </div>
    {#if r.skipped.length}
      <div class="card list">
        <h3>Left alone ({r.skipped.length})</h3>
        {#each r.skipped as [p, why]}<div class="mono small"><strong>{p}</strong> — {why}</div>{/each}
      </div>
    {/if}
    {#if dups.undone}
      <div class="card list"><h3>Undone</h3><p>{dups.undone.removed + dups.undone.restored} copies put back.</p>
        {#each dups.undone.failed as [p, e]}<div class="mono small">{p} — {e}</div>{/each}</div>
    {/if}
    {#if error}<p class="problem">{error}</p>{/if}
    <div class="row">
      {#if r.moved}<button class="danger" disabled={app.busy || !!dups.undone} onclick={undoMove}>Undo: put them back</button>{/if}
      <span class="spacer"></span>
      <button disabled={app.busy} onclick={() => { resetDups(); app.view = "home"; }}>⌂ Back to menu</button>
      <button disabled={app.busy} onclick={resetDups}>Check other folders</button>
      {#if r.moved && !dups.undone}
        <button class="primary go" data-continue onclick={() => openFolder(dups.target).catch((e) => (error = String(e)))}>Open the duplicates folder</button>
      {/if}
    </div>
  {/if}
</section>

<style>
  .wrap { max-width: 1240px; margin: 0 auto; padding: 24px 24px 48px; display: grid; gap: 16px; }
  .head { display: grid; gap: 4px; }
  .row.head { display: flex; }
  .form { padding: 20px 24px; display: grid; gap: 12px; }
  .label { font-weight: 600; font-size: 15px; }
  .folder { padding: 8px 12px; background: var(--surface-2); border-radius: var(--radius-sm); }
  .typed { flex: 1; }
  .check { display: flex; gap: 8px; align-items: center; cursor: pointer; }
  .check input, .opt input, .copy input { accent-color: var(--accent); }
  .small { font-size: 12.5px; }
  .actions { border-top: 1px solid var(--border); padding-top: 14px; }
  .progress { display: grid; gap: 6px; flex: 1; max-width: 420px; }
  .problem { color: var(--red); background: var(--red-soft); padding: 8px 12px; border-radius: var(--radius-sm); }
  .good { color: var(--green); font-weight: 600; }
  .go { font-size: 15px; padding: 10px 20px; }
  .empty { padding: 36px; display: grid; gap: 8px; justify-items: center; text-align: center; }
  .center { justify-content: center; }

  .split { display: grid; grid-template-columns: 300px 1fr; gap: 14px; align-items: start; }
  .glist { display: grid; gap: 4px; max-height: 72vh; overflow: auto; padding-right: 4px; }
  .grow {
    display: grid; grid-template-columns: 52px 1fr; gap: 10px; align-items: center; text-align: left;
    padding: 6px 8px; border-color: transparent; background: transparent;
  }
  .grow:hover:not(:disabled) { background: var(--surface-2); border-color: transparent; }
  .grow.sel, .grow.sel:hover:not(:disabled) { background: var(--accent-soft); border-color: var(--accent); }
  .grow.off { opacity: 0.5; }
  .mini { width: 52px; height: 52px; border-radius: 6px; overflow: hidden; background: var(--surface-2); display: grid; place-items: center; }
  .mini img { width: 100%; height: 100%; object-fit: cover; }
  .gtxt { display: grid; min-width: 0; }
  .gtxt strong, .gtxt span { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .gtxt span { font-size: 12.5px; }
  .more { justify-self: center; margin-top: 6px; }

  .pane { position: sticky; top: 8px; overflow: hidden; display: grid; }
  .big { background: var(--surface-2); height: 46vh; min-height: 240px; display: grid; place-items: center; }
  .big img { max-width: 100%; max-height: 46vh; object-fit: contain; }
  .placeholder { color: var(--muted); font-weight: 600; padding: 16px; text-align: center; }
  .pinfo { padding: 12px 16px 14px; display: grid; gap: 6px; }
  .copy {
    display: grid; grid-template-columns: auto auto 1fr auto auto; gap: 10px; align-items: center;
    padding: 8px 10px; border: 1px solid var(--border); border-radius: var(--radius-sm); cursor: pointer;
  }
  .copy.kept { background: var(--green-soft); border-color: var(--green); }
  .copy.off { opacity: 0.6; }
  .where { display: grid; min-width: 0; }
  .fname { font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* Ellipsis at the start of long folder paths so the end (the folder name) stays visible. */
  .dir { font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; }
  .dir > span { unicode-bidi: isolate; }
  kbd { font: 12px var(--mono); border: 1px solid var(--border); border-bottom-width: 2px; border-radius: 4px; padding: 0 6px; background: var(--surface); }
  .keys { margin-top: 2px; }

  .decide { padding: 18px 22px; display: grid; gap: 10px; }
  .opt { display: grid; grid-template-columns: auto 1fr; gap: 12px; align-items: start; padding: 10px 12px; border: 1px solid var(--border); border-radius: var(--radius-sm); cursor: pointer; }
  .opt.on { border-color: var(--accent); background: var(--accent-soft); }
  .opt input[type="radio"] { margin-top: 4px; }
  .target { margin-top: 8px; }
  .target input { flex: 1; }
  .done { padding: 24px; display: flex; gap: 18px; align-items: center; }
  .tick { width: 48px; height: 48px; border-radius: 50%; display: grid; place-items: center; background: var(--green-soft); color: var(--green); font-size: 24px; font-weight: 700; flex: none; }
  .list { padding: 16px 18px; display: grid; gap: 6px; max-height: 300px; overflow: auto; }
</style>
