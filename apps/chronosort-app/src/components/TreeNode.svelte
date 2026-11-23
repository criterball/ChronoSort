<script>
  import TreeNode from "./TreeNode.svelte";
  import Icon from "./Icon.svelte";
  import { listSubdirs } from "../lib/api.js";
  import { bprefs } from "../lib/state.svelte.js";
  import { samePath, isInside, joinIn } from "../lib/paths.js";

  // node: { path, label, icon, hint? }
  // follow: unfold towards the browsed folder (only the closest root does, so Home and Pictures do not both open).
  let { node, depth = 0, current, sep, onopen, oncontext, refreshKey = 0, follow = true, ondragto, ondropto } = $props();
  let over = $state(false);

  let open = $state(false);
  let kids = $state(null); // null = not loaded; [] = no sub-folders
  let failed = $state(false);
  let loadedKey = -1;

  async function load() {
    try {
      const subs = await listSubdirs(node.path, bprefs.showHidden);
      kids = subs.map((e) => ({ path: joinIn(node.path, e.name, sep), label: e.name, icon: "folder" }));
      failed = false;
    } catch {
      kids = [];
      failed = true;
    }
    loadedKey = refreshKey;
  }

  async function toggle(e) {
    e?.stopPropagation();
    open = !open;
    if (open && kids === null) await load();
  }

  // Follow the browsed folder: unfold every ancestor of it, once per folder change
  // (so the user can still fold a branch by hand).
  let followed = null;
  $effect(() => {
    const c = current, f = follow;
    if (c === followed) return;
    followed = c;
    if (f && c && isInside(c, node.path, sep) && !samePath(c, node.path, sep) && !open) {
      open = true;
      if (kids === null) load();
    }
  });
  // Re-read after housekeeping (new folder, rename, delete …).
  $effect(() => {
    if (refreshKey !== loadedKey && kids !== null && open) load();
  });

  const here = $derived(samePath(current, node.path, sep));
  let rowEl = $state();
  // Keep the current folder visible in the tree.
  $effect(() => {
    if (here && rowEl) requestAnimationFrame(() => rowEl.scrollIntoView({ block: "nearest" }));
  });
</script>

<li role="treeitem" aria-expanded={kids && kids.length ? open : undefined} aria-selected={here}>
  <div class="row" bind:this={rowEl} class:here style="padding-left:{4 + depth * 16}px" title={node.hint ?? node.path}
    onclick={() => onopen(node.path)} ondblclick={toggle} oncontextmenu={(e) => oncontext?.(e, node)}
    class:over ondragover={(e) => { ondragto?.(e, node); over = e.defaultPrevented; }} ondragleave={() => (over = false)}
    ondrop={(e) => { over = false; ondropto?.(e, node); }}
    onkeydown={(e) => e.key === "Enter" && onopen(node.path)} role="button" tabindex="-1">
    <button class="tw" tabindex="-1" onclick={toggle} aria-label={open ? "Fold" : "Unfold"}
      style:visibility={kids && kids.length === 0 ? "hidden" : "visible"}>
      <Icon name={open ? "chev-down" : "chev-right"} size={13} />
    </button>
    <span class="ic" class:k={node.icon === "folder"}><Icon name={node.icon} size={node.icon === "folder" ? 17 : 16} /></span>
    <span class="lbl">{node.label}</span>
    {#if failed}<span class="lock" title="This folder cannot be opened">🔒</span>{/if}
  </div>
  {#if open && kids && kids.length}
    <ul role="group">
      {#each kids as k (k.path)}
        <TreeNode node={k} depth={depth + 1} {current} {sep} {onopen} {oncontext} {refreshKey} {ondragto} {ondropto} />
      {/each}
    </ul>
  {/if}
</li>

<style>
  li { list-style: none; }
  ul { margin: 0; padding: 0; }
  .row {
    display: flex; align-items: center; gap: 3px; height: 24px; padding-right: 8px; cursor: default;
    white-space: nowrap; border-radius: 4px; font-size: 13.5px; user-select: none;
  }
  .row:hover { background: var(--surface-2); }
  .row.here { background: var(--accent); color: var(--accent-text); }
  .row.over { outline: 2px dashed var(--accent); background: var(--accent-soft); }
  .row.here .ic { color: inherit; }
  .tw { border: none; background: none; padding: 0; width: 16px; height: 16px; display: grid; place-items: center; color: var(--muted); }
  .row.here .tw { color: inherit; }
  .ic { display: grid; place-items: center; color: var(--muted); width: 20px; }
  .lbl { overflow: hidden; text-overflow: ellipsis; }
  .lock { font-size: 11px; opacity: 0.6; }
</style>
