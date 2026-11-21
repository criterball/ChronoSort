<script>
  import { makePlan, formatBytes, thumbUrl, previewUrl } from "../lib/api.js";
  import { app, sort } from "../lib/state.svelte.js";

  function onStart() {
    sort.runKey += 1;
    sort.step = 5;
  }

  let error = $state("");
  let search = $state("");
  let moveConfirmed = $state(false);

  async function refresh() {
    error = "";
    app.busy = true;
    try {
      sort.plan = await makePlan(sort.settings.dest, sort.settings.months, sort.settings.naming, sort.resolutions);
    } catch (e) {
      error = String(e);
      sort.plan = null;
    } finally {
      app.busy = false;
    }
  }
  $effect(() => {
    // Re-plan whenever the month choice changes (and on first show).
    sort.settings.months;
    refresh();
  });

  function chooseMonths(choice) {
    sort.settings.months = choice;
  }

  // Preview: what happens to each file, with a picture. "renamed" = its name was
  // already taken, so _1, _2 … was added (nothing is ever overwritten).
  let show = $state("all"); // all | renamed | there
  let big = $state(null); // { hash, title } for the large picture
  let broken = $state({});
  const match = (...t) => !search || t.join(" ").toLowerCase().includes(search.toLowerCase());
  let renamedRows = $derived(sort.plan ? sort.plan.sample.filter((r) => r.taken_by) : []);
  let rows = $derived(
    !sort.plan ? [] :
    show === "there" ? sort.plan.already_there.filter((r) => match(r.src, r.existing)) :
    (show === "renamed" ? renamedRows : sort.plan.sample).filter((r) => match(r.src, r.dst)),
  );
  const nameOf = (p) => p.slice(p.lastIndexOf("/") + 1);
  const why = (r) => r.taken_by === "destination"
    ? `A different file called “${nameOf(r.src)}” is already in this folder of the destination. It stays as it is; this one gets a number added.`
    : `Another photo in this run has the same name. Both are kept; this one gets a number added.`;
  let maxPerYear = $derived(sort.plan ? Math.max(1, ...Object.values(sort.plan.per_year)) : 1);
  let canStart = $derived(sort.plan && sort.plan.ready && sort.plan.files > 0 && !app.busy && (sort.settings.mode === "copy" || moveConfirmed));
</script>

<section class="wrap">
  {#if error}<p class="problem">{error}</p>{/if}

  {#if sort.plan && !sort.plan.ready}
    <div class="card question">
      <h2>Your source is already organised into month folders</h2>
      <p class="muted">For example:</p>
      <ul class="mono">
        {#each sort.plan.month_folders.slice(0, 8) as [folder, n]}<li>{folder} <span class="muted">({n} {n === 1 ? "file" : "files"})</span></li>{/each}
        {#if sort.plan.month_folders.length > 8}<li class="muted">… and {sort.plan.month_folders.length - 8} more</li>{/if}
      </ul>
      <p>How should these be sorted?</p>
      <div class="choices">
        <button onclick={() => chooseMonths("keep")}>
          <strong>Keep the month folders</strong>
          <span class="mono muted">2005\03\photo.jpg</span>
        </button>
        <button onclick={() => chooseMonths("flatten")}>
          <strong>Put everything in the year folder</strong>
          <span class="mono muted">2005\photo.jpg</span>
        </button>
      </div>
    </div>
  {:else if sort.plan}
    <div class="row head">
      <div>
        <p class="kicker">Final check</p>
        <h1>{sort.plan.files} files · {formatBytes(sort.plan.total_bytes)}</h1>
        <p class="muted">
          will be {sort.settings.mode === "move" ? "moved" : "copied"} into {Object.keys(sort.plan.per_year).length} year folders in
          <span class="mono">{sort.settings.dest}</span>
        </p>
      </div>
      <span class="spacer"></span>
      {#if sort.settings.months !== "ask"}
        <button class="ghost small" onclick={() => chooseMonths("ask")}>Change month-folder choice</button>
      {/if}
    </div>

    <div class="cards">
      <div class="card stat"><span class="big">{sort.plan.duplicates}</span><span>duplicates not copied</span><span class="muted">{formatBytes(sort.plan.saved_bytes)} saved</span></div>
      <div class="card stat"><span class="big">{sort.plan.already_in_destination}</span><span>already in destination</span><span class="muted">same photo already there, not copied again</span></div>
      <div class="card stat"><span class="big">{sort.plan.unresolved}</span><span>left for later</span><span class="muted">no year confirmed in Check</span></div>
      <div class="card stat"><span class="big">{sort.plan.renamed}</span><span>renamed</span><span class="muted">naming scheme or <span class="mono">_1</span> for clashes</span></div>
    </div>

    <div class="card years">
      {#each Object.entries(sort.plan.per_year) as [y, n]}
        <div class="yrow"><span class="mono">{y}</span><div class="bar"><div style="width:{(100 * n) / maxPerYear}%"></div></div><span class="muted">{n}</span></div>
      {/each}
    </div>

    <div class="card preview">
      <div class="row">
        <h3>Preview</h3>
        <div class="tabs">
          <button class:on={show === "all"} onclick={() => (show = "all")}>All {sort.plan.files}</button>
          <button class:on={show === "renamed"} onclick={() => (show = "renamed")} title="Their name was already taken, so a number is added">
            Name taken {renamedRows.length}{renamedRows.length === 500 ? "+" : ""}</button>
          <button class:on={show === "there"} onclick={() => (show = "there")} title="The same photo is already in the destination; it is not copied again">
            Already there {sort.plan.already_in_destination}</button>
        </div>
        <span class="spacer"></span>
        <input type="text" placeholder="Filter…" bind:value={search} />
      </div>
      <p class="muted small">Nothing in the destination is ever overwritten or changed. Folders that already exist are used as they are. Click a picture to see it large.</p>
      <div class="list">
        {#each rows.slice(0, 500) as r (r.src)}
          <div class="pr">
            <button class="mini" onclick={() => r.hash && (big = { hash: r.hash, title: r.src })} disabled={!r.hash} title="Show large">
              {#if r.hash && !broken[r.hash]}<img src={thumbUrl(r.hash)} alt="" loading="lazy" onerror={() => (broken = { ...broken, [r.hash]: true })} />
              {:else}<span class="muted small">{nameOf(r.src).split(".").pop()?.toUpperCase()}</span>{/if}
            </button>
            <span class="src mono" title={r.src}><span dir="ltr">{r.src}</span></span>
            <span class="arrow">→</span>
            {#if show === "there"}
              <span class="dst"><span class="chip">already there</span> <span class="mono">{r.existing}</span></span>
            {:else}
              <span class="dst"><span class="mono">{r.dst}</span>
                {#if r.taken_by}<span class="chip amber" title={why(r)}>{r.taken_by === "destination" ? "name taken in destination" : "same name as another photo"}</span>{/if}
              </span>
            {/if}
          </div>
        {:else}
          <p class="muted">{show === "there" ? "Nothing from this source is in the destination yet." : show === "renamed" ? "No name clashes: every file keeps its name." : "No files match."}</p>
        {/each}
        {#if show === "all" && sort.plan.sample.length < sort.plan.files}
          <p class="muted small">Showing {sort.plan.sample.length} of {sort.plan.files} files (files whose name was taken come first).</p>
        {/if}
      </div>
      {#each sort.plan.notes as n}<p class="muted note">Note: {n}</p>{/each}
    </div>

    <div class="row foot">
      {#if sort.settings.mode === "move"}
        <label class="confirm"><input type="checkbox" bind:checked={moveConfirmed} /> I understand: originals go to the Recycle Bin after each verified copy.</label>
      {:else}
        <span class="muted">Every copy is verified byte-for-byte, keeps its dates, and can be undone.</span>
      {/if}
      <span class="spacer"></span>
      <button onclick={() => (sort.step = 3)}>← File names</button>
      <button class="primary go" data-continue disabled={!canStart} onclick={onStart}>
        Start {sort.settings.mode === "move" ? "moving" : "copying"}
      </button>
    </div>
  {:else if !error}
    <div class="center planning">
      <div class="bar indeterminate"><div></div></div>
      <p class="muted">Planning … checking what is already in the destination folder. A big destination can take a few minutes.</p>
    </div>
  {/if}
</section>

{#if big}
  <dialog class="bigview" open onclose={() => (big = null)} onclick={(e) => e.target === e.currentTarget && (big = null)}>
    <img src={previewUrl(big.hash)} alt={big.title} />
    <div class="row"><span class="mono small">{big.title}</span><span class="spacer"></span><button onclick={() => (big = null)}>Close (Esc)</button></div>
  </dialog>
{/if}

<svelte:window onkeydown={(e) => big && e.key === "Escape" && ((big = null), e.preventDefault())} />

<style>
  .wrap { max-width: 1000px; margin: 0 auto; padding: 28px 24px 48px; display: grid; gap: 18px; }
  .problem { color: var(--red); background: var(--red-soft); padding: 10px 14px; border-radius: var(--radius-sm); }
  .question { padding: 26px; display: grid; gap: 12px; }
  .question ul { margin: 0; padding-left: 18px; }
  .choices { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin-top: 6px; }
  .choices button { display: grid; gap: 4px; text-align: left; padding: 16px; }
  .choices button:hover { border-color: var(--accent); background: var(--accent-soft); }
  .cards { display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; }
  .stat { padding: 14px 16px; display: grid; gap: 2px; }
  .big { font-size: 24px; font-weight: 650; }
  .years { padding: 14px 18px; display: grid; gap: 6px; max-height: 260px; overflow: auto; }
  .yrow { display: grid; grid-template-columns: 48px 1fr 56px; gap: 10px; align-items: center; }
  .preview { padding: 14px 18px; display: grid; gap: 10px; }
  .list { max-height: 420px; overflow: auto; display: grid; gap: 2px; }
  .pr { display: grid; grid-template-columns: 44px 1fr 20px 1fr; gap: 8px; align-items: center; padding: 3px 0; border-bottom: 1px dashed var(--border); }
  /* Ellipsis at the *start* of long source paths, without reordering the text. */
  .src { color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; }
  .src > span { unicode-bidi: isolate; }
  .arrow { color: var(--muted); text-align: center; }
  .dst { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .note { font-size: 12.5px; }
  .foot { padding-top: 4px; }
  .confirm { display: flex; gap: 8px; align-items: center; color: var(--amber); }
  .kicker { color: var(--accent); font-weight: 650; text-transform: uppercase; font-size: 12px; letter-spacing: 0.06em; }
  .go { font-size: 15px; padding: 10px 20px; }
  .center { text-align: center; padding: 40px; }
  .tabs { display: flex; background: var(--surface-2); border-radius: var(--radius-sm); padding: 3px; margin-left: 12px; }
  .tabs button { border: none; background: transparent; padding: 4px 10px; color: var(--muted); font-size: 13px; }
  .tabs button.on { background: var(--surface); color: var(--text); box-shadow: var(--shadow); }
  .small { font-size: 12.5px; }
  .mini { padding: 0; width: 44px; height: 44px; border-radius: 6px; overflow: hidden; background: var(--surface-2); display: grid; place-items: center; flex: none; }
  .mini img { width: 100%; height: 100%; object-fit: cover; }
  .planning { display: grid; gap: 10px; max-width: 520px; margin: 40px auto; }
  .bigview { padding: 0; border: none; border-radius: var(--radius); background: var(--surface); max-width: 92vw; max-height: 92vh; }
  .bigview img { display: block; max-width: 90vw; max-height: 80vh; object-fit: contain; }
  .bigview .row { padding: 8px 12px; }
</style>
