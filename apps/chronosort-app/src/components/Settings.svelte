<script>
  import { prefs, resetPrefs, THEMES, FONTS, SIZES } from "../lib/prefs.svelte.js";
  import AiModels from "./AiModels.svelte";
  import Portable from "./Portable.svelte";

  // Small colour previews (bg, surface, accent, text) — keep in sync with app.css.
  const SWATCH = {
    system: ["linear-gradient(135deg, #f6f5f2 50%, #141518 50%)", "#ffffff", "#2357c6", "#1d1f23"],
    light: ["#f6f5f2", "#ffffff", "#2357c6", "#1d1f23"],
    dark: ["#141518", "#1c1e22", "#6f9bff", "#eceef1"],
    navy: ["#132137", "#1d304d", "#ffc46b", "#e9f1fb"],
    pastel: ["#292633", "#383345", "#f2c48d", "#f1eaf4"],
    contrast: ["#000000", "#000000", "#ffff00", "#ffffff"],
  };

</script>

<section class="wrap">
  <h1>Settings</h1>

  <div class="card block">
    <h2>Colours</h2>
    <div class="themes">
      {#each THEMES as t}
        {@const [bg, surf, acc, txt] = SWATCH[t.id]}
        <button class="theme" class:on={prefs.theme === t.id} onclick={() => (prefs.theme = t.id)} aria-pressed={prefs.theme === t.id}>
          <span class="sw" style="background:{bg}">
            <span class="sw-card" style="background:{surf}; color:{txt}">Aa<span class="sw-acc" style="background:{acc}"></span></span>
          </span>
          <strong>{t.label}</strong>
          <span class="muted small">{t.hint}</span>
        </button>
      {/each}
    </div>
  </div>

  <div class="card block two">
    <div>
      <h2>Text size</h2>
      <div class="sizes">
        {#each SIZES as s, i}
          <button class:on={prefs.size === s.id} onclick={() => (prefs.size = s.id)} aria-pressed={prefs.size === s.id} style="font-size:{13 + i * 2.5}px">{s.label}</button>
        {/each}
      </div>
      <p class="muted small">Makes everything bigger, including buttons and pictures.</p>
    </div>
    <div>
      <h2><label for="font">Font</label></h2>
      <select id="font" bind:value={prefs.font}>
        {#each FONTS as f}<option value={f.id}>{f.label}</option>{/each}
      </select>
      <p class="sample">The quick brown fox jumps over the lazy dog. 2005 · 1987</p>
    </div>
  </div>

  <AiModels />

  <Portable />

  <div class="row">
    <span class="muted small">Settings are saved automatically (in portable mode: on the stick).</span>
    <span class="spacer"></span>
    <button onclick={resetPrefs}>Reset to defaults</button>
  </div>
</section>

<style>
  .wrap { max-width: 960px; margin: 0 auto; padding: 28px 24px 48px; display: grid; gap: 16px; }
  .block { padding: 18px 22px; display: grid; gap: 12px; }
  .two { grid-template-columns: 1fr 1fr; gap: 28px; }
  .two > div { display: grid; gap: 10px; align-content: start; }
  .small { font-size: 12.5px; }
  .themes { display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; }
  .theme { display: grid; gap: 4px; justify-items: start; text-align: left; padding: 10px; border-width: 1.5px; }
  .theme.on { border-color: var(--accent); background: var(--accent-soft); }
  .sw { width: 100%; height: 64px; border-radius: 8px; display: grid; place-items: center; border: 1px solid var(--border); margin-bottom: 4px; }
  .sw-card { padding: 6px 12px; border-radius: 6px; font-weight: 650; display: flex; gap: 8px; align-items: center; font-size: 15px; }
  .sw-acc { width: 26px; height: 12px; border-radius: 4px; }
  .sizes { display: flex; gap: 8px; flex-wrap: wrap; }
  .sizes button.on { border-color: var(--accent); background: var(--accent-soft); font-weight: 600; }
  select { font: inherit; color: var(--text); background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-sm); padding: 8px 10px; }
  .sample { padding: 10px 12px; background: var(--surface-2); border-radius: var(--radius-sm); font-size: 15px; }
</style>
