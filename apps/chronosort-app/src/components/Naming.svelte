<script>
  import { onMount } from "svelte";
  import { checkNaming } from "../lib/api.js";
  import { sort } from "../lib/state.svelte.js";

  const OPTIONS = [
    { id: "keep", title: "Keep the original names", text: "Recommended. If two files share a name, the second gets _1 at the end.", value: "keep" },
    { id: "datetime", title: "Date and time", text: "Named after when the photo was taken, to the second.", value: "datetime" },
    { id: "date_orig", title: "Date + original name", text: "Easy to sort by name and still recognisable.", value: "{year}-{month}-{day}_{original}" },
    { id: "month_orig", title: "Year-month + original name", text: "Groups a month together.", value: "{year}-{month}_{original}" },
    { id: "custom", title: "My own pattern", text: "Build a name from the pieces below.", value: null },
  ];
  const TOKENS = [
    ["{original}", "original name"], ["{year}", "year"], ["{month}", "month"], ["{day}", "day"],
    ["{hour}", "hour"], ["{minute}", "minute"], ["{second}", "second"],
  ];

  let examples = $state({}); // option id -> [[from, to], ...]
  let custom = $state({ ok: true, error: null, examples: [] });
  let input = $state(null);

  const valueOf = (id) => (id === "custom" ? sort.template : OPTIONS.find((o) => o.id === id).value);

  onMount(async () => {
    const out = {};
    for (const o of OPTIONS.filter((o) => o.value)) {
      try { out[o.id] = (await checkNaming(o.value)).examples.slice(0, 3); } catch { out[o.id] = []; }
    }
    examples = out;
  });

  // Live check of the user's own pattern (debounced).
  let timer;
  $effect(() => {
    const t = sort.template;
    clearTimeout(timer);
    timer = setTimeout(async () => {
      try { custom = await checkNaming(t); } catch (e) { custom = { ok: false, error: String(e), examples: [] }; }
    }, 200);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    sort.settings.naming = valueOf(sort.namingChoice);
  });

  let canContinue = $derived(sort.namingChoice !== "custom" || custom.ok);

  function insert(tok) {
    const el = input;
    const [a, b] = el ? [el.selectionStart ?? sort.template.length, el.selectionEnd ?? sort.template.length] : [sort.template.length, sort.template.length];
    sort.template = sort.template.slice(0, a) + tok + sort.template.slice(b);
    requestAnimationFrame(() => { el?.focus(); el?.setSelectionRange(a + tok.length, a + tok.length); });
  }

  function next() {
    sort.plan = null;
    sort.step = 4;
  }
</script>

<section class="wrap">
  <div class="head">
    <h1>How should the files be named?</h1>
    <p class="muted">The examples use your own files. Files whose date is only known to the year keep their original name.</p>
  </div>

  <div class="opts">
    {#each OPTIONS as o}
      <label class="card opt" class:on={sort.namingChoice === o.id}>
        <input type="radio" bind:group={sort.namingChoice} value={o.id} />
        <div class="otext">
          <strong>{o.title}</strong>
          <span class="muted">{o.text}</span>
        </div>
        <div class="ex mono">
          {#if o.id === "custom"}
            {#if sort.namingChoice !== "custom"}<span class="muted">e.g. {sort.template}</span>{/if}
          {:else}
            {#each examples[o.id] ?? [] as [from, to]}
              <div><span class="muted">{from}</span> → {to}</div>
            {/each}
          {/if}
        </div>
      </label>
    {/each}
  </div>

  {#if sort.namingChoice === "custom"}
    <div class="card custom">
      <label for="tpl"><strong>Your pattern</strong></label>
      <input id="tpl" class="mono" type="text" bind:this={input} bind:value={sort.template} class:bad={!custom.ok} spellcheck="false" />
      <div class="tokens">
        <span class="muted">Add:</span>
        {#each TOKENS as [tok, label]}
          <button class="small" onclick={() => insert(tok)} title="Insert {tok}">{label}</button>
        {/each}
      </div>
      {#if !custom.ok}
        <p class="problem">✕ {custom.error}</p>
      {:else}
        <p class="good">✓ This pattern is fine.</p>
        <div class="ex mono">
          {#each custom.examples as [from, to]}<div><span class="muted">{from}</span> → {to}</div>{/each}
        </div>
      {/if}
      <p class="hint">
        Rules: must contain the original name or the full date and time (down to the second), at most 80 characters,
        and none of <span class="mono">/ \ : * ? " &lt; &gt; |</span>. The file extension is added automatically.
      </p>
    </div>
  {/if}

  <div class="row foot">
    <span class="spacer"></span>
    <button onclick={() => (sort.step = sort.analysis.review.length ? 2 : 1)}>← Back</button>
    <button class="primary big" data-continue disabled={!canContinue} onclick={next}>Continue to final check →</button>
  </div>
</section>

<style>
  .wrap { max-width: 920px; margin: 0 auto; padding: 28px 24px 48px; display: grid; gap: 16px; }
  .head { display: grid; gap: 4px; }
  .opts { display: grid; gap: 10px; }
  .opt {
    display: grid; grid-template-columns: auto 280px 1fr; gap: 14px; align-items: center;
    padding: 14px 16px; cursor: pointer; border-width: 1.5px; box-shadow: none;
  }
  .opt.on { border-color: var(--accent); background: var(--accent-soft); }
  .opt input { accent-color: var(--accent); width: 18px; height: 18px; }
  .otext { display: grid; gap: 2px; }
  .otext strong { font-size: 15px; }
  .otext .muted { font-size: 13px; }
  .ex { display: grid; gap: 2px; font-size: 12.5px; min-width: 0; }
  .ex div { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .custom { padding: 16px 18px; display: grid; gap: 10px; }
  .custom input { font-size: 15px; padding: 9px 12px; }
  .custom input.bad { border-color: var(--red); }
  .tokens { display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
  .problem { color: var(--red); background: var(--red-soft); padding: 8px 12px; border-radius: var(--radius-sm); }
  .good { color: var(--green); font-weight: 600; }
  .hint { font-size: 12.5px; color: var(--muted); }
  .big { font-size: 15px; padding: 10px 20px; }
</style>
