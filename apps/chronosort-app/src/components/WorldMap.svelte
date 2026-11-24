<script>
  // A small offline map (Natural Earth outline, built in) with a mark for a place.
  // kind "town": a pin at the exact position. "area" / "country": a dashed ring
  // around the rough middle — only a picture, nothing about it is written to photos.
  import { LAND } from "../lib/worldmap.js";

  let { lat, lon, kind = "town", label = "" } = $props();

  // Width of the shown window in degrees (height is half of it).
  const SPAN = { town: 36, area: 60, country: 110 };
  let view = $derived.by(() => {
    const w = SPAN[kind] ?? 36, h = w / 2;
    const x = Math.min(Math.max(lon + 180 - w / 2, 0), 360 - w);
    const y = Math.min(Math.max(90 - lat - h / 2, 0), 180 - h);
    return { x, y, w, h };
  });
  let px = $derived(lon + 180);
  let py = $derived(90 - lat);
  // Mark sizes in degrees, so they look the same at every zoom.
  let u = $derived(view.w / 100);
</script>

<figure class="map">
  <svg viewBox="{view.x} {view.y} {view.w} {view.h}" role="img" aria-label={label ? `Map: ${label}` : "Map"} preserveAspectRatio="xMidYMid slice">
    <rect x="0" y="0" width="360" height="180" class="sea" />
    <path d={LAND} class="land" />
    {#if kind === "town"}
      <circle cx={px} cy={py} r={u * 4} class="halo" />
      <circle cx={px} cy={py} r={u * 1.6} class="pin" />
    {:else}
      <circle cx={px} cy={py} r={u * (kind === "country" ? 10 : 8)} class="ring" stroke-dasharray="{u * 2} {u * 1.5}" stroke-width={u * 0.8} />
      <circle cx={px} cy={py} r={u * 0.9} class="pin" />
    {/if}
  </svg>
  <figcaption class="muted small">
    {#if kind === "town"}{label}{:else}{label} · rough middle shown; no map position is written{/if}
  </figcaption>
</figure>

<style>
  /* Theme tokens only, so every theme (light, dark, navy, pastel, contrast) works. */
  .map { margin: 0.5rem 0 0; }
  svg { display: block; width: 100%; max-width: 360px; aspect-ratio: 2 / 1; border-radius: var(--radius-sm); border: 1px solid var(--border); }
  .sea { fill: color-mix(in srgb, var(--accent) 16%, var(--surface)); }
  .land { fill: color-mix(in srgb, var(--text) 16%, var(--surface)); stroke: var(--muted); stroke-width: 0.6; vector-effect: non-scaling-stroke; }
  .pin { fill: var(--red); stroke: var(--surface); stroke-width: 1.5; vector-effect: non-scaling-stroke; }
  .halo { fill: var(--red); opacity: 0.2; }
  .ring { fill: var(--red-soft); fill-opacity: 0.6; stroke: var(--red); }
  figcaption { margin-top: 0.25rem; }
</style>
