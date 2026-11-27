<script>
  // Sits exactly over the shown picture while retouching: brushes for heal and
  // clone, boxes for red eyes, and the drawing layer (kept at the picture's own
  // resolution, shown scaled). Screen points are turned into picture pixels.
  // rt: shared state with RetouchPanel; rect: the picture on screen {x,y,w,h};
  // imgW / imgH: picture pixels; onapply(op): send a retouch step to the backend.
  let { rt, rect, imgW, imgH, onapply } = $props();

  let layerEl = $state(); // full-resolution drawing layer
  let tempEl = $state(); // full-resolution preview of the shape being dragged
  let dabs = $state([]); // current brush stroke, picture pixels
  let box = $state(null); // red-eye box being dragged, screen
  let pointer = $state(null); // brush outline, screen

  const k = $derived(imgW / rect.w); // picture pixels per screen pixel
  const toImg = (e) => ({ x: (e.clientX - rect.x) * k, y: (e.clientY - rect.y) * k });
  const toScr = (p) => ({ x: p.x / k, y: p.y / k });
  const brushR = $derived(rt.size * k); // brush radius in picture pixels

  // The drawing layer lives in rt so the panel can apply or clear it.
  $effect(() => {
    if (layerEl) rt.layer = layerEl;
  });

  function ctxOf(c) {
    const x = c.getContext("2d");
    x.lineCap = "round";
    x.lineJoin = "round";
    x.strokeStyle = x.fillStyle = rt.color;
    x.lineWidth = rt.width * k;
    return x;
  }
  function shape(x, kind, a, b) {
    if (kind === "line" || kind === "arrow") {
      x.beginPath(); x.moveTo(a.x, a.y); x.lineTo(b.x, b.y); x.stroke();
      if (kind === "arrow") {
        const ang = Math.atan2(b.y - a.y, b.x - a.x), len = Math.max(12 * k, x.lineWidth * 4);
        x.beginPath();
        x.moveTo(b.x, b.y);
        x.lineTo(b.x - len * Math.cos(ang - 0.45), b.y - len * Math.sin(ang - 0.45));
        x.lineTo(b.x - len * Math.cos(ang + 0.45), b.y - len * Math.sin(ang + 0.45));
        x.closePath(); x.fill();
      }
    } else if (kind === "rect") {
      x.strokeRect(Math.min(a.x, b.x), Math.min(a.y, b.y), Math.abs(b.x - a.x), Math.abs(b.y - a.y));
    } else if (kind === "ellipse") {
      x.beginPath();
      x.ellipse((a.x + b.x) / 2, (a.y + b.y) / 2, Math.abs(b.x - a.x) / 2 || 1, Math.abs(b.y - a.y) / 2 || 1, 0, 0, Math.PI * 2);
      x.stroke();
    }
  }

  function down(e) {
    if (rt.busy || e.button !== 0) return;
    e.preventDefault();
    const p = toImg(e);
    const t = rt.tool;
    if (t === "clone" && (e.altKey || rt.pickSource || !rt.src)) {
      rt.src = p;
      rt.offset = null;
      rt.pickSource = false;
      return;
    }
    if (t === "draw" && rt.shape === "text") {
      if (!rt.text.trim()) { rt.hint = "Type the text in the panel first, then click where it should go."; return; }
      const x = ctxOf(layerEl);
      x.font = `${rt.font * k}px "Segoe UI", system-ui, sans-serif`;
      x.textBaseline = "top";
      rt.text.split("\n").forEach((line, n) => x.fillText(line, p.x, p.y + n * rt.font * k * 1.2));
      rt.drew = true;
      return;
    }
    const start = p;
    let last = p;
    if (t === "heal" || t === "clone") dabs = [{ x: p.x, y: p.y, r: brushR }];
    if (t === "redeye") box = { x1: e.clientX, y1: e.clientY, x2: e.clientX, y2: e.clientY };
    if (t === "draw" && rt.shape === "pen") {
      const x = ctxOf(layerEl);
      x.beginPath(); x.moveTo(p.x, p.y); x.lineTo(p.x + 0.01, p.y); x.stroke();
      rt.drew = true;
    }
    const move = (ev) => {
      const q = toImg(ev);
      if (t === "heal" || t === "clone") {
        if (Math.hypot(q.x - last.x, q.y - last.y) > brushR / 3) {
          dabs = [...dabs, { x: q.x, y: q.y, r: brushR }];
          last = q;
        }
      } else if (t === "redeye") {
        box = { ...box, x2: ev.clientX, y2: ev.clientY };
      } else if (t === "draw" && rt.shape === "pen") {
        const x = ctxOf(layerEl);
        x.beginPath(); x.moveTo(last.x, last.y); x.lineTo(q.x, q.y); x.stroke();
        last = q;
      } else if (t === "draw") {
        const x = ctxOf(tempEl);
        x.clearRect(0, 0, imgW, imgH);
        shape(x, rt.shape, start, q);
        last = q;
      }
    };
    const up = async () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      if (t === "draw" && rt.shape !== "pen") {
        tempEl.getContext("2d").clearRect(0, 0, imgW, imgH);
        if (Math.hypot(last.x - start.x, last.y - start.y) > 2) {
          shape(ctxOf(layerEl), rt.shape, start, last);
          rt.drew = true;
        }
      } else if (t === "redeye") {
        const b = box;
        box = null;
        const a = toImg({ clientX: Math.min(b.x1, b.x2), clientY: Math.min(b.y1, b.y2) });
        const c = toImg({ clientX: Math.max(b.x1, b.x2), clientY: Math.max(b.y1, b.y2) });
        if (c.x - a.x > 3 && c.y - a.y > 3) {
          await onapply({ kind: "red_eye", eyes: [{ cx: (a.x + c.x) / 2, cy: (a.y + c.y) / 2, rx: (c.x - a.x) / 2, ry: (c.y - a.y) / 2, pupilSize: rt.pupil, darken: rt.darken }] });
        }
      } else if (t === "heal") {
        const d = dabs;
        await onapply({ kind: "heal", dabs: d });
        dabs = [];
      } else if (t === "clone") {
        const d = dabs;
        // Aligned: the offset is fixed by the first stroke after the source was set.
        if (!rt.offset) rt.offset = { dx: rt.src.x - d[0].x, dy: rt.src.y - d[0].y };
        await onapply({ kind: "clone", dabs: d, dx: rt.offset.dx, dy: rt.offset.dy, hardness: rt.hardness });
        dabs = [];
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }
</script>

<div class="rtlayer" class:busy={rt.busy} style="left:{rect.x}px; top:{rect.y}px; width:{rect.w}px; height:{rect.h}px"
  role="presentation" onpointerdown={down}
  onpointermove={(e) => (pointer = { x: e.clientX - rect.x, y: e.clientY - rect.y })} onpointerleave={() => (pointer = null)}>
  <canvas bind:this={layerEl} width={imgW} height={imgH}></canvas>
  <canvas bind:this={tempEl} width={imgW} height={imgH}></canvas>
  <svg>
    {#each dabs as d}<circle cx={d.x / k} cy={d.y / k} r={d.r / k} class="dab" />{/each}
    {#if rt.tool === "clone" && rt.src}
      {@const s = toScr(rt.src)}
      <g class="src"><circle cx={s.x} cy={s.y} r={rt.size} /><path d="M{s.x - 7} {s.y}h14M{s.x} {s.y - 7}v14" /></g>
    {/if}
    {#if pointer && (rt.tool === "heal" || rt.tool === "clone")}
      <circle cx={pointer.x} cy={pointer.y} r={rt.size} class="ring" />
    {/if}
    {#if box}
      <ellipse cx={(box.x1 + box.x2) / 2 - rect.x} cy={(box.y1 + box.y2) / 2 - rect.y} rx={Math.abs(box.x2 - box.x1) / 2} ry={Math.abs(box.y2 - box.y1) / 2} class="eye" />
    {/if}
  </svg>
</div>

<style>
  .rtlayer { position: fixed; z-index: 2; cursor: crosshair; touch-action: none; }
  .rtlayer.busy { cursor: progress; }
  canvas, svg { position: absolute; inset: 0; width: 100%; height: 100%; pointer-events: none; }
  .dab { fill: rgba(255, 80, 80, 0.35); }
  .ring { fill: none; stroke: #fff; stroke-width: 1.5; filter: drop-shadow(0 0 1px #000); }
  .src circle { fill: none; stroke: #6fd3ff; stroke-width: 1.5; stroke-dasharray: 4 3; }
  .src path { stroke: #6fd3ff; stroke-width: 1.5; }
  .eye { fill: none; stroke: #ffc46b; stroke-width: 2; }
</style>
