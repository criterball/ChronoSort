<script>
  // Slideshow: a settings card first, then the photos (and videos) full screen
  // with a transition, optional music, and controls that appear when the mouse
  // moves. The screen is kept awake while it plays. Nothing is changed on disk.
  import { onMount, tick } from "svelte";
  import { fade, fly, scale } from "svelte/transition";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Icon from "./Icon.svelte";
  import { fthumbUrl, mediaUrl, pickMusic, slideshowMusic, slideshowAwake, slideshowPlayable } from "../lib/api.js";

  // items: browse entries { id, name, kind, modified }; start: index to begin at; onclose(lastId)
  let { items, start = 0, onclose } = $props();

  const MEMORY = "chronosort.slideshow.v1";
  const DEFAULTS = { seconds: 5, transition: "fade", shuffle: false, loop: true, videos: "play", sound: true, caption: "none", music: [], volume: 70 };
  function load() {
    try { return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(MEMORY) ?? "{}") }; } catch { return { ...DEFAULTS }; }
  }
  let opt = $state(load());
  function save() {
    try { localStorage.setItem(MEMORY, JSON.stringify($state.snapshot(opt))); } catch { /* no memory: fine */ }
  }

  let phase = $state("setup"); // setup | play | end
  let order = $state([]); // indexes into items
  let pos = $state(0); // position in `order`
  let paused = $state(false);
  let controls = $state(true);
  let playable = new Set(); // ids of videos the webview can play
  let tracks = $state([]); // [{ id, name }]
  let track = $state(0);
  let musicOn = $state(true);
  let note = $state("");
  let el = $state(), audio = $state(), video = $state();
  let timer = null, hideTimer = null, wasFull = false;

  const shown = $derived(items.length ? (opt.videos === "skip" ? items.filter((x) => x.kind !== "video") : items) : []);
  const countText = $derived.by(() => {
    const v = items.filter((x) => x.kind === "video").length, p = items.length - v;
    const ps = `${p} ${p === 1 ? "photo" : "photos"}`, vs = `${v} ${v === 1 ? "video" : "videos"}`;
    if (!v) return ps;
    if (opt.videos === "skip") return `${ps} (${vs} left out)`;
    return p ? `${ps} and ${vs}` : vs;
  });
  const cur = $derived(order.length ? shown[order[pos]] : null);
  const isVideo = (e) => e?.kind === "video";
  const plays = (e) => isVideo(e) && opt.videos === "play" && playable.has(e.id);
  const TRANSITION_MS = 900;
  const KINDS = ["fade", "slide", "zoom"];
  let kind = $state("fade"); // transition of the current change

  // ---------- setup ----------
  async function chooseMusic() {
    const picked = await pickMusic("Choose music for the slideshow (one or more songs)");
    if (!picked?.length) return;
    try {
      const t = await slideshowMusic(picked);
      tracks = t.map(([id, name]) => ({ id, name }));
      opt.music = picked;
      note = "";
    } catch (e) {
      note = String(e);
    }
  }
  function noMusic() {
    opt.music = [];
    tracks = [];
  }
  onMount(() => {
    // Music from last time: only if the files are still there.
    if (opt.music?.length) slideshowMusic(opt.music).then((t) => (tracks = t.map(([id, name]) => ({ id, name })))).catch(() => (opt.music = []));
    tick().then(() => el?.focus());
    return () => {
      clearTimeout(timer);
      clearTimeout(hideTimer);
      slideshowAwake(false).catch(() => {});
      if (phase !== "setup" && !wasFull) getCurrentWindow().setFullscreen(false).catch(() => {});
    };
  });

  async function begin() {
    if (!shown.length) { note = "There is nothing to show: no photos here" + (opt.videos === "skip" ? " (videos are left out)." : "."); return; }
    save();
    try { playable = new Set(await slideshowPlayable(shown.filter(isVideo).map((x) => x.id))); } catch { playable = new Set(); }
    const n = shown.length;
    const first = Math.max(0, shown.findIndex((x) => x.id === items[start]?.id));
    let idx = [...Array(n).keys()];
    if (opt.shuffle) {
      for (let i = n - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [idx[i], idx[j]] = [idx[j], idx[i]]; }
      idx = [first, ...idx.filter((i) => i !== first)];
    } else idx = [...idx.slice(first), ...idx.slice(0, first)];
    order = idx;
    pos = 0;
    paused = false;
    phase = "play";
    const win = getCurrentWindow();
    wasFull = await win.isFullscreen().catch(() => false);
    if (!wasFull) await win.setFullscreen(true).catch(() => {});
    slideshowAwake(true).catch(() => {});
    pickKind();
    startMusic();
    poke();
    tick().then(() => el?.focus());
  }

  // ---------- playing ----------
  function pickKind() {
    kind = opt.transition === "random" ? KINDS[Math.floor(Math.random() * KINDS.length)] : opt.transition;
  }
  function schedule() {
    clearTimeout(timer);
    if (paused || phase !== "play" || !cur) return;
    if (plays(cur)) return; // a playing video moves on when it ends
    timer = setTimeout(() => step(1), opt.seconds * 1000);
  }
  // Called when the current slide's picture is ready (or failed): its time starts now.
  function ready(id) {
    if (cur?.id === id) schedule();
    preloadNext();
  }
  function preloadNext() {
    const n = order[(pos + 1) % order.length];
    const e = shown[n];
    if (e && !isVideo(e)) new Image().src = fthumbUrl(e.id, "o");
  }
  function step(d) {
    clearTimeout(timer);
    if (!order.length) return;
    let p = pos + d;
    if (p >= order.length) {
      if (!opt.loop) return finish();
      p = 0;
    }
    if (p < 0) p = order.length - 1;
    pickKind();
    pos = p;
  }
  function finish() {
    phase = "end";
    controls = true;
    fadeMusic();
    slideshowAwake(false).catch(() => {});
  }
  function togglePause() {
    if (phase !== "play") return;
    paused = !paused;
    if (paused) {
      clearTimeout(timer);
      video?.pause();
      audio?.pause();
    } else {
      if (video) video.play().catch(() => {});
      if (musicOn && tracks.length) audio?.play().catch(() => {});
      schedule();
    }
    poke();
  }
  function speed(f) {
    opt.seconds = Math.min(60, Math.max(1, Math.round(opt.seconds * f)));
    save();
    schedule();
    poke();
  }
  function restart() {
    phase = "play";
    pos = 0;
    paused = false;
    slideshowAwake(true).catch(() => {});
    startMusic();
    poke();
    tick().then(() => el?.focus());
  }
  function close() {
    clearTimeout(timer);
    onclose?.(cur?.id ?? null);
  }

  // ---------- music ----------
  async function startMusic() {
    if (!tracks.length || !musicOn) return;
    await tick();
    if (!audio) return;
    audio.volume = (opt.volume / 100) * (ducked ? 0.2 : 1);
    audio.play().catch(() => {});
  }
  function nextTrack() {
    if (!tracks.length) return;
    track = (track + 1) % tracks.length;
    tick().then(() => audio?.play().catch(() => {}));
  }
  function toggleMusic() {
    musicOn = !musicOn;
    if (musicOn && !paused) audio?.play().catch(() => {});
    else audio?.pause();
    poke();
  }
  function fadeMusic() {
    if (!audio || audio.paused) return;
    const a = audio, v0 = a.volume;
    let t = 0;
    const iv = setInterval(() => {
      t += 1;
      a.volume = Math.max(0, v0 * (1 - t / 20));
      if (t >= 20) { clearInterval(iv); a.pause(); a.volume = v0; }
    }, 100);
  }
  // Music plays softly while a video with its own sound is on screen.
  const ducked = $derived(!!cur && plays(cur) && opt.sound);
  $effect(() => { if (audio) audio.volume = (opt.volume / 100) * (ducked ? 0.2 : 1); });

  // ---------- controls ----------
  function poke() {
    controls = true;
    clearTimeout(hideTimer);
    if (phase === "play" && !paused) hideTimer = setTimeout(() => (controls = false), 2500);
  }
  function onKey(e) {
    const k = e.key;
    if (phase === "setup") {
      if (k === "Escape") close();
      else if (k === "Enter" && !e.target.closest?.("button, select, input")) begin();
      else return;
    } else if (k === "Escape") close();
    else if (k === " " || k === "k") togglePause();
    else if (k === "ArrowRight" || k === "PageDown") { step(1); poke(); }
    else if (k === "ArrowLeft" || k === "PageUp") { step(-1); poke(); }
    else if (k === "+" || k === "=") speed(0.75);
    else if (k === "-") speed(1.35);
    else if (k === "m" || k === "M") toggleMusic();
    else if (k === "Home") { pos = 0; poke(); }
    else return;
    e.preventDefault();
    e.stopPropagation();
  }

  // ---------- transitions ----------
  function enter(node) {
    if (kind === "slide") return fly(node, { x: node.clientWidth || 800, duration: TRANSITION_MS, opacity: 1 });
    if (kind === "zoom") return scale(node, { start: 1.15, duration: TRANSITION_MS, opacity: 0 });
    if (kind === "none") return { duration: 0 };
    return fade(node, { duration: TRANSITION_MS });
  }
  function leave(node) {
    if (kind === "slide") return fly(node, { x: -(node.clientWidth || 800), duration: TRANSITION_MS, opacity: 1 });
    if (kind === "none") return { duration: 0 };
    return fade(node, { duration: TRANSITION_MS });
  }
  const dateOf = (e) => (e?.modified ? new Date(e.modified * 1000).toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" }) : "");
</script>

<div class="show slideshow" class:play={phase !== "setup"} class:nocursor={phase === "play" && !controls} bind:this={el} tabindex="0"
  role="dialog" aria-modal="true" aria-label="Slideshow" onkeydown={onKey} onpointermove={poke}>
  {#if phase === "setup"}
    <div class="card">
      <h2><Icon name="slideshow" size={22} /> Slideshow</h2>
      <p class="muted">{countText}. Nothing is changed in your files.</p>

      <label class="sl"><span>Show each photo for <b>{opt.seconds} seconds</b></span>
        <input type="range" min="1" max="30" step="1" bind:value={opt.seconds} /></label>

      <div class="lab">Change to the next photo with</div>
      <div class="seg">
        {#each [["none", "No effect"], ["fade", "Fade"], ["slide", "Slide"], ["zoom", "Zoom"], ["random", "Mixed"]] as [k, t]}
          <button class:on={opt.transition === k} onclick={() => (opt.transition = k)}>{t}</button>
        {/each}
      </div>

      <div class="lab">Videos</div>
      <div class="seg">
        {#each [["play", "Play them"], ["still", "Show a still"], ["skip", "Leave out"]] as [k, t]}
          <button class:on={opt.videos === k} onclick={() => (opt.videos = k)}>{t}</button>
        {/each}
      </div>
      {#if opt.videos === "play"}<label class="chk"><input type="checkbox" bind:checked={opt.sound} /> Video sound on</label>{/if}

      <div class="lab">Text on the photo</div>
      <div class="seg">
        {#each [["none", "None"], ["name", "File name"], ["date", "Name and date"]] as [k, t]}
          <button class:on={opt.caption === k} onclick={() => (opt.caption = k)}>{t}</button>
        {/each}
      </div>

      <div class="row">
        <label class="chk"><input type="checkbox" bind:checked={opt.shuffle} /> Random order</label>
        <label class="chk"><input type="checkbox" bind:checked={opt.loop} /> Start again at the end</label>
      </div>

      <div class="lab">Music</div>
      <div class="music">
        {#if tracks.length}
          <span class="songs" title={tracks.map((t) => t.name).join("\n")}>♪ {tracks.length === 1 ? tracks[0].name : `${tracks.length} songs`}</span>
          <button onclick={chooseMusic}>Change…</button>
          <button onclick={noMusic}>No music</button>
        {:else}
          <button onclick={chooseMusic}><Icon name="plus" size={16} /> Choose music…</button>
          <span class="muted small">MP3, M4A, WAV, OGG or FLAC from this PC</span>
        {/if}
      </div>
      {#if tracks.length}<label class="sl"><span>Music volume <b>{opt.volume} %</b></span><input type="range" min="0" max="100" bind:value={opt.volume} /></label>{/if}
      {#if note}<p class="err">{note}</p>{/if}

      <p class="keys muted small">While it plays: <kbd>Space</kbd> pause · <kbd>←</kbd> <kbd>→</kbd> back / next · <kbd>+</kbd> <kbd>−</kbd> faster / slower · <kbd>M</kbd> music on/off · <kbd>Esc</kbd> stop</p>
      <div class="btns">
        <button onclick={close}>Cancel</button>
        <button class="primary" onclick={begin} data-continue><Icon name="play" size={16} /> Start <kbd>Enter</kbd></button>
      </div>
    </div>
  {:else}
    <div class="stage">
      {#key `${pos}-${cur?.id}`}
        <div class="slide" in:enter out:leave>
          {#if cur && plays(cur)}
            <!-- svelte-ignore a11y_media_has_caption -->
            <video bind:this={video} src={mediaUrl(cur.id)} autoplay muted={!opt.sound} playsinline
              onended={() => step(1)} onerror={() => { playable.delete(cur.id); schedule(); }}
              onplaying={() => preloadNext()}></video>
          {:else if cur}
            <img class:kb={kind === "zoom" || opt.transition === "zoom"} style="--kb: {opt.seconds + 1}s"
              src={fthumbUrl(cur.id, isVideo(cur) ? "p" : "o")} alt={cur.name}
              onload={() => ready(cur.id)}
              onerror={(x) => { const im = x.currentTarget; if (!im.dataset.fallback) { im.dataset.fallback = "1"; im.src = fthumbUrl(cur.id, "p"); } else ready(cur.id); }} />
          {/if}
          {#if cur && opt.caption !== "none"}
            <div class="cap" class:up={controls || paused || phase === "end"}>{cur.name}{#if opt.caption === "date" && dateOf(cur)}<span>&nbsp;·&nbsp;{dateOf(cur)}</span>{/if}</div>
          {/if}
        </div>
      {/key}
    </div>

    {#if phase === "end"}
      <div class="endcard" transition:fade>
        <h2>The end</h2>
        <div class="btns"><button onclick={restart}><Icon name="play" size={16} /> Play again</button><button class="primary" onclick={close}>Close</button></div>
      </div>
    {/if}

    <div class="ctl" class:on={controls || paused || phase === "end"}>
      <button onclick={() => { step(-1); poke(); }} title="Back (←)"><Icon name="back" size={18} /></button>
      <button onclick={togglePause} title={paused ? "Play (Space)" : "Pause (Space)"}><Icon name={paused ? "play" : "pause"} size={18} /></button>
      <button onclick={() => { step(1); poke(); }} title="Next (→)"><Icon name="forward" size={18} /></button>
      <span class="pos">{pos + 1} / {order.length}</span>
      <button onclick={() => speed(1.35)} title="Slower (−)">−</button>
      <span class="secs">{opt.seconds} s</span>
      <button onclick={() => speed(0.75)} title="Faster (+)">+</button>
      {#if tracks.length}<button class:off={!musicOn} onclick={toggleMusic} title="Music on / off (M)">♪{musicOn ? "" : " off"}</button>{/if}
      {#if paused}<span class="paused">Paused</span>{/if}
      <button class="exit" onclick={close} title="Stop the slideshow (Esc)"><Icon name="close" size={18} /> Stop</button>
    </div>
  {/if}
  {#if tracks.length}
    <audio bind:this={audio} src={mediaUrl(tracks[track].id)} onended={nextTrack} preload="auto"></audio>
  {/if}
</div>

<style>
  .show { position: fixed; inset: 0; z-index: 60; background: rgba(0, 0, 0, 0.72); display: grid; place-items: center; outline: none; }
  .show.play { background: #000; }
  .show.nocursor { cursor: none; }
  .card { width: min(560px, calc(100vw - 32px)); max-height: calc(100vh - 32px); overflow: auto; background: var(--surface);
    color: var(--text); border: 1px solid var(--border); border-radius: 14px; padding: 20px 22px; box-shadow: 0 20px 60px #0008; }
  h2 { display: flex; align-items: center; gap: 8px; margin: 0 0 4px; font-size: 20px; }
  .muted { color: var(--muted); }
  .small { font-size: 12.5px; }
  .lab { margin: 14px 0 6px; font-weight: 600; font-size: 14px; }
  .sl { display: block; margin-top: 14px; }
  .sl span { display: flex; justify-content: space-between; font-size: 14px; font-weight: 600; }
  .sl b { font-weight: 500; }
  input[type="range"] { width: 100%; accent-color: var(--accent); }
  .seg { display: flex; gap: 6px; flex-wrap: wrap; }
  .seg button { flex: 1; min-width: 80px; padding: 7px 8px; }
  .seg button.on { background: var(--accent); border-color: var(--accent); color: var(--accent-text); }
  .chk { display: flex; align-items: center; gap: 8px; margin-top: 10px; font-size: 14px; }
  .row { display: flex; gap: 20px; flex-wrap: wrap; margin-top: 6px; }
  .music { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .songs { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .err { color: var(--red); font-size: 13px; }
  .keys { margin-top: 16px; }
  .btns { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
  .btns button { padding: 8px 16px; }

  .stage { position: absolute; inset: 0; overflow: hidden; }
  .slide { position: absolute; inset: 0; display: grid; place-items: center; }
  /* Fill the screen (small photos are enlarged), keeping the whole picture visible. */
  .slide img, .slide video { width: 100%; height: 100%; object-fit: contain; display: block; }
  .kb { animation: kb var(--kb, 6s) ease-out both; }
  @keyframes kb { from { transform: scale(1); } to { transform: scale(1.07); } }
  .cap { position: absolute; left: 0; right: 0; bottom: 28px; text-align: center; color: #fff; font-size: 20px;
    text-shadow: 0 1px 4px #000, 0 0 12px #000; pointer-events: none; }
  .cap span { opacity: 0.85; }
  .cap.up { bottom: 92px; }
  .cap { transition: bottom 0.3s; }

  .ctl { position: absolute; left: 50%; bottom: 22px; transform: translateX(-50%); display: flex; align-items: center; gap: 6px;
    background: rgba(20, 22, 27, 0.88); border: 1px solid #ffffff22; border-radius: 12px; padding: 8px 10px; color: #fff;
    opacity: 0; pointer-events: none; transition: opacity 0.3s; }
  .ctl.on { opacity: 1; pointer-events: auto; }
  .ctl button { background: #ffffff14; color: #fff; border: 1px solid #ffffff22; min-width: 36px; height: 36px; padding: 0 10px;
    display: inline-flex; align-items: center; gap: 6px; border-radius: 8px; font-size: 16px; }
  .ctl button:hover { background: #ffffff26; }
  .ctl button.off { opacity: 0.6; }
  .ctl .exit { margin-left: 8px; background: #c62828; border-color: #c62828; font-size: 14px; }
  .pos, .secs { font-variant-numeric: tabular-nums; min-width: 54px; text-align: center; font-size: 14px; }
  .secs { min-width: 36px; }
  .paused { color: #ffc83d; font-size: 14px; margin-left: 4px; }
  .endcard { position: absolute; inset: 0; display: grid; place-content: center; text-align: center; background: #000c; color: #fff; }
  .endcard h2 { justify-content: center; font-size: 32px; margin-bottom: 8px; }
</style>
