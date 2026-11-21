<script>
  import { sort, people, app } from "../lib/state.svelte.js";
  import { plural } from "../lib/api.js";

  const a = $derived(sort.analysis);
  let autoTotal = $derived(Object.values(a.auto_by_year).reduce((x, y) => x + y, 0));
  let years = $derived(Object.entries(a.auto_by_year));
  let maxYear = $derived(Math.max(1, ...years.map(([, n]) => n)));
  let span = $derived(years.length ? `${years[0][0]} – ${years[years.length - 1][0]}` : "");
  let showErrors = $state(false);

  function findFaces() {
    people.folder = sort.settings.source;
    people.autostart = true;
    app.view = "people";
  }

  function next() {
    sort.step = a.review.length ? 2 : 3;
  }
</script>

<section class="wrap">
  <div class="head">
    <h1>Here is what we found</h1>
    <p class="muted mono">{sort.settings.source}</p>
  </div>

  <div class="cards">
    <div class="card stat"><span class="big">{a.total}</span><span>files found</span><span class="muted">photos, videos and sidecars</span></div>
    <div class="card stat good"><span class="big">{autoTotal}</span><span>dated with confidence</span><span class="muted">sorted automatically</span></div>
    <div class="card stat" class:warn={a.review.length}><span class="big">{a.review.length}</span><span>need your check</span><span class="muted">the date is uncertain</span></div>
    <div class="card stat"><span class="big">{a.duplicates}</span><span>exact duplicates</span><span class="muted">copied only once</span></div>
  </div>

  {#if years.length}
    <div class="card chart">
      <div class="row">
        <h3>Photos and videos per year</h3>
        <span class="muted">{span}</span>
      </div>
      <div class="years">
        {#each years as [y, n]}
          <div class="yr" title="{plural(n, 'file', 'files')} in {y}">
            <span class="n">{n}</span>
            <div class="col" style="height: {6 + (110 * n) / maxYear}px"></div>
            <span>{y}</span>
          </div>
        {/each}
      </div>
    </div>
  {/if}

  <div class="facts">
    <span class="chip">{plural(a.junk, "system file", "system files")} skipped (thumbs.db, .DS_Store …)</span>
    {#if a.errors.length}
      <button class="chip red linkish" onclick={() => (showErrors = !showErrors)}>
        {plural(a.errors.length, "file could not be read", "files could not be read")} {showErrors ? "▲" : "▼"}
      </button>
    {/if}
  </div>
  {#if showErrors}
    <div class="card errs mono">{#each a.errors as e}<div>{e}</div>{/each}</div>
  {/if}

  <div class="row foot">
    <span class="muted">
      {#if a.review.length}Next: look at the {plural(a.review.length, "file", "files")} with an uncertain date.{:else}Every file has a reliable date.{/if}
    </span>
    <span class="spacer"></span>
    <button onclick={() => (sort.step = 0)}>← Folders</button>
    <button onclick={findFaces} title="Opens People and looks for faces in this folder; the photos' years are remembered with each face">Find the faces in these photos</button>
    <button class="primary big" data-continue onclick={next}>
      {a.review.length ? `Check ${plural(a.review.length, "file", "files")} →` : "Continue to file names →"}
    </button>
  </div>
</section>

<style>
  .wrap { max-width: 1000px; margin: 0 auto; padding: 28px 24px 48px; display: grid; gap: 18px; }
  .head { display: grid; gap: 4px; }
  .cards { display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px; }
  .stat { padding: 16px 18px; display: grid; gap: 2px; }
  .stat.good .big { color: var(--green); }
  .stat.warn .big { color: var(--amber); }
  .big { font-size: 28px; font-weight: 650; }
  .chart { padding: 16px 20px; display: grid; gap: 12px; }
  .years { display: flex; gap: 6px; align-items: flex-end; overflow-x: auto; padding-bottom: 4px; }
  .yr { display: grid; justify-items: center; gap: 4px; font-size: 11.5px; color: var(--muted); min-width: 34px; }
  .yr .n { font-size: 11px; }
  .col { width: 22px; border-radius: 4px 4px 2px 2px; background: var(--accent); opacity: 0.85; }
  .facts { display: flex; gap: 8px; flex-wrap: wrap; }
  .linkish { border: none; cursor: pointer; }
  .errs { padding: 12px 16px; max-height: 200px; overflow: auto; display: grid; gap: 3px; }
  .foot { padding-top: 6px; }
  .big.primary { font-size: 15px; padding: 10px 20px; }
</style>
