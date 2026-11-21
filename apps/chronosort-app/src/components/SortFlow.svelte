<script>
  import Setup from "./Setup.svelte";
  import Overview from "./Overview.svelte";
  import Review from "./Review.svelte";
  import Naming from "./Naming.svelte";
  import PlanView from "./PlanView.svelte";
  import RunView from "./RunView.svelte";
  import { app, sort } from "../lib/state.svelte.js";

  let { active } = $props();

  const STEPS = ["Folders", "Overview", "Check", "File names", "Final check", "Copy"];

  // Which steps can be revisited: nothing past Folders before an analysis,
  // and the Copy step only through its Start button.
  let maxStep = $derived(sort.step === 5 ? 5 : sort.analysis ? 4 : 0);
  let locked = $derived(app.busy || sort.step === 5);

  function go(n) {
    if (!locked && n <= maxStep) sort.step = n;
  }
</script>

<div class="flow">
  <nav aria-label="Steps" class="steps">
    {#each STEPS as label, i}
      <button
        class="step"
        class:active={sort.step === i}
        class:done={i < sort.step}
        disabled={i > maxStep || (locked && i !== sort.step)}
        onclick={() => go(i)}
      >
        <span class="num">{i < sort.step ? "✓" : i + 1}</span>{label}
      </button>
      {#if i < STEPS.length - 1}<span class="sep"></span>{/if}
    {/each}
  </nav>

  {#if sort.step === 0}
    <Setup {active} />
  {:else if sort.step === 1}
    <Overview />
  {:else if sort.step === 2}
    <Review />
  {:else if sort.step === 3}
    <Naming />
  {:else if sort.step === 4}
    <PlanView />
  {:else}
    {#key sort.runKey}<RunView />{/key}
  {/if}
</div>

<style>
  .steps {
    display: flex; align-items: center; justify-content: center; gap: 4px; flex-wrap: wrap;
    padding: 10px 16px; border-bottom: 1px solid var(--border); background: var(--surface);
  }
  .step { display: flex; align-items: center; gap: 8px; border: none; background: transparent; color: var(--muted); padding: 6px 10px; }
  .step.active { color: var(--text); font-weight: 600; background: var(--surface-2); }
  .step.done { color: var(--green); }
  .num {
    display: inline-grid; place-items: center; width: 22px; height: 22px; border-radius: 50%;
    background: var(--surface-2); font-size: 12px; font-weight: 600;
  }
  .step.active .num { background: var(--accent); color: var(--accent-text); }
  .step.done .num { background: var(--green-soft); color: var(--green); }
  .sep { width: 22px; height: 1px; background: var(--border); }
</style>
