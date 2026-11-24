<script>
  // Large preview of one uncertain file, with the year decision right next to it.
  // Keys: ← / → previous / next, Enter confirm (and go to the next), Esc close.
  import { previewUrl } from "../lib/api.js";

  let { items, index = $bindable(), resolutions, suggested, evidence, onYear, onClose } = $props();

  let dlg;
  let input = $state(null);
  let year = $state("");
  let broken = $state({});
  let it = $derived(items[index]);
  let done = $derived(it ? resolutions[it.rel_path] : null);

  $effect(() => {
    dlg?.showModal();
  });
  // New file shown: pre-fill with the confirmed year (or nothing) and focus the box.
  $effect(() => {
    it;
    year = done ? String(done) : "";
    requestAnimationFrame(() => input?.focus());
  });

  const nameOf = (rel) => rel.slice(rel.lastIndexOf("/") + 1);
  const folderOf = (rel) => (rel.includes("/") ? rel.slice(0, rel.lastIndexOf("/")) : "(top folder)");
  const step = (d) => (index = Math.max(0, Math.min(items.length - 1, index + d)));

  function confirm(v) {
    if (!v) return;
    onYear(it, v);
    if (index < items.length - 1) step(1);
  }

  function onKey(e) {
    const typing = e.target === input && year !== "";
    if (e.key === "ArrowRight" && !typing) step(1);
    else if (e.key === "ArrowLeft" && !typing) step(-1);
    else if (e.key === "Enter" && !e.ctrlKey) confirm(year || suggested(it));
    else return;
    e.preventDefault();
    e.stopPropagation();
  }
</script>

<dialog bind:this={dlg} onclose={onClose} onkeydown={onKey} aria-label="Large preview">
  {#if it}
    <div class="box">
      <div class="pic">
        {#if !it.hash || broken[it.hash]}
          <div class="placeholder">{it.kind === "video" ? "▶ Video: no preview for this kind of video (the optional FFmpeg pack in Settings → AI models reads more kinds)" : "No preview for this file"}</div>
        {:else}
          {#key it.hash}
            <img src={previewUrl(it.hash)} alt={nameOf(it.rel_path)} onerror={() => (broken = { ...broken, [it.hash]: true })} />
          {/key}
        {/if}
        <button class="nav prev" disabled={index === 0} onclick={() => step(-1)} aria-label="Previous (←)">‹</button>
        <button class="nav next" disabled={index === items.length - 1} onclick={() => step(1)} aria-label="Next (→)">›</button>
      </div>

      <div class="side">
        <div class="row">
          <span class="muted">{index + 1} of {items.length}</span>
          <span class="spacer"></span>
          <button class="ghost close" onclick={() => dlg.close()} aria-label="Close (Esc)">✕ Close</button>
        </div>
        <h2 class="name">{nameOf(it.rel_path)}</h2>
        <p class="mono muted folder">{folderOf(it.rel_path)}</p>
        {#if it.flags.some((f) => f.startsWith("damaged"))}<span class="chip red">damaged file</span>{/if}

        <p class="reason">{it.decision.reason}</p>
        <div class="chips">
          {#each evidence(it, 20) as e}
            <span class="chip" class:accent={e.strong} class:ai={e.ai} class:distrusted={e.distrusted} title={e.note}>{e.label} {e.year}</span>
          {/each}
        </div>

        <div class="decide">
          <label for="lb-year"><strong>Year this photo was taken</strong></label>
          <div class="row">
            <input id="lb-year" type="number" bind:this={input} bind:value={year} placeholder={suggested(it) ?? "Year"} />
            <button class="primary" disabled={!(year || suggested(it))} onclick={() => confirm(year || suggested(it))}>
              ✓ {year || suggested(it) || "Confirm"}
            </button>
          </div>
          {#if done}
            <div class="row">
              <span class="chip green">Confirmed: {done}</span>
              <button class="ghost small" onclick={() => onYear(it, "")}>Undo</button>
            </div>
          {:else}
            <span class="muted small">Not confirmed yet: this file stays where it is for now.</span>
          {/if}
        </div>

        <p class="muted small keys"><kbd>←</kbd> <kbd>→</kbd> previous / next · <kbd>Enter</kbd> confirm and next · <kbd>Esc</kbd> close</p>
      </div>
    </div>
  {/if}
</dialog>

<style>
  dialog {
    width: min(1400px, 94vw); height: min(900px, 90vh); padding: 0; border: 1px solid var(--border);
    border-radius: var(--radius); background: var(--surface); color: var(--text); box-shadow: 0 20px 60px rgba(0, 0, 0, 0.45);
  }
  dialog::backdrop { background: rgba(0, 0, 0, 0.65); }
  .box { display: grid; grid-template-columns: 1fr 340px; height: 100%; }
  .pic { position: relative; background: #111; display: grid; place-items: center; min-height: 0; overflow: hidden; }
  .pic img { max-width: 100%; max-height: 100%; object-fit: contain; }
  .placeholder { color: #bbb; font-weight: 600; }
  .nav {
    position: absolute; top: 50%; transform: translateY(-50%); width: 52px; height: 84px; font-size: 40px; line-height: 1;
    background: rgba(0, 0, 0, 0.45); color: #fff; border: none; border-radius: 10px;
  }
  .nav:hover:not(:disabled) { background: rgba(0, 0, 0, 0.7); }
  .nav:active:not(:disabled) { transform: translateY(-50%); }
  .nav:disabled { opacity: 0.15; }
  .prev { left: 12px; }
  .next { right: 12px; }
  .side { padding: 16px 18px; display: flex; flex-direction: column; gap: 10px; overflow: auto; border-left: 1px solid var(--border); }
  .name { font-size: 18px; overflow-wrap: anywhere; }
  .folder { overflow-wrap: anywhere; }
  .reason { font-size: 14px; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; }
  .chip.distrusted { text-decoration: line-through; opacity: 0.7; }
  .decide { display: grid; gap: 8px; padding: 14px; background: var(--surface-2); border-radius: var(--radius-sm); margin-top: 6px; }
  .decide input { width: 110px; font-size: 18px; padding: 7px 10px; }
  .decide .primary { font-size: 15px; padding: 9px 16px; }
  .small { font-size: 12.5px; }
  .keys { margin-top: auto; }
  kbd { font: 12px var(--mono); border: 1px solid var(--border); border-bottom-width: 2px; border-radius: 4px; padding: 0 6px; background: var(--surface); }
  .chip.ai { background: var(--amber-soft); color: var(--amber); }
</style>
