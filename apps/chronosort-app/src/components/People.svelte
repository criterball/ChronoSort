<script>
  import { onMount } from "svelte";
  import {
    pickFolder, confirmBox, findFaces, suggestPerson, savePerson, listPeople, renamePerson, forgetPerson,
    mergePeople, showFacePhoto, onProgress, faceUrl, personUrl, plural, forgetEveryone, setBirthYear,
  } from "../lib/api.js";
  import { app, people, resetPeople } from "../lib/state.svelte.js";

  let { active, go } = $props();

  let progress = $state(null);
  let error = $state("");
  let notice = $state("");
  let open = $state({}); // group id -> show all faces
  let chosen = $state({}); // "group id:face" -> selected face (to split off)
  let renaming = $state(null); // { id, name }
  let mergeFrom = $state({}); // person id -> target person id (select value)
  let nextId = 1;
  const STRIP = 16;

  onMount(refreshKnown);

  // Opened from the sort flow ("Find the faces in these photos"): start right away.
  $effect(() => {
    if (active && people.autostart && !app.busy) {
      people.autostart = false;
      resetPeople();
      scan();
    }
  });

  async function forgetAll() {
    const ok = await confirmBox(
      `Forget all ${plural(people.known.length, "person", "people")}?\n\nEvery name and every remembered face is deleted from ChronoSort. Your photos are not touched. This cannot be undone.`,
      "Forget everyone", "Forget everyone", "Cancel",
    );
    if (!ok) return;
    try { people.known = await forgetEveryone(); notice = "Everyone was forgotten."; } catch (e) { error = String(e); }
  }
  async function refreshKnown() {
    try { people.known = await listPeople(); } catch (e) { error = String(e); }
  }

  async function browse() {
    const p = await pickFolder("Folder with photos of people");
    if (p) people.folder = p;
  }

  // A suggestion this alike is a confident one; weaker ones (seen: wrong people in
  // group photos at 40 %) are only offered as "might be", without filling the name in.
  const SURE = 0.5;
  const sure = (sg) => sg && sg.similarity >= SURE;
  const mkGroup = (faces, suggestion = null) =>
    ({ id: nextId++, faces, suggestion, name: sure(suggestion) ? suggestion.name : "", saved: null, pick: false });

  async function scan() {
    error = notice = "";
    app.busy = true;
    people.phase = "scanning";
    progress = { done: 0, total: 0 };
    const stop = await onProgress("faces-progress", (p) => (progress = p));
    try {
      const r = await findFaces(people.folder);
      people.result = r;
      people.groups = r.groups.map((g) => mkGroup(g.faces, g.suggestion));
      people.phase = "results";
      open = {}; chosen = {};
    } catch (e) {
      error = String(e);
      people.phase = "pick";
    } finally {
      stop();
      app.busy = false;
      progress = null;
    }
  }

  let multi = $derived(people.groups.filter((g) => g.faces.length > 1));
  let singles = $derived(people.groups.filter((g) => g.faces.length === 1));
  let picked = $derived(people.groups.filter((g) => g.pick));
  let knownNames = $derived(people.known.map((p) => p.name));

  function yearsOf(faces) {
    const ys = [...new Set(faces.map((i) => people.result.faces[i].year).filter((y) => y != null))].sort();
    if (!ys.length) return "";
    return ys.length === 1 ? `${ys[0]}` : `${ys[0]}–${ys[ys.length - 1]}`;
  }
  const photosOf = (faces) => new Set(faces.map((i) => people.result.faces[i].rel)).size;

  async function resuggest(g) {
    try {
      g.suggestion = await suggestPerson(g.faces);
      if (!g.name && sure(g.suggestion)) g.name = g.suggestion.name;
    } catch { /* keep the old suggestion */ }
  }

  async function save(g, personId = null) {
    error = notice = "";
    const name = g.name.trim();
    if (!personId && !name) { error = "Please type a name first."; return; }
    if (g.saved && g.saved === (personId ? g.suggestion?.name : name)) return; // already remembered
    try {
      people.known = await savePerson(g.faces, personId ? null : name, personId);
      g.saved = personId ? g.suggestion.name : name;
      g.pick = false;
      notice = `✓ ${plural(g.faces.length, "face", "faces")} remembered as ${g.saved}.`;
    } catch (e) {
      error = String(e);
    }
  }

  // Selected faces of a group leave it and become a group of their own.
  function splitOff(g) {
    const sel = g.faces.filter((i) => chosen[`${g.id}:${i}`]);
    if (!sel.length || sel.length === g.faces.length) return;
    g.faces = g.faces.filter((i) => !sel.includes(i));
    const n = mkGroup(sel);
    const at = people.groups.indexOf(g);
    people.groups.splice(at + 1, 0, n);
    for (const i of sel) delete chosen[`${g.id}:${i}`];
    chosen = { ...chosen };
    resuggest(g);
    resuggest(people.groups[at + 1]);
  }

  function mergePicked() {
    const [first, ...rest] = picked;
    if (!first || !rest.length) return;
    first.faces = [...first.faces, ...rest.flatMap((g) => g.faces)];
    first.saved = null;
    first.pick = false;
    people.groups = people.groups.filter((g) => !rest.includes(g));
    resuggest(first);
  }

  const toggleFace = (g, i) => (chosen = { ...chosen, [`${g.id}:${i}`]: !chosen[`${g.id}:${i}`] });
  const selCount = (g) => g.faces.filter((i) => chosen[`${g.id}:${i}`]).length;
  function reveal(i) {
    showFacePhoto(i).catch((e) => (error = String(e)));
  }

  async function saveBorn(p, v) {
    const y = String(v).trim() === "" ? null : Number(v);
    if (y !== null && !Number.isInteger(y)) { error = "Please type a year, e.g. 1985."; return; }
    try { people.known = await setBirthYear(p.id, y); error = ""; } catch (e) { error = String(e); }
  }

  async function doRename() {
    try {
      people.known = await renamePerson(renaming.id, renaming.name);
      renaming = null;
    } catch (e) { error = String(e); }
  }
  async function forget(p) {
    const ok = await confirmBox(
      `Forget ${p.name}?\n\nThe name and the ${plural(p.samples, "remembered face", "remembered faces")} are deleted from ChronoSort. Your photos are not touched.`,
      "Forget a person", "Forget", "Cancel",
    );
    if (!ok) return;
    try { people.known = await forgetPerson(p.id); } catch (e) { error = String(e); }
  }
  async function merge(p) {
    const target = people.known.find((k) => k.id === Number(mergeFrom[p.id]));
    if (!target) return;
    const ok = await confirmBox(`${p.name} and ${target.name} are the same person?\n\nAll faces of ${p.name} move to ${target.name}.`, "Same person", "Yes, merge", "Cancel");
    if (!ok) return;
    try {
      people.known = await mergePeople(target.id, p.id);
      delete mergeFrom[p.id];
    } catch (e) { error = String(e); }
  }
  const yearSpan = (ys) => (!ys.length ? "" : ys.length === 1 ? ` · ${ys[0]}` : ` · ${ys[0]}–${ys[ys.length - 1]}`);
  const pct = (s) => Math.round(s * 100);
</script>

<section class="wrap">
  <div class="head">
    <h1>People</h1>
    <p class="muted">ChronoSort finds faces in your photos and groups the ones that look alike. Give a group a name and ChronoSort will recognise that person in other photos too. All of this happens on this PC.</p>
  </div>

  {#if error}<p class="problem">{error}</p>{/if}
  {#if notice}<p class="good">{notice}</p>{/if}

  {#if people.known.length}
    <div class="card known">
      <div class="row">
        <h2>People you have named ({people.known.length})</h2>
        <span class="spacer"></span>
        <button class="ghost small" onclick={forgetAll} title="Delete all names and remembered faces">Forget everyone…</button>
      </div>
      <div class="kgrid">
        {#each people.known as p (p.id)}
          <div class="person">
            <img class="cover" src={personUrl(p.id, p.samples)} alt="" />
            <div class="pinfo">
              {#if renaming?.id === p.id}
                <div class="row">
                  <input type="text" bind:value={renaming.name} maxlength="60" onkeydown={(e) => e.key === "Enter" && doRename()} />
                  <button class="small primary" onclick={doRename}>OK</button>
                  <button class="small ghost" onclick={() => (renaming = null)}>Cancel</button>
                </div>
              {:else}
                <strong>{p.name}</strong>
              {/if}
              <span class="muted small">{plural(p.samples, "face", "faces")}{yearSpan(p.years)}</span>
              <label class="born small muted" title="Optional. Photos with this person can then not be dated before this year.">Born
                <input type="number" min="1850" max="2100" placeholder="year" value={p.birth_year ?? ""}
                  onchange={(e) => saveBorn(p, e.currentTarget.value)} /></label>
              <div class="row acts">
                <button class="small ghost" onclick={() => (renaming = { id: p.id, name: p.name })}>Rename</button>
                <button class="small ghost" onclick={() => forget(p)}>Forget</button>
                {#if people.known.length > 1}
                  <select class="small" bind:value={mergeFrom[p.id]} title="This is the same person as …">
                    <option value={undefined}>Same person as…</option>
                    {#each people.known.filter((k) => k.id !== p.id) as k}<option value={k.id}>{k.name}</option>{/each}
                  </select>
                  {#if mergeFrom[p.id]}<button class="small" onclick={() => merge(p)}>Merge</button>{/if}
                {/if}
              </div>
            </div>
          </div>
        {/each}
      </div>
    </div>
  {/if}

  {#if people.phase !== "results"}
    <div class="card form">
      <span class="label">Folder with photos</span>
      <div class="row">
        <input type="text" class="typed mono" placeholder="Choose a folder, or type / paste its path" bind:value={people.folder}
          disabled={app.busy} onkeydown={(e) => e.key === "Enter" && people.folder.trim() && scan()} />
        <button disabled={app.busy} onclick={browse}>Browse…</button>
      </div>
      <span class="muted small">Photos are only read, never changed. Sub-folders are included. About 10 photos per second.</span>
      <div class="row actions">
        {#if progress}
          <div class="progress">
            <div class="bar" class:indeterminate={!progress.done}><div style="width: {progress.total ? (100 * progress.done) / progress.total : 3}%"></div></div>
            <span class="muted">{progress.done ? `Looked at ${progress.done} of ${progress.total} photos` : "Loading the face recognition and listing the photos …"}</span>
          </div>
        {/if}
        <span class="spacer"></span>
        {#if error.includes("Settings")}<button onclick={() => go("settings")}>Open Settings</button>{/if}
        <button class="primary go" data-continue disabled={!people.folder.trim() || app.busy} onclick={scan}>Find faces</button>
      </div>
    </div>
  {:else}
    {@const r = people.result}
    <div class="row head">
      <div>
        <h2>{plural(r.faces.length, "face", "faces")} in {plural(r.photos_with_faces, "photo", "photos")}</h2>
        <p class="muted">{plural(r.photos, "photo", "photos")} looked at · {plural(multi.length, "group", "groups")} of faces that look alike · {plural(singles.length, "face", "faces")} seen only once</p>
      </div>
      <span class="spacer"></span>
      <button class="ghost" onclick={resetPeople}>← Another folder</button>
    </div>

    <p class="muted how">
      Type a name and press <kbd>Enter</kbd> to remember a group. A face in the wrong group? Click it, then <em>Split off</em>.
      Two groups are the same person? Tick both, then <em>Merge</em>. Double-click a face to see its photo in Explorer.
    </p>

    {#if picked.length > 1}
      <div class="mergebar card">
        <strong>{plural(picked.length, "group", "groups")} ticked</strong>
        <span class="spacer"></span>
        <button onclick={() => people.groups.forEach((g) => (g.pick = false))}>Untick</button>
        <button class="primary" onclick={mergePicked}>Merge into one group</button>
      </div>
    {/if}

    {#each multi as g (g.id)}
      <div class="card group" class:saved={g.saved}>
        <div class="row ghead">
          <label class="check"><input type="checkbox" bind:checked={g.pick} /> <strong>{plural(g.faces.length, "face", "faces")}</strong></label>
          <span class="muted small">in {plural(photosOf(g.faces), "photo", "photos")}{yearsOf(g.faces) ? ` · ${yearsOf(g.faces)}` : ""}</span>
          {#if g.suggestion && !g.saved}
            {#if sure(g.suggestion)}
              <span class="chip accent">Looks like {g.suggestion.name} ({pct(g.suggestion.similarity)} % alike)</span>
            {:else}
              <span class="chip" title="Only a weak resemblance: please check the faces before saying yes">Might be {g.suggestion.name}? ({pct(g.suggestion.similarity)} % alike)</span>
            {/if}
            <button class="small" onclick={() => save(g, g.suggestion.person_id)}>Yes, that's {g.suggestion.name}</button>
          {/if}
          <span class="spacer"></span>
          {#if g.saved}<span class="chip green">✓ {g.saved}</span>{/if}
        </div>
        <div class="strip">
          {#each open[g.id] ? g.faces : g.faces.slice(0, STRIP) as i (i)}
            <button class="face" class:sel={chosen[`${g.id}:${i}`]} title={people.result.faces[i].rel}
              onclick={() => toggleFace(g, i)} ondblclick={() => reveal(i)}>
              <img src={faceUrl(i)} alt="" loading="lazy" />
            </button>
          {/each}
          {#if !open[g.id] && g.faces.length > STRIP}
            <button class="more" onclick={() => (open = { ...open, [g.id]: true })}>+{g.faces.length - STRIP} more</button>
          {/if}
        </div>
        <div class="row gfoot">
          {#if selCount(g)}
            <button class="small" disabled={selCount(g) === g.faces.length} onclick={() => splitOff(g)}>
              Split off {plural(selCount(g), "face", "faces")} (not this person)
            </button>
          {/if}
          <span class="spacer"></span>
          <input type="text" list="known-names" placeholder="Who is this?" maxlength="60" bind:value={g.name}
            onkeydown={(e) => e.key === "Enter" && save(g)} />
          <button class="primary" disabled={!g.name.trim() || g.saved === g.name.trim()} onclick={() => save(g)}>Save</button>
        </div>
      </div>
    {/each}

    {#if singles.length}
      <div class="card group">
        <div class="row ghead">
          <strong>Faces seen only once ({singles.length})</strong>
          <span class="muted small">Tick faces of the same person and merge them, or name one on its own.</span>
          <span class="spacer"></span>
          <button class="small" onclick={() => (people.showSingles = !people.showSingles)}>{people.showSingles ? "Hide" : "Show"}</button>
        </div>
        {#if people.showSingles}
          <div class="singles">
            {#each singles as g (g.id)}
              {@const i = g.faces[0]}
              <div class="single" class:saved={g.saved} class:sel={g.pick}>
                <button class="face big" title={people.result.faces[i].rel} onclick={() => (g.pick = !g.pick)} ondblclick={() => reveal(i)}>
                  <img src={faceUrl(i)} alt="" loading="lazy" />
                </button>
                {#if g.saved}<span class="chip green small">✓ {g.saved}</span>
                {:else}
                  {#if g.suggestion}<span class="muted small" title="{pct(g.suggestion.similarity)} % alike">{sure(g.suggestion) ? "" : "maybe "}{g.suggestion.name}?</span>{/if}
                  <input type="text" class="sname" list="known-names" placeholder="Name" maxlength="60" bind:value={g.name}
                    onkeydown={(e) => e.key === "Enter" && save(g)} />
                {/if}
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/if}

    {#if r.errors.length}
      <details class="card errs"><summary>{plural(r.errors.length, "photo", "photos")} could not be read</summary>
        {#each r.errors as e}<div class="mono small">{e}</div>{/each}
      </details>
    {/if}
    <datalist id="known-names">{#each knownNames as n}<option value={n}></option>{/each}</datalist>
  {/if}
</section>

<style>
  .wrap { max-width: 1240px; margin: 0 auto; padding: 24px 24px 48px; display: grid; gap: 16px; }
  .head { display: grid; gap: 4px; }
  .row.head { display: flex; }
  .form { padding: 20px 24px; display: grid; gap: 12px; }
  .label { font-weight: 600; font-size: 15px; }
  .typed { flex: 1; }
  .small { font-size: 12.5px; }
  .actions { border-top: 1px solid var(--border); padding-top: 14px; }
  .progress { display: grid; gap: 6px; flex: 1; max-width: 420px; }
  .problem { color: var(--red); background: var(--red-soft); padding: 8px 12px; border-radius: var(--radius-sm); }
  .good { color: var(--green); font-weight: 600; }
  .go { font-size: 15px; padding: 10px 20px; }
  .how { font-size: 14px; }
  kbd { font: 12px var(--mono); border: 1px solid var(--border); border-bottom-width: 2px; border-radius: 4px; padding: 0 6px; background: var(--surface); }
  .check { display: flex; gap: 8px; align-items: center; cursor: pointer; }
  .check input { accent-color: var(--accent); width: 18px; height: 18px; }

  .known { padding: 16px 20px; display: grid; gap: 12px; }
  .kgrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 10px; }
  .person { display: grid; grid-template-columns: 64px 1fr; gap: 12px; align-items: center; padding: 8px; border: 1px solid var(--border); border-radius: var(--radius-sm); }
  .cover { width: 64px; height: 64px; border-radius: 50%; object-fit: cover; background: var(--surface-2); }
  .pinfo { display: grid; gap: 2px; min-width: 0; }
  .pinfo strong { font-size: 16px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .acts { gap: 4px; flex-wrap: wrap; }
  .acts select { max-width: 150px; }
  .born { display: flex; gap: 6px; align-items: center; }
  .born input { width: 78px; padding: 2px 6px; font-size: 13px; }

  .mergebar { position: sticky; top: 6px; z-index: 2; display: flex; gap: 10px; align-items: center; padding: 10px 16px; border-color: var(--accent); }
  .group { padding: 12px 16px; display: grid; gap: 10px; }
  .group.saved { border-color: var(--green); }
  .ghead { gap: 10px; flex-wrap: wrap; }
  .gfoot { gap: 8px; }
  .gfoot input { width: 240px; }
  .strip { display: flex; flex-wrap: wrap; gap: 6px; }
  .face { padding: 0; width: 76px; height: 76px; border-radius: 8px; overflow: hidden; border: 3px solid transparent; background: var(--surface-2); }
  .face img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .face:hover:not(:disabled) { border-color: var(--border); }
  .face.sel { border-color: var(--red); }
  .face.big { width: 96px; height: 96px; }
  .more { height: 76px; }
  .singles { display: grid; grid-template-columns: repeat(auto-fill, minmax(112px, 1fr)); gap: 10px; }
  .single { display: grid; gap: 4px; justify-items: center; }
  .single.saved { opacity: 0.7; }
  .single.sel .face { border-color: var(--accent); }
  .sname { width: 104px; font-size: 13px; padding: 4px 6px; }
  .errs { padding: 12px 16px; }
</style>
