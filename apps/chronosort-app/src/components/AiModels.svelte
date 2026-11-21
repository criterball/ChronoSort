<script>
  import { onMount, onDestroy } from "svelte";
  import {
    aiOverview, linkModelFile, unlinkModel, downloadPack, cancelDownload, deleteModelFiles,
    pickModelFile, platformInfo, formatBytes, confirmBox, onProgress, openFolder,
  } from "../lib/api.js";
  import { dl, downloadEnded, timeLeft } from "../lib/state.svelte.js";

  let ov = $state(null);
  let info = $state(null);
  let error = $state("");
  let busy = $state(false); // linking / deleting
  let prog = $state(null); // last model-progress event
  let stopListen;

  onMount(async () => {
    stopListen = await onProgress("model-progress", (p) => (prog = p));
    if (dl.pack) prog = dl.prog; // came back to Settings during a download
    try {
      [ov, info] = await Promise.all([aiOverview(), platformInfo()]);
    } catch (e) {
      error = String(e);
    }
  });
  onDestroy(() => stopListen?.());

  const TIER = {
    max: ["Max", "the most accurate models"],
    standard: ["Standard", "the standard models"],
    light: ["Light", "the light models"],
    metadata: ["Without AI", "dates from the files only"],
  };
  const PURPOSE = {
    faces: "Recognising faces",
    engine: "AI engine (the program that runs the photo-reading models)",
    vision: "Reading the photos (dates, handwriting, era)",
    helper: "Previews of videos and iPhone (HEIC) photos",
  };
  const ROLE = { model: "model file", projector: "vision add-on (mmproj)", detector: "face finder", recogniser: "face recogniser", archive: "program" };
  const FIT = {
    gpu: ["green", "Runs on your graphics card"],
    cpu: ["amber", "Runs on the processor (slower)"],
    too_big: ["red", "Too big for this PC"],
  };
  const STATUS = {
    ready: ["green", "✓ Ready"],
    partial: ["amber", "Some files still needed"],
    changed: ["red", "A file was moved or changed"],
    missing: ["", "Not set up"],
  };
  const extOf = (name) => name.slice(name.lastIndexOf(".") + 1);
  const gb = (b) => formatBytes(b);
  const pct = (p) => (p && p.total ? Math.floor((100 * p.done) / p.total) : 0);

  let byPurpose = $derived.by(() => {
    if (!ov) return [];
    return ["faces", "engine", "vision", "helper"].filter((p) => ov.packs.some((x) => x.purpose === p)).map((p) => [
      p,
      ov.packs.filter((x) => x.purpose === p).sort((a, b) => (ov.recommended[p] === b.id) - (ov.recommended[p] === a.id)),
    ]);
  });
  let downloading = $derived(ov?.downloading ?? null);

  async function download(pack, need, confirmed = false) {
    error = "";
    if (!confirmed && need > 500 * 1024 * 1024) {
      const ok = await confirmBox(
        `Download "${pack.name}" (${gb(need)}) from ${pack.unpack ? "the official llama.cpp releases on GitHub" : "Hugging Face"}?\n\nOnly the ${pack.unpack ? "program" : "model"} is downloaded; nothing about you or your photos is sent. Each file is checked against its fingerprint before it is used. You can cancel and continue later.`,
        "Download AI model", "Download", "Cancel",
      );
      if (!ok) return;
    }
    ov = { ...ov, downloading: pack.id };
    prog = null;
    Object.assign(dl, { pack: pack.id, name: pack.name, prog: null, rate: 0 });
    try {
      ov = await downloadPack(pack.id);
    } catch (e) {
      error = `${pack.name}: ${e}`;
      ov = await aiOverview().catch(() => ({ ...ov, downloading: null }));
    } finally {
      prog = null;
      downloadEnded();
    }
  }

  // Everything suggested for this PC that is still missing, smallest first.
  let suggestedMissing = $derived(
    ov
      ? Object.values(ov.recommended)
          .map((id) => ov.packs.find((p) => p.id === id))
          .filter((p) => p && p.status !== "ready" && p.downloadable && p.fit !== "too_big")
          .sort((a, b) => a.approx_bytes - b.approx_bytes)
      : [],
  );

  async function getAll() {
    const total = suggestedMissing.reduce((n, p) => n + p.approx_bytes, 0);
    const ok = await confirmBox(
      `Download everything suggested for this PC (${gb(total)})?\n\n${suggestedMissing.map((p) => "• " + p.name).join("\n")}\n\nOnly the models and the AI engine are downloaded; nothing about you or your photos is sent. Every file is checked before it is used. You can cancel at any time and continue later.`,
      "Set up AI", "Download", "Cancel",
    );
    if (!ok) return;
    for (const p of [...suggestedMissing]) {
      await download(p, 0, true);
      if (error) break; // stop on the first failure or cancel
    }
  }

  async function link(pack, file) {
    error = "";
    const path = await pickModelFile(`Choose the ${ROLE[file.role] ?? file.role}: ${file.name}`, extOf(file.name));
    if (!path) return;
    busy = true;
    prog = null;
    try {
      ov = await linkModelFile(pack.id, file.role, path);
    } catch (e) {
      error = `${file.name}: ${e}`;
    } finally {
      busy = false;
      prog = null;
    }
  }

  async function forget(pack) {
    const ours = pack.owns_files;
    const ok = await confirmBox(
      ours
        ? `Delete the downloaded files of "${pack.name}" (${gb(pack.approx_bytes)})? You can download them again later.`
        : `Forget where "${pack.name}" is? Your model files themselves are not deleted.`,
      ours ? "Delete model" : "Forget model",
      ours ? "Delete" : "Forget",
      "Cancel",
    );
    if (!ok) return;
    busy = true;
    try {
      ov = ours ? await deleteModelFiles(pack.id) : await unlinkModel(pack.id);
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="card block">
  <h2>This computer</h2>
  {#if ov && info}
    <dl>
      <dt>Memory (RAM)</dt><dd>{ov.ram_bytes ? gb(ov.ram_bytes) : "unknown"}</dd>
      {#each ov.gpus as g}
        <dt>Graphics</dt>
        <dd>{g.name} <span class="muted">· {g.integrated ? "built into the processor" : g.vram_bytes ? `${gb(g.vram_bytes)} graphics memory` : "memory unknown"}</span></dd>
      {:else}
        <dt>Graphics</dt><dd class="muted">no graphics card found</dd>
      {/each}
      <dt>Processor threads</dt><dd>{info.cpu_threads}</dd>
      <dt>ChronoSort</dt><dd>version {info.version} · {info.os}</dd>
    </dl>
    <div class="verdict tier-{ov.assessment.tier}">
      <span class="level">{TIER[ov.assessment.tier][0]}</span>
      <div>
        <strong>Suggested AI level for this PC: {TIER[ov.assessment.tier][0]}</strong>
        <p>{ov.assessment.reason}</p>
      </div>
    </div>
  {:else if error}
    <p class="problem">{error}</p>
  {:else}
    <p class="muted">Checking this computer…</p>
  {/if}
</div>

{#if ov}
  <div class="card block">
    <div>
      <h2>AI models</h2>
      <p class="muted small">
        The AI runs on this PC; your photos and videos never leave it. A model only needs to be downloaded once.
        ChronoSort does not use these models yet (faces and date reading are the next features), but you can get
        them ready now.
      </p>
    </div>

    {#if error}<p class="problem">{error}</p>{/if}

    {#if suggestedMissing.length}
      <div class="getall">
        <div>
          <strong>Get everything suggested for this PC</strong>
          <p class="muted small">{suggestedMissing.map((p) => p.name).join(" · ")} — about {gb(suggestedMissing.reduce((n, p) => n + p.approx_bytes, 0))}</p>
        </div>
        <button class="primary" disabled={busy || !!downloading} onclick={getAll}>⬇ Download all</button>
      </div>
    {/if}

    {#each byPurpose as [purpose, packs]}
      <h3>{PURPOSE[purpose]}</h3>
      <div class="packs">
        {#each packs as p (p.id)}
          {@const rec = ov.recommended[purpose] === p.id}
          {@const ft = p.fit === "cpu" && p.vram_gb === 0 ? ["green", "Runs on the processor (no graphics card needed)"] : FIT[p.fit]}
          {@const mine = downloading === p.id}
          {@const checking = busy && prog?.pack === p.id && prog?.phase === "check"}
          {@const toGet = p.files.filter((f) => !p.verified[f.role]).reduce((n, f) => n + (f.bytes ?? 0), 0)}
          <div class="pack" class:rec>
            <div class="row top">
              <strong class="pname">{p.name}</strong>
              {#if rec}<span class="chip accent">★ Suggested for this PC</span>{/if}
              <span class="spacer"></span>
              <span class="chip {STATUS[p.status][0]}">{STATUS[p.status][1]}</span>
            </div>
            <p class="small">{p.description}</p>
            <div class="row facts">
              <span class="chip {ft[0]}">{ft[1]}</span>
              <span class="muted small">{gb(p.approx_bytes)} · licence {p.licence}</span>
            </div>

            <div class="files">
              {#each p.files as f}
                <div class="file">
                  <span class="muted small role">{ROLE[f.role] ?? f.role}</span>
                  {#if p.linked[f.role]}
                    <span class="mono path" title={p.linked[f.role]}><span dir="ltr">{p.linked[f.role]}</span></span>
                    {#if p.unpack}
                      <span class="chip green" title="The download matched GitHub's published SHA-256 before it was unpacked">✓ checked</span>
                    {:else if p.verified[f.role]}
                      <span class="chip green" title="Size and SHA-256 fingerprint match the checked original">✓ checked</span>
                    {:else}
                      <span class="chip amber" title="Not byte-for-byte the checked file (e.g. another publisher's copy). It may still work.">your own copy</span>
                    {/if}
                  {:else}
                    <span class="mono muted fname">{f.name}</span>
                    {#if !p.unpack}<button class="small ghost" disabled={busy || mine} onclick={() => link(p, f)}>Use a file I have…</button>{/if}
                  {/if}
                </div>
              {/each}
            </div>

            {#if mine}
              <div class="dl">
                <div class="bar" class:indeterminate={!prog}><div style="width:{pct(prog)}%"></div></div>
                <div class="row">
                  <span class="small">
                    {#if !prog}Connecting to the download server…
                    {:else if prog.phase === "check"}Checking file {prog.index + 1} of {prog.files}… {pct(prog)}%
                    {:else}Downloading file {prog.index + 1} of {prog.files}: {gb(prog.done)} of {gb(prog.total)} ({pct(prog)}%){#if dl.rate}{" · " + gb(dl.rate) + "/s"}{/if}{#if timeLeft()}{" · " + timeLeft()}{/if}{/if}
                    <br /><span class="muted">You can use the rest of ChronoSort meanwhile; the download keeps going.</span>
                  </span>
                  <span class="spacer"></span>
                  <button class="small" onclick={() => cancelDownload()}>Cancel</button>
                </div>
              </div>
            {:else if checking}
              <div class="dl">
                <div class="bar"><div style="width:{pct(prog)}%"></div></div>
                <span class="small">Checking {prog.file}… {pct(prog)}%</span>
              </div>
            {/if}

            <div class="row actions">
              {#if toGet > 0 && p.downloadable && !mine}
                <button class:primary={rec} disabled={busy || !!downloading || p.fit === "too_big"} onclick={() => download(p, toGet)}>
                  {#if toGet < p.approx_bytes}⬇ Download the checked missing part ({gb(toGet)}){:else}⬇ Download ({gb(toGet)}){/if}
                </button>
                {#if downloading && downloading !== p.id}<span class="muted small">another download is running</span>{/if}
              {/if}
              <span class="spacer"></span>
              {#if p.status !== "missing" && !mine}
                <button class="ghost small" disabled={busy} onclick={() => forget(p)}>
                  {p.owns_files ? "Delete downloaded files" : "Forget this model"}
                </button>
              {/if}
            </div>
          </div>
        {/each}
      </div>
    {/each}

    <p class="muted small">
      Models come from Hugging Face, from the official publisher of each model; the AI engine comes from the official
      llama.cpp releases on GitHub. Both are pinned to a fixed version. Every
      file is checked against its SHA-256 fingerprint before ChronoSort will use it; a damaged download is deleted.
      Models are stored in <button class="linkbtn mono" onclick={() => openFolder(ov.models_dir).catch(() => {})}>{ov.models_dir}</button>
      {#if ov.free_bytes != null}({gb(ov.free_bytes)} free){/if}. No internet on this PC? Copy the files from a USB stick and use “Use a file I have…”.
    </p>
  </div>
{/if}

<style>
  .block { padding: 18px 22px; display: grid; gap: 12px; }
  .small { font-size: 12.5px; }
  dl { display: grid; grid-template-columns: 160px 1fr; gap: 6px 12px; margin: 0; }
  dt { color: var(--muted); }
  dd { margin: 0; font-weight: 600; }
  .verdict { display: flex; gap: 14px; align-items: center; padding: 12px 14px; border-radius: var(--radius-sm); background: var(--surface-2); }
  .verdict p { color: var(--muted); }
  .level {
    min-width: 92px; text-align: center; padding: 10px 8px; border-radius: var(--radius-sm); font-weight: 700;
    background: var(--accent); color: var(--accent-text);
  }
  .tier-metadata .level { background: var(--surface); color: var(--muted); border: 1px solid var(--border); }
  h3 { margin-top: 6px; }
  .packs { display: grid; gap: 10px; }
  .pack { border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 12px 14px; display: grid; gap: 6px; }
  .pack.rec { border-color: var(--accent); border-width: 1.5px; }
  .top, .facts { flex-wrap: wrap; }
  .pname { font-size: 15px; }
  .files { display: grid; gap: 4px; margin-top: 2px; }
  .file { display: grid; grid-template-columns: 170px 1fr auto; gap: 10px; align-items: center; min-height: 28px; }
  .path { font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; direction: rtl; text-align: left; }
  .path > span { unicode-bidi: isolate; }
  .fname { font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .getall { display: flex; gap: 14px; align-items: center; justify-content: space-between; padding: 12px 14px; border-radius: var(--radius-sm); background: var(--accent-soft); border: 1px solid var(--accent); }
  .getall .primary { padding: 9px 18px; font-size: 15px; }
  .dl { display: grid; gap: 6px; padding: 8px 10px; background: var(--surface-2); border-radius: var(--radius-sm); }
  .actions { margin-top: 2px; }
  .actions button.primary { padding: 8px 16px; }
  .linkbtn { border: none; background: none; padding: 0; color: var(--accent); text-decoration: underline; font-size: 12px; cursor: pointer; }
  .problem { color: var(--red); background: var(--red-soft); padding: 8px 12px; border-radius: var(--radius-sm); }
</style>
