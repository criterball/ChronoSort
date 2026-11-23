<script>
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { metaRead, metaWrite, formatBytes } from "../lib/api.js";

  // item: browser entry; index/total for "[n / N]"; onchanged() after a save (undo label etc.);
  // onpin(bool): keep the fly-out open while editing.
  let { item, index, total, onchanged, onpin } = $props();

  let info = $state(null);
  let error = $state("");
  let tab = $state("summary"); // summary | all | edit
  let filter = $state("");
  let form = $state(null);
  let saving = $state(false);
  let placeQ = $state("");
  let placeHits = $state([]);

  // Summary rows: the first tag found (any group) for each name.
  const ROWS = [
    ["Camera", ["Make", "Model"]], ["Lens", ["LensModel", "Lens", "LensID"]], ["Date taken", ["DateTimeOriginal", "CreateDate"]],
    ["Exposure", ["ExposureTime"]], ["Aperture", ["FNumber"]], ["ISO", ["ISO"]], ["Focal length", ["FocalLength"]],
    ["35 mm equivalent", ["FocalLengthIn35mmFormat"]], ["Flash", ["Flash"]], ["Program", ["ExposureProgram"]],
    ["Metering", ["MeteringMode"]], ["Exposure bias", ["ExposureCompensation"]], ["Picture size", ["ImageSize"]],
    ["Title", ["Title", "XPTitle", "ObjectName"]], ["Description", ["Description", "ImageDescription", "Caption-Abstract"]],
    ["Keywords", ["Subject", "Keywords", "XPKeywords"]], ["Author", ["Creator", "Artist", "By-line"]], ["Copyright", ["Rights", "Copyright", "CopyrightNotice"]],
    ["Place", ["City", "State", "Country"]], ["Software", ["Software"]],
  ];
  const find = (name) => info?.tags.find((t) => t.name === name)?.value ?? "";
  function row(names) {
    if (names[0] === "Make") return [find("Make"), find("Model").replace(find("Make"), "").trim()].filter(Boolean).join(" ");
    if (names[0] === "City") return names.map(find).filter(Boolean).join(", ");
    for (const n of names) if (find(n)) return find(n);
    return "";
  }

  // Re-read only when another photo is shown (not when the tab changes).
  $effect(() => {
    const id = item?.id;
    untrack(() => {
      info = null;
      error = "";
      if (form) onpin?.(false);
      form = null;
      if (tab === "edit") tab = "summary";
    });
    if (id == null) return;
    metaRead(id).then((r) => { if (item?.id === id) info = r; }).catch((e) => (error = String(e)));
  });

  function startEdit() {
    const dt = find("DateTimeOriginal") || find("CreateDate");
    form = {
      date: dt ? dt.replace(/^(\d{4}):(\d{2}):(\d{2})/, "$1-$2-$3").slice(0, 19) : "",
      title: row(["Title", "XPTitle", "ObjectName"]),
      description: row(["Description", "ImageDescription", "Caption-Abstract"]),
      keywords: row(["Subject", "Keywords", "XPKeywords"]).replace(/;\s*/g, ", "),
      author: row(["Creator", "Artist", "By-line"]),
      copyright: row(["Rights", "Copyright", "CopyrightNotice"]),
      lat: info?.gps ? info.gps[0].toFixed(6) : "",
      lon: info?.gps ? info.gps[1].toFixed(6) : "",
    };
    form.orig = { ...form };
    tab = "edit";
    onpin?.(true);
  }
  function cancelEdit() {
    form = null;
    tab = "summary";
    error = "";
    onpin?.(false);
  }

  async function save() {
    const f = form, o = form.orig, ch = {};
    if (f.date.trim() !== o.date.trim()) ch.date_taken = f.date;
    if (f.title !== o.title) ch.title = f.title;
    if (f.description !== o.description) ch.description = f.description;
    if (f.keywords !== o.keywords) ch.keywords = f.keywords.split(/[,;]/).map((s) => s.trim()).filter(Boolean);
    if (f.author !== o.author) ch.author = f.author;
    if (f.copyright !== o.copyright) ch.copyright = f.copyright;
    if (f.lat !== o.lat || f.lon !== o.lon) {
      if (!f.lat.trim() && !f.lon.trim()) ch.clear_gps = true;
      else {
        const la = parseFloat(f.lat), lo = parseFloat(f.lon);
        if (!isFinite(la) || !isFinite(lo)) { error = "Type the position as two numbers, e.g. 48.8566 and 2.3522."; return; }
        ch.gps = [la, lo];
        // A town picked from the list (and not changed by hand since) also writes its names.
        const p = form.place;
        if (p && p.lat.toFixed(6) === f.lat && p.lon.toFixed(6) === f.lon)
          ch.place = { city: p.city, region: p.region, country: p.country, country_code: p.country_code };
      }
    }
    if (!Object.keys(ch).length) { cancelEdit(); return; }
    saving = true;
    error = "";
    try {
      info = await metaWrite(item.id, ch);
      form = null;
      tab = "summary";
      onpin?.(false);
      onchanged?.("Information saved. Ctrl+Z in the browser puts the old information back.");
    } catch (e) {
      error = String(e);
    } finally {
      saving = false;
    }
  }

  let searchTimer;
  function searchPlace() {
    clearTimeout(searchTimer);
    const q = placeQ.trim();
    if (q.length < 2) { placeHits = []; return; }
    searchTimer = setTimeout(async () => {
      try {
        const r = await invoke("places_search", { query: q });
        placeHits = r.places.slice(0, 6);
      } catch (e) {
        error = String(e);
      }
    }, 250);
  }
  function pickPlace(p) {
    form.lat = p.lat.toFixed(6);
    form.lon = p.lon.toFixed(6);
    form.place = p;
    placeQ = [p.city, p.region, p.country].filter(Boolean).join(", ");
    placeHits = [];
  }

  let allTags = $derived((info?.tags ?? []).filter((t) => !filter || (t.group + t.name + t.value).toLowerCase().includes(filter.toLowerCase())));
  const fmtDate = (s) => (s ? new Date(s * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "");
</script>

<div class="info">
  <h3>File</h3>
  <dl>
    <dt>Name</dt><dd>{item.name} &nbsp;[{index + 1} / {total}]</dd>
    <dt>Type</dt><dd>{item.ext.toUpperCase()}{find("ImageSize") ? " · " + find("ImageSize") : ""}</dd>
    <dt>Size</dt><dd>{formatBytes(item.size)}</dd>
    <dt>Modified</dt><dd>{fmtDate(item.modified)}</dd>
  </dl>

  <div class="tabs" role="tablist">
    <button role="tab" class:on={tab === "summary"} aria-selected={tab === "summary"} onclick={() => (form ? null : (tab = "summary"))}>EXIF</button>
    <button role="tab" class:on={tab === "all"} aria-selected={tab === "all"} onclick={() => (form ? null : (tab = "all"))}>All tags</button>
    {#if info?.writable}
      <button role="tab" class="edit" class:on={tab === "edit"} aria-selected={tab === "edit"} onclick={() => !form && startEdit()}>✎ Edit</button>
    {/if}
  </div>

  {#if error}<p class="err">{error}</p>{/if}
  {#if !info && !error}
    <p class="muted">Reading…</p>
  {:else if info && tab === "summary"}
    <table>
      <tbody>
        {#each ROWS as [label, names]}
          <tr><th>{label}</th><td>{row(names)}</td></tr>
        {/each}
        <tr><th>GPS</th><td>{info.gps ? `${info.gps[0].toFixed(5)}, ${info.gps[1].toFixed(5)}` : ""}</td></tr>
      </tbody>
    </table>
  {:else if info && tab === "all"}
    <input class="flt" bind:value={filter} placeholder="Find a tag…" spellcheck="false" />
    <table class="all">
      <tbody>
        {#each allTags as t}
          <tr><th title={t.group}>{t.name}<small>{t.group}</small></th><td>{t.value}</td></tr>
        {/each}
      </tbody>
    </table>
  {:else if form && tab === "edit"}
    <form class="ed" onsubmit={(e) => { e.preventDefault(); save(); }}>
      <label>Date taken <input bind:value={form.date} placeholder="YYYY-MM-DD HH:MM:SS" /></label>
      <label>Title <input bind:value={form.title} /></label>
      <label>Description <textarea rows="2" bind:value={form.description}></textarea></label>
      <label>Keywords <input bind:value={form.keywords} placeholder="beach, family, 2004" /></label>
      <label>Author <input bind:value={form.author} /></label>
      <label>Copyright <input bind:value={form.copyright} /></label>
      <fieldset>
        <legend>Place (GPS + town, region, country)</legend>
        <input bind:value={placeQ} oninput={searchPlace} placeholder="Find a town… (offline list)" />
        {#if placeHits.length}
          <ul class="hits">
            {#each placeHits as p}<li><button type="button" onclick={() => pickPlace(p)}>{[p.city, p.region, p.country].filter(Boolean).join(", ")}</button></li>{/each}
          </ul>
        {/if}
        <div class="ll">
          <input bind:value={form.lat} placeholder="Latitude" aria-label="Latitude" />
          <input bind:value={form.lon} placeholder="Longitude" aria-label="Longitude" />
          <button type="button" onclick={() => { form.lat = ""; form.lon = ""; form.place = null; placeQ = ""; }} title="Remove the position and the place names">✕</button>
        </div>
      </fieldset>
      <p class="muted small">A safety copy is kept first; Ctrl+Z in the browser puts the old information back.</p>
      <div class="btns">
        <button type="button" onclick={cancelEdit} disabled={saving}>Cancel</button>
        <button class="primary" type="submit" disabled={saving}>{saving ? "Saving…" : "Save information"}</button>
      </div>
    </form>
  {/if}
</div>

<style>
  .info { display: grid; gap: 6px; padding: 0 4px; }
  h3 { font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; color: #9aa1ab; margin: 8px 8px 2px; font-weight: 600; }
  dl { display: grid; grid-template-columns: 80px 1fr; gap: 2px 10px; margin: 0 8px; font-size: 13px; }
  dt { color: #9aa1ab; }
  dd { margin: 0; overflow-wrap: anywhere; }
  .tabs { display: flex; gap: 2px; margin: 6px 6px 0; border-bottom: 1px solid #3a3e45; }
  .tabs button { background: none; border: none; color: #c9ced6; padding: 5px 10px; border-radius: 6px 6px 0 0; font-weight: 500; font-size: 13px; }
  .tabs button.on { background: #2b2f36; color: #fff; }
  .tabs .edit { margin-left: auto; color: #8fb4ff; }
  table { width: 100%; border-collapse: collapse; font-size: 12.5px; }
  th { text-align: left; font-weight: 400; color: #9aa1ab; width: 42%; padding: 3px 8px; vertical-align: top; }
  td { padding: 3px 8px; overflow-wrap: anywhere; }
  tr:nth-child(odd) { background: rgba(255, 255, 255, 0.03); }
  .all th small { display: block; font-size: 10.5px; opacity: 0.7; }
  .flt, .ed input, .ed textarea { width: 100%; box-sizing: border-box; background: #15171a; color: #eee; border: 1px solid #3a3e45; border-radius: 5px; padding: 5px 7px; font: 13px var(--font); }
  .flt { margin: 4px 0; }
  .ed { display: grid; gap: 7px; padding: 4px 6px; font-size: 12.5px; color: #c9ced6; }
  .ed label { display: grid; gap: 2px; }
  fieldset { border: 1px solid #3a3e45; border-radius: 6px; padding: 6px; display: grid; gap: 5px; }
  legend { padding: 0 4px; }
  .ll { display: grid; grid-template-columns: 1fr 1fr auto; gap: 4px; }
  .ll button, .btns button { background: #2b2f36; color: #eee; border: 1px solid #3a3e45; }
  .hits { list-style: none; margin: 0; padding: 0; max-height: 140px; overflow: auto; }
  .hits button { width: 100%; text-align: left; background: none; border: none; color: #dfe3ea; padding: 3px 6px; font-weight: 400; }
  .hits button:hover { background: #3a64c8; }
  .btns { display: flex; justify-content: flex-end; gap: 6px; }
  .btns .primary { background: #3a64c8; border-color: #3a64c8; color: #fff; }
  .err { color: #ff8a80; margin: 4px 8px; font-size: 13px; }
  .muted { color: #9aa1ab; margin: 4px 8px; }
  .small { font-size: 11.5px; margin: 0; }
</style>
