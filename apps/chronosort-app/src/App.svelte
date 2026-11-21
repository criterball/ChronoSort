<script>
  import { onMount } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import Home from "./components/Home.svelte";
  import SortFlow from "./components/SortFlow.svelte";
  import Duplicates from "./components/Duplicates.svelte";
  import People from "./components/People.svelte";
  import Places from "./components/Places.svelte";
  import Settings from "./components/Settings.svelte";
  import Help from "./components/Help.svelte";
  import { app, sort, dl, onDownloadProgress, timeLeft } from "./lib/state.svelte.js";
  import { prefs, applyPrefs, SIZES } from "./lib/prefs.svelte.js";
  import { exitApp, confirmBox, startupArgs, onProgress } from "./lib/api.js";

  const TITLES = { sort: "Sort into year folders", dups: "Find duplicates", people: "People", places: "Add a place", settings: "Settings", help: "Help" };

  // Apply theme/font now and whenever they change; text size uses real page zoom
  // so every layout scales cleanly.
  $effect(() => {
    prefs.theme; prefs.font; prefs.size;
    applyPrefs();
    const z = SIZES.find((s) => s.id === prefs.size)?.zoom ?? 1;
    getCurrentWebview().setZoom(z).catch(() => {});
  });

  onMount(async () => {
    onProgress("model-progress", (p) => dl.pack && onDownloadProgress(p));
    // Command-line start options go straight into the sort flow.
    const a = await startupArgs().catch(() => ({}));
    if (a.source || a.dest) {
      if (a.source) sort.settings.source = a.source;
      if (a.dest) sort.settings.dest = a.dest;
      sort.settings.goto = a.goto;
      sort.autostart = a.analyze;
      app.view = "sort";
    }
  });

  let notice = $state("");
  let noticeTimer;
  function say(msg) {
    notice = msg;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => (notice = ""), 4000);
  }

  export function go(view) {
    if (view === app.view) return;
    // Settings and Help are safe to open at any time; a running job keeps
    // running and its screen is kept.
    if (app.busy && !(view === "settings" || view === "help" || view === "home")) {
      say("Please wait until the current job has finished.");
      return;
    }
    app.view = view;
  }

  async function exit() {
    if (dl.pack) {
      const ok = await confirmBox(
        `“${dl.name}” is still downloading.\n\nIf you exit now the download stops. Nothing is lost: next time, click Download again and it continues where it stopped.\n\nExit anyway?`,
        "Exit ChronoSort", "Exit", "Keep downloading",
      );
      if (!ok) return;
    } else if (app.busy) {
      const ok = await confirmBox(
        "A job is still running. If you exit now it stops part-way; your originals stay safe and the next run picks up where it left off.\n\nExit anyway?",
        "Exit ChronoSort",
        "Exit",
        "Keep working",
      );
      if (!ok) return;
    }
    exitApp();
  }

  function onKey(e) {
    const ctrl = e.ctrlKey || e.metaKey;
    const k = e.key;
    let handled = true;
    if (k === "F1") go("help");
    else if (k === "Escape" && !document.querySelector("dialog[open]")) {
      if (app.view !== "home") go("home");
      else handled = false;
    } else if (e.altKey && k === "Home") go("home");
    else if (ctrl && k === ",") go("settings");
    else if (ctrl && (k === "q" || k === "Q")) exit();
    else if (ctrl && k === "1") go("sort");
    else if (ctrl && k === "2") go("dups");
    else if (ctrl && k === "3") go("people");
    else if (ctrl && k === "4") go("places");
    else if (ctrl && k === "Enter" && !document.querySelector("dialog[open]")) {
      // Every screen marks its main "continue" button with data-continue.
      // Screens stay mounted while hidden, so only count a visible button.
      const b = [...document.querySelectorAll("main [data-continue]:not(:disabled)")].find((x) => x.offsetParent);
      if (b) b.click();
      else handled = false;
    } else handled = false;
    if (handled) e.preventDefault();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="shell">
  <header>
    <button class="brand ghost" onclick={() => go("home")} title="Main menu (Esc)">
      <svg viewBox="0 0 32 32" width="28" height="28" aria-hidden="true">
        <rect x="1" y="1" width="30" height="30" rx="8" fill="var(--accent)" />
        <circle cx="16" cy="16" r="8.5" fill="none" stroke="var(--accent-text)" stroke-width="2.2" />
        <path d="M16 16 L16 10.5 M16 16 L12.5 14" stroke="var(--accent-text)" stroke-width="2.2" stroke-linecap="round" />
      </svg>
      <span>ChronoSort</span>
    </button>
    {#if app.view !== "home"}<span class="where">{TITLES[app.view]}</span>{/if}
    <span class="spacer"></span>
    {#if dl.pack}
      {@const p = dl.prog}
      <button class="chip amber dlchip" onclick={() => go("settings")} title="Downloading {dl.name}. Click to see it in Settings.">
        <span class="spin" aria-hidden="true"></span>
        {#if p && p.phase === "check"}Checking the download… {p.total ? Math.floor((100 * p.done) / p.total) : 0}%
        {:else if p && p.total}Downloading AI model {Math.floor((100 * p.done) / p.total)}%{#if p.files > 1} (file {p.index + 1} of {p.files}){/if}{#if timeLeft()}{" · " + timeLeft()}{/if}
        {:else}Starting the download…{/if}
      </button>
    {:else if app.busy}<span class="chip amber"><span class="spin" aria-hidden="true"></span> Working…</span>{/if}
    <span class="chip green" title="Your photos and videos never leave this PC, and the AI runs on this PC. The internet is only used if you choose to download an AI model.">🔒 Photos stay on this PC</span>
    <nav class="row" aria-label="Main">
      <button class="hbtn" class:on={app.view === "home"} onclick={() => go("home")} title="Main menu (Esc)">
        <span aria-hidden="true">⌂</span> Home
      </button>
      <button class="hbtn" class:on={app.view === "help"} onclick={() => go("help")} title="Help (F1)">
        <span aria-hidden="true">?</span> Help
      </button>
      <button class="hbtn" class:on={app.view === "settings"} onclick={() => go("settings")} title="Settings (Ctrl+,)">
        <span aria-hidden="true">⚙</span> Settings
      </button>
      <button class="hbtn exit" onclick={exit} title="Exit ChronoSort (Ctrl+Q)">
        <span aria-hidden="true">✕</span> Exit
      </button>
    </nav>
  </header>

  {#if notice}<div class="notice" role="status">{notice}</div>{/if}

  <main>
    <!-- The sort and duplicate screens stay mounted (hidden) so a running job keeps its screen. -->
    <div class="view" hidden={app.view !== "home"}><Home {go} /></div>
    <div class="view" hidden={app.view !== "sort"}><SortFlow active={app.view === "sort"} /></div>
    <div class="view" hidden={app.view !== "dups"}><Duplicates active={app.view === "dups"} /></div>
    <div class="view" hidden={app.view !== "people"}><People active={app.view === "people"} {go} /></div>
    <div class="view" hidden={app.view !== "places"}><Places active={app.view === "places"} /></div>
    {#if app.view === "settings"}<Settings />{/if}
    {#if app.view === "help"}<Help />{/if}
  </main>
</div>

<style>
  .shell { display: flex; flex-direction: column; height: 100%; }
  header {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 10px 18px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .brand { display: flex; align-items: center; gap: 10px; font-weight: 650; font-size: 17px; padding: 4px 8px; }
  .where { color: var(--muted); font-size: 15px; padding-left: 12px; border-left: 1px solid var(--border); }
  nav { gap: 6px; }
  .hbtn { display: flex; align-items: center; gap: 7px; padding: 8px 14px; font-size: 15px; font-weight: 550; }
  .hbtn span { font-size: 16px; line-height: 1; }
  .hbtn.on { background: var(--accent-soft); border-color: var(--accent); color: var(--text); }
  .exit { background: var(--red); border-color: var(--red); color: var(--surface); font-weight: 650; padding: 8px 18px; }
  .exit:hover:not(:disabled) { filter: brightness(1.08); border-color: var(--red); }
  .dlchip { border: none; cursor: pointer; font-weight: 600; }
  .notice {
    margin: 10px auto 0; padding: 8px 16px; border-radius: var(--radius-sm);
    background: var(--amber-soft); color: var(--amber); font-weight: 600;
  }
  main { flex: 1; overflow: auto; }
  .view[hidden] { display: none; }
</style>
