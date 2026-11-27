<script>
  import { onMount, tick } from "svelte";
  import Icon from "./Icon.svelte";

  // items: [{ label, icon?, key?, action, danger?, disabled?, bold? } | "-"]
  //   or a star row: { stars: current (0–5, -1 = mixed), label, action(n) }
  let { items, x, y, onclose } = $props();

  let el = $state();
  // svelte-ignore state_referenced_locally
  let pos = $state({ left: x, top: y }); // first guess; adjusted on mount
  let active = $state(-1);
  const real = $derived(items.map((it, i) => ({ it, i })).filter(({ it }) => it !== "-" && !it.disabled && it.stars == null));

  onMount(async () => {
    await tick();
    // Keep the menu on screen (open upwards/leftwards near the edges).
    const r = el.getBoundingClientRect();
    const vw = window.innerWidth, vh = window.innerHeight;
    pos = { left: x + r.width > vw - 4 ? Math.max(4, x - r.width) : x, top: y + r.height > vh - 4 ? Math.max(4, vh - r.height - 4) : y };
    el.focus();
  });

  function run(it) {
    if (!it || it.disabled) return;
    onclose();
    it.action();
  }

  function onKey(e) {
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); onclose(); return; }
    const n = real.length;
    if (!n) return;
    const cur = real.findIndex((r) => r.i === active);
    if (e.key === "ArrowDown") { e.preventDefault(); active = real[(cur + 1 + n) % n].i; }
    else if (e.key === "ArrowUp") { e.preventDefault(); active = real[(cur - 1 + n) % n].i; }
    else if (e.key === "Enter" && active >= 0) { e.preventDefault(); e.stopPropagation(); run(items[active]); }
  }
</script>

<svelte:window onmousedown={(e) => el && !el.contains(e.target) && onclose()} onblur={onclose} onresize={onclose} />

<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div class="menu" role="menu" tabindex="0" bind:this={el} style="left:{pos.left}px; top:{pos.top}px" onkeydown={onKey}
  oncontextmenu={(e) => e.preventDefault()}>
  {#each items as it, i}
    {#if it === "-"}
      <div class="sep" role="separator"></div>
    {:else if it.stars != null}
      <div class="starrow" role="group" aria-label={it.label}>
        <span class="lbl">{it.label}</span>
        {#each [1, 2, 3, 4, 5] as n}
          <button class="st" class:on={it.stars >= n} title="{n} {n === 1 ? 'star' : 'stars'} (key {n})" onclick={() => { onclose(); it.action(n); }}>★</button>
        {/each}
        <button class="st none" title="No stars (key 0)" disabled={it.stars === 0} onclick={() => { onclose(); it.action(0); }}>✕</button>
      </div>
    {:else}
      <button role="menuitem" class="item" class:danger={it.danger} class:bold={it.bold} class:active={active === i} disabled={it.disabled}
        onmouseenter={() => (active = i)} onclick={() => run(it)}>
        <span class="ic">{#if it.icon}<Icon name={it.icon} size={16} />{/if}</span>
        <span class="lbl">{it.label}</span>
        {#if it.key}<span class="key">{it.key}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .menu {
    position: fixed; z-index: 1000; min-width: 230px; padding: 4px;
    background: var(--surface); border: 1px solid var(--border); border-radius: 8px;
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.22); outline: none; font-size: 13.5px;
  }
  .item {
    display: grid; grid-template-columns: 22px 1fr auto; align-items: center; gap: 8px; width: 100%;
    padding: 5px 10px 5px 6px; border: none; border-radius: 5px; background: none; text-align: left; color: var(--text); font-weight: 400;
  }
  .item.active:not(:disabled) { background: var(--accent); color: var(--accent-text); }
  .item.active:not(:disabled) .key { color: inherit; opacity: 0.85; }
  .item:disabled { opacity: 0.45; }
  .item.danger { color: var(--red); }
  .item.danger.active:not(:disabled) { background: var(--red); color: #fff; }
  .item.bold .lbl { font-weight: 650; }
  .ic { display: grid; place-items: center; color: var(--muted); }
  .item.active .ic { color: inherit; }
  .key { color: var(--muted); font-size: 12px; padding-left: 18px; }
  .starrow { display: flex; align-items: center; gap: 1px; padding: 3px 6px 3px 36px; }
  .starrow .lbl { flex: 1; color: var(--muted); font-size: 12.5px; margin-right: 8px; }
  .st { width: 24px; height: 24px; padding: 0; border: none; border-radius: 4px; background: none; color: var(--muted); font-size: 16px; line-height: 1; }
  .st.on { color: #e0a800; }
  .st:hover:not(:disabled) { background: var(--accent-soft); color: #e0a800; }
  .st.none { font-size: 12px; margin-left: 4px; }
  .st:disabled { opacity: 0.35; }
  .sep { height: 1px; background: var(--border); margin: 4px 6px; }
</style>
