<script>
  import { onMount } from "svelte";
  import { sort, dups, people, places } from "../lib/state.svelte.js";
  import { aiOverview, formatBytes } from "../lib/api.js";

  let { go } = $props();

  // Offer the AI parts this PC is suited for, once they are not all there.
  // Nothing is downloaded from here: the button only opens Settings.
  const NEVER = "chronosort.aiOffer.never";
  let offer = $state(null); // { bytes, names }
  let later = $state(false);
  onMount(async () => {
    if (localStorage.getItem(NEVER)) return;
    try {
      const ov = await aiOverview();
      if (ov.assessment.tier === "metadata") return;
      const missing = Object.values(ov.recommended)
        .map((id) => ov.packs.find((p) => p.id === id))
        .filter((p) => p && p.status !== "ready" && p.downloadable);
      if (missing.length) {
        offer = { bytes: missing.reduce((n, p) => n + p.approx_bytes, 0), names: missing.map((p) => p.name) };
      }
    } catch {
      /* no offer if the check fails */
    }
  });

  const tiles = [
    { view: "sort", icon: "cal", title: "Sort photos into year folders", text: "Copy photos and videos into folders like 2005, 2006 … by the date they were taken.", key: "Ctrl+1" },
    { view: "dups", icon: "dup", title: "Find duplicates", text: "Find identical copies of the same photo and decide what to do with the extras.", key: "Ctrl+2" },
    { view: "people", icon: "ppl", title: "People", text: "Name the people in your photos. ChronoSort learns their faces, on this PC only.", key: "Ctrl+3" },
    { view: "places", icon: "pin", title: "Add a place", text: "Write the town or country into many photos at once, for maps and searching.", key: "Ctrl+4" },
    { view: "settings", icon: "⚙", title: "Settings", text: "Colours, font, text size, and the local AI.", key: "Ctrl+," },
    { view: "help", icon: "?", title: "Help", text: "How it works, how your photos are kept safe, and shortcut keys.", key: "F1" },
  ];
  const inProgress = (v) => (v === "sort" && sort.analysis) || (v === "dups" && dups.phase !== "pick") || (v === "people" && people.phase === "results") || (v === "places" && places.phase !== "pick");
</script>

<section class="wrap">
  <div class="hello">
    <h1>What would you like to do?</h1>
    <p class="muted">Your photos are never changed: ChronoSort makes verified copies, and everything can be undone.</p>
  </div>

  {#if offer && !later}
    <div class="card offer">
      <div class="otxt">
        <strong>Set up the local AI for this PC?</strong>
        <span class="muted">
          It can recognise faces and read dates on old prints, entirely on this PC; your photos never leave it.
          Still needed: {offer.names.join(", ")} (about {formatBytes(offer.bytes)}, downloaded once).
        </span>
      </div>
      <div class="obtn">
        <button class="primary" onclick={() => go("settings")}>Set up AI…</button>
        <button class="ghost" onclick={() => (later = true)}>Not now</button>
        <button class="ghost small" onclick={() => { localStorage.setItem(NEVER, "1"); offer = null; }}>Don't ask again</button>
      </div>
    </div>
  {/if}

  <div class="tiles">
    {#each tiles as t, i}
      <button class="tile card" class:main={i < 2} onclick={() => go(t.view)}>
        <span class="icon" aria-hidden="true">
          {#if t.icon === "cal"}
            <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><rect x="3" y="5" width="18" height="16" rx="2.5"/><path d="M3 10h18M8 3v4M16 3v4"/><path d="M7.5 14h2M11 14h2M14.5 14h2M7.5 17.5h2M11 17.5h2"/></svg>
          {:else if t.icon === "dup"}
            <svg viewBox="0 0 24 24" width="40" height="40" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><rect x="3" y="3" width="13" height="13" rx="2.5"/><rect x="8" y="8" width="13" height="13" rx="2.5" fill="var(--accent)"/><path d="M12 14.5h5M14.5 12v5" stroke-linecap="round"/></svg>
          {:else if t.icon === "pin"}
            <svg viewBox="0 0 24 24" width="34" height="34" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linejoin="round"><path d="M12 21s-6.5-6.1-6.5-11.2a6.5 6.5 0 0 1 13 0C18.5 14.9 12 21 12 21z"/><circle cx="12" cy="9.8" r="2.4"/></svg>
          {:else if t.icon === "ppl"}
            <svg viewBox="0 0 24 24" width="34" height="34" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><circle cx="9" cy="8" r="3.5"/><path d="M2.5 20c0-3.6 2.9-6 6.5-6s6.5 2.4 6.5 6"/><circle cx="17" cy="9" r="2.7"/><path d="M16.5 14.2c2.9.2 5 2.2 5 5.3"/></svg>
          {:else}{t.icon}{/if}
        </span>
        <span class="txt">
          <strong>{t.title}</strong>
          <span class="muted">{t.text}</span>
          {#if inProgress(t.view)}<span class="chip accent">Continue where you left off</span>{/if}
        </span>
        <kbd>{t.key}</kbd>
      </button>
    {/each}
  </div>
</section>

<style>
  .wrap { max-width: 960px; margin: 0 auto; padding: 36px 24px 40px; display: grid; gap: 26px; }
  .hello { display: grid; gap: 6px; text-align: center; }
  .hello h1 { font-size: 28px; }
  .hello p { font-size: 15.5px; }
  .offer { display: flex; gap: 18px; align-items: center; padding: 16px 20px; border-color: var(--accent); border-width: 1.5px; }
  .otxt { display: grid; gap: 4px; flex: 1; }
  .otxt strong { font-size: 16px; }
  .obtn { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; justify-content: flex-end; }
  /* Two big tiles on top, the others three across. */
  .tiles { display: grid; grid-template-columns: repeat(6, 1fr); gap: 18px; }
  .tile { grid-column: span 3; }
  .tile.main { grid-column: span 3; }
  .tile {
    display: grid; grid-template-columns: auto 1fr; gap: 18px; align-items: center;
    text-align: left; padding: 22px 24px; position: relative; border-width: 1.5px;
  }
  .tile.main { padding: 30px 26px; }
  .tile:hover:not(:disabled) { border-color: var(--accent); background: var(--accent-soft); }
  .icon {
    width: 64px; height: 64px; border-radius: 16px; display: grid; place-items: center;
    background: var(--surface-2); font-size: 32px; color: var(--accent); font-weight: 700;
  }
  .main .icon { width: 76px; height: 76px; font-size: 38px; background: var(--accent); color: var(--accent-text); }
  .txt { display: grid; gap: 6px; justify-items: start; }
  .txt strong { font-size: 19px; }
  .main .txt strong { font-size: 21px; }
  .txt .muted { font-size: 14.5px; }
  kbd {
    position: absolute; top: 12px; right: 14px; font: 12px var(--mono); color: var(--muted);
    border: 1px solid var(--border); border-radius: 5px; padding: 1px 6px;
  }
</style>
