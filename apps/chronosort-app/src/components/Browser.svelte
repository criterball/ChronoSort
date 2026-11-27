<script>
  import { onMount, tick } from "svelte";
  import Icon from "./Icon.svelte";
  import TreeNode from "./TreeNode.svelte";
  import ContextMenu from "./ContextMenu.svelte";
  import Viewer from "./Viewer.svelte";
  import Slideshow from "./Slideshow.svelte";
  import BatchDialog from "./BatchDialog.svelte";
  import CreateDialog from "./CreateDialog.svelte";
  import Compare from "./Compare.svelte";
  import {
    browseRoots, listDir, folderSamples, revealItem, fthumbUrl, formatBytes, plural, confirmBox, openFolder, pickFolder,
    thumbCachePrescan, thumbCacheCancel, stripMeta,
    binStatus, checkName, newFolder, renameItem, transferItems, deleteItems, cancelFileOp, lastUndo, undoFileOp, onProgress,
    readMarks, setRating, setTagged, starText,
  } from "../lib/api.js";
  import {
    app, browser, bprefs, saveBrowserPrefs, sort, dups, people, places,
    resetSort, resetDups, resetPeople, resetPlaces,
  } from "../lib/state.svelte.js";
  import { samePath, isInside, joinIn, nameOf, sortEntries } from "../lib/paths.js";

  let { go, active = true } = $props();

  let roots = $state({ volumes: [], known: [], sep: "\\" });
  let sep = $derived(roots.sep);
  let notice = $state("");
  let menu = $state(null); // { x, y, items }
  let viewer = $state(null); // { ids: [...], index }
  let compare = $state(null); // 2–4 entries shown side by side
  let slideshow = $state(null); // { items, start }
  let batch = $state(null); // { items, tab }
  let create = $state(null); // { items, tab }
  let marks = $state({}); // file id -> { rating (0–5, null = not a photo), tagged }
  let rateFilter = $state("any"); // any | "1".."5" (at least) | none | tagged  (this session only)
  let failedThumbs = $state(new Set());
  let samples = $state({}); // folder id -> [ids]
  let area = $state(); // the file area element
  let addr = $state(""); // address bar text
  let treeRefresh = $state(0);

  const KNOWN_LABEL = { pictures: "Pictures", desktop: "Desktop", downloads: "Downloads", documents: "Documents", videos: "Videos", home: "Home" };
  const SIZE_PX = { s: 96, m: 150, l: 240 };
  const MEDIA = new Set(["image", "raw", "video"]);

  // ---------- housekeeping state ----------
  let bin = $state({ available: true, reason: null }); // Recycle Bin of the current drive
  let clip = $state(null); // { ids, mode: "copy" | "move", from, names }
  let job = $state(null); // running copy/move/delete: { title, prog }
  let undoLabel = $state(null);
  let ask = $state(null); // { kind: "name" | "delete", … } modal
  let askInput = $state();
  let dragOver = $state(null); // folder id (or "here") under a drag
  let viewerChanged = false; // the viewer saved something: refresh when it closes

  onMount(() => {
    const un = onProgress("fileop-progress", (p) => job && (job.prog = p));
    return () => un.then((f) => f());
  });

  onMount(async () => {
    roots = await browseRoots();
    const start = bprefs.last || roots.known.find((k) => k.id === "pictures")?.path || "";
    await navigate(start, { push: false, quiet: true });
  });

  function say(msg) {
    notice = msg;
    clearTimeout(say.t);
    say.t = setTimeout(() => (notice = ""), 5000);
  }

  function volLabel(v) {
    const root = v.path.replace(/[\\/]$/, "") || "/";
    const kind = { removable: "USB / card", network: "Network", optical: "CD/DVD", fixed: "", ram: "RAM disk", other: "" }[v.kind];
    return v.label ? `${v.label} (${root})` : kind ? `${kind} (${root})` : root === "/" ? "Computer (/)" : `Local disk (${root})`;
  }
  const volIcon = (v) => ({ removable: "usb", network: "network", optical: "disc" })[v.kind] ?? "drive";

  /** Open a folder ("" = This PC). */
  export async function navigate(path, { push = true, quiet = false } = {}) {
    const from = browser.path;
    browser.loading = true;
    browser.error = "";
    try {
      const listing = path ? await listDir(path, bprefs.showHidden) : null;
      if (push && !samePath(from, listing?.path ?? "", sep)) {
        browser.back = [...browser.back.slice(-99), from];
        browser.forward = [];
      }
      browser.listing = listing;
      browser.path = listing?.path ?? "";
      browser.selected = [];
      browser.anchor = null;
      browser.filter = "";
      addr = browser.path;
      bprefs.last = browser.path;
      saveBrowserPrefs();
      bin = browser.path ? await binStatus(browser.path).catch(() => ({ available: false, reason: "Could not check the Recycle Bin." })) : bin;
      undoLabel = await lastUndo().catch(() => null);
      await tick();
      area?.scrollTo(0, 0);
    } catch (e) {
      if (quiet) {
        browser.path = "";
        browser.listing = null;
        addr = "";
      } else {
        browser.error = String(e);
        addr = browser.path;
      }
    } finally {
      browser.loading = false;
    }
  }

  async function goBack() {
    if (!browser.back.length) return;
    const p = browser.back.at(-1);
    browser.back = browser.back.slice(0, -1);
    browser.forward = [browser.path, ...browser.forward];
    await navigate(p, { push: false });
  }
  async function goForward() {
    if (!browser.forward.length) return;
    const [p, ...rest] = browser.forward;
    browser.forward = rest;
    browser.back = [...browser.back, browser.path];
    await navigate(p, { push: false });
  }
  const goUp = () => browser.path && navigate(browser.listing?.parent ?? "");
  const refresh = () => {
    treeRefresh++;
    return navigate(browser.path, { push: false });
  };

  // ---------- what is shown ----------
  let entries = $derived.by(() => {
    const l = browser.listing?.entries ?? [];
    const f = browser.filter.trim().toLowerCase();
    const shown = l.filter((e) => (bprefs.show === "all" || e.dir || MEDIA.has(e.kind)) && (!f || e.name.toLowerCase().includes(f)) && passRating(e));
    return sortEntries(shown, bprefs.sortBy, bprefs.sortDesc, (e) => marks[e.id]?.rating ?? -1);
  });
  function passRating(e) {
    if (rateFilter === "any" || e.dir) return true;
    const m = marks[e.id];
    if (rateFilter === "tagged") return !!m?.tagged;
    if (rateFilter === "none") return m?.rating === 0;
    return (m?.rating ?? 0) >= +rateFilter;
  }

  // ---------- stars and tags ----------
  // Read for every listed file after the folder opens (in parts, so big folders fill in as they go).
  $effect(() => {
    const l = browser.listing;
    if (!l) { marks = {}; return; }
    loadMarks(l.entries.filter((e) => !e.dir).map((e) => e.id), l);
  });
  async function loadMarks(ids, l) {
    const out = {};
    for (let i = 0; i < ids.length; i += 300) {
      const part = await readMarks(ids.slice(i, i + 300)).catch(() => []);
      if (browser.listing !== l) return; // another folder by now
      for (const m of part) out[m.id] = m;
      marks = { ...out };
    }
    if (!ids.length) marks = {};
  }
  const ratable = () => selEntries.filter((e) => marks[e.id]?.rating != null);
  async function rate(stars) {
    const list = ratable();
    const skipped = selEntries.filter((e) => !e.dir).length - list.length;
    const skipNote = skipped ? ` · ${plural(skipped, "file was", "files were")} left out (stars are for photos only)` : "";
    if (!list.length) return say("Select one or more photos to give them stars (keys 1–5, 0 removes the stars). Videos and other files cannot have stars.");
    const ids = list.map((e) => e.id);
    if (ids.length > 3) job = { title: stars ? `Giving ${starText(stars)} to ${ids.length} photos…` : `Removing the stars from ${ids.length} photos…`, prog: null, cancel: false };
    try {
      const r = await setRating(ids, stars);
      const msg = [];
      if (r.done) msg.push(stars ? `${starText(stars)} saved in ${plural(r.done, "photo", "photos")}` : `Stars removed from ${plural(r.done, "photo", "photos")}`);
      if (r.unchanged) msg.push(`${plural(r.unchanged, "photo already had", "photos already had")} ${stars ? "these stars" : "no stars"}`);
      if (r.failed.length) msg.push(`not possible for ${r.failed.map(([n, why]) => `${n} (${why})`).join(", ")}`);
      say(msg.join(" · ") + skipNote + (r.done ? " (Ctrl+Z undoes)" : ""));
    } catch (err) {
      say(String(err));
    } finally {
      job = null;
    }
    await quietRefresh();
  }
  async function toggleTag() {
    const ids = selEntries.filter((e) => !e.dir).map((e) => e.id);
    if (!ids.length) return say("Select files to tag them (Q).");
    const on = !ids.every((id) => marks[id]?.tagged);
    try {
      await setTagged(ids, on);
      const m = { ...marks };
      for (const id of ids) m[id] = { ...(m[id] ?? { id, rating: null }), tagged: on };
      marks = m;
      say(on ? `${plural(ids.length, "file", "files")} tagged. To see only tagged files, choose “Tagged” in the stars list at the top.` : `Tag removed from ${plural(ids.length, "file", "files")}.`);
    } catch (err) {
      say(String(err));
    }
  }
  // Re-read the folder (new file sizes) but keep the selection and the scroll position.
  async function quietRefresh() {
    const top = area?.scrollTop ?? 0;
    const names = new Set(selEntries.map((e) => e.name.toLowerCase()));
    const anchor = entries.find((e) => e.id === browser.anchor)?.name.toLowerCase();
    await navigate(browser.path, { push: false });
    browser.selected = entries.filter((e) => names.has(e.name.toLowerCase())).map((e) => e.id);
    browser.anchor = entries.find((e) => e.name.toLowerCase() === anchor)?.id ?? browser.selected[0] ?? null;
    await tick();
    if (area) area.scrollTop = top;
    undoLabel = await lastUndo().catch(() => null);
    area?.focus();
  }
  let selSet = $derived(new Set(browser.selected));
  let selEntries = $derived(entries.filter((e) => selSet.has(e.id)));
  let one = $derived(selEntries.length === 1 ? selEntries[0] : null);
  let counts = $derived.by(() => {
    let folders = 0, files = 0, bytes = 0, hiddenByFilter = 0;
    for (const e of browser.listing?.entries ?? []) {
      if (e.dir) folders++;
      else { files++; bytes += e.size; }
    }
    hiddenByFilter = (browser.listing?.entries?.length ?? 0) - entries.length;
    return { folders, files, bytes, hiddenByFilter };
  });
  let selBytes = $derived(selEntries.reduce((n, e) => n + e.size, 0));

  const pathOf = (e) => joinIn(browser.path, e.name, sep);
  // The tree root that unfolds towards the current folder: the deepest one containing it.
  let followRoot = $derived(
    [...(bprefs.root ? [bprefs.root] : []), ...bprefs.favs.map((f) => f.path), ...roots.known.map((k) => k.path), ...roots.volumes.map((v) => v.path)]
      .filter((p) => isInside(browser.path, p, sep))
      .sort((a, b) => b.length - a.length)[0] ?? "",
  );
  const thumbable = (e) => MEDIA.has(e.kind) && !failedThumbs.has(e.id);
  function thumbFailed(id) {
    failedThumbs = new Set(failedThumbs).add(id);
  }

  function setPref(k, v) {
    bprefs[k] = v;
    saveBrowserPrefs();
  }
  function sortByCol(k) {
    if (bprefs.sortBy === k) setPref("sortDesc", !bprefs.sortDesc);
    else { bprefs.sortBy = k; setPref("sortDesc", k === "date" || k === "size"); }
  }

  // ---------- selection ----------
  function select(e, ev) {
    const ids = entries.map((x) => x.id);
    if (ev?.shiftKey && browser.anchor != null) {
      const a = ids.indexOf(browser.anchor), b = ids.indexOf(e.id);
      const [lo, hi] = a < b ? [a, b] : [b, a];
      const range = ids.slice(lo, hi + 1);
      browser.selected = ev.ctrlKey ? [...new Set([...browser.selected, ...range])] : range;
    } else if (ev?.ctrlKey || ev?.metaKey) {
      browser.selected = selSet.has(e.id) ? browser.selected.filter((x) => x !== e.id) : [...browser.selected, e.id];
      browser.anchor = e.id;
    } else {
      browser.selected = [e.id];
      browser.anchor = e.id;
    }
  }

  function openEntry(e) {
    if (e.dir) return navigate(pathOf(e));
    if (MEDIA.has(e.kind)) {
      const media = entries.filter((x) => MEDIA.has(x.kind));
      viewer = { items: media, index: Math.max(0, media.findIndex((x) => x.id === e.id)), folder: browser.path };
      return;
    }
    revealItem(e.id).catch((err) => say(String(err)));
  }

  // Compare side by side: 2–4 selected photos (or videos: their first frame).
  const compareable = () => selEntries.filter((x) => MEDIA.has(x.kind));
  function startCompare() {
    const m = compareable();
    if (m.length < 2 || m.length > 4) return say("Select 2 to 4 photos to compare them side by side (Ctrl+click to add one).");
    compare = m.map((x) => ({ id: x.id, name: x.name, size: x.size, modified: x.modified }));
  }

  // Slideshow: the selected photos when 2 or more are selected, otherwise every
  // photo and video in this folder (as sorted and filtered), from the selected one.
  function startSlideshow() {
    const sel = selEntries.filter((x) => MEDIA.has(x.kind));
    const all = entries.filter((x) => MEDIA.has(x.kind));
    const list = sel.length >= 2 ? sel : all;
    if (!list.length) return say("There are no photos or videos in this folder for a slideshow.");
    const from = sel.length === 1 ? list.findIndex((x) => x.id === sel[0].id) : 0;
    slideshow = { items: list.map((x) => ({ id: x.id, name: x.name, kind: x.kind, modified: x.modified })), start: Math.max(0, from) };
  }

  // Batch tools: the selected files, or every file in this folder when none is selected.
  function startBatch(tab) {
    if (!browser.path) return;
    const sel = selEntries.filter((x) => !x.dir);
    const list = sel.length ? sel : entries.filter((x) => !x.dir);
    if (!list.length) return say("There are no files here for the batch tools.");
    batch = { items: list.map((x) => ({ id: x.id, name: x.name, kind: x.kind, dir: false })), tab };
  }

  // Create (contact sheet, joined picture, PDF, ZIP): the selected files, or the whole folder.
  function startCreate(tab) {
    if (!browser.path) return;
    const sel = selEntries.filter((x) => !x.dir);
    const list = sel.length ? sel : entries.filter((x) => !x.dir);
    if (!list.length) return say("There are no files here.");
    create = { items: list.map((x) => ({ id: x.id, name: x.name, kind: x.kind, modified: x.modified, dir: false })), tab };
  }

  // Make the thumbnails of a folder (and its sub-folders) now, so browsing it later is instant.
  async function prescanFolder(p) {
    job = { title: `Making thumbnails for “${nameOf(p) || p}”…`, prog: null, cancel: () => thumbCacheCancel(), stop: "Stop" };
    app.busy = true;
    const un = await onProgress("cache-progress", (x) => { if (job) job.prog = { files_done: x.done, files_total: x.total, current: x.name }; });
    try {
      const sizes = bprefs.size === "l" ? [320, 640] : [320];
      const r = await thumbCachePrescan(p, true, sizes);
      say(r.photos ? `${r.cancelled ? "Stopped. " : ""}Thumbnails ready for ${plural(r.made, "photo or video", "photos and videos")}${r.failed ? ` (${r.failed} could not be read)` : ""}.` : "There are no photos or videos in that folder.");
    } catch (e) {
      say(String(e));
    } finally {
      un();
      app.busy = false;
      job = null;
    }
  }

  // Remove information before sharing: "place" = GPS and place names; "all" = everything
  // except the turn and the colour profile. Backups + one Undo, like star ratings.
  async function removeInfo(what) {
    const ids = selEntries.filter((e) => MEDIA.has(e.kind)).map((e) => e.id);
    if (!ids.length) return say("Select one or more photos first.");
    const n = plural(ids.length, "photo", "photos");
    const ok = await confirmBox(
      what === "place"
        ? `Remove the place (GPS position, town, region, country) from ${n}?\n\nEverything else stays. A backup is kept, and Ctrl+Z puts it back.`
        : `Remove all information from ${n}?\n\nThis takes out the camera, dates, place, names, keywords and comments — handy before sharing. The pictures themselves do not change. A backup is kept, and Ctrl+Z puts it all back.\n\nNote: without the date taken, ChronoSort can no longer sort these copies by year.`,
      "Remove photo information", "Remove", "Keep it",
    );
    if (!ok) return;
    job = { title: what === "place" ? `Removing the place from ${n}…` : `Removing the information from ${n}…`, prog: null, cancel: false };
    try {
      const r = await stripMeta(ids, what);
      const msg = [];
      if (r.done) msg.push(`${plural(r.done, "photo", "photos")} done`);
      if (r.unchanged) msg.push(`${plural(r.unchanged, "photo had", "photos had")} nothing to remove`);
      if (r.failed.length) msg.push(`not possible for ${r.failed.map(([f, w]) => `${f} (${w})`).join(", ")}`);
      say(msg.join(" · ") + (r.done ? " (Ctrl+Z undoes)" : ""));
    } catch (e) {
      say(String(e));
    } finally {
      job = null;
    }
    await quietRefresh();
  }

  // ---------- favourite folders and "show only this folder" ----------
  const isFav = (p) => bprefs.favs.some((f) => samePath(f.path, p, sep));
  function toggleFav(p) {
    if (!p) return;
    if (isFav(p)) {
      bprefs.favs = bprefs.favs.filter((f) => !samePath(f.path, p, sep));
      say(`“${nameOf(p) || p}” is no longer a favourite.`);
    } else {
      if (bprefs.favs.length >= 40) return say("You have 40 favourites already; remove one first.");
      bprefs.favs = [...bprefs.favs, { path: p, label: nameOf(p) || p }];
      say(`“${nameOf(p) || p}” added to your favourites (at the top of the folder list).`);
    }
    saveBrowserPrefs();
  }
  function moveFav(p, d) {
    const i = bprefs.favs.findIndex((f) => samePath(f.path, p, sep));
    const j = i + d;
    if (i < 0 || j < 0 || j >= bprefs.favs.length) return;
    const a = [...bprefs.favs];
    [a[i], a[j]] = [a[j], a[i]];
    bprefs.favs = a;
    saveBrowserPrefs();
  }
  function setRoot(p) {
    bprefs.root = p || "";
    saveBrowserPrefs();
    if (p && !isInside(browser.path, p, sep)) navigate(p);
  }
  const favItems = (p) => {
    if (!p) return [];
    const fav = isFav(p);
    const i = bprefs.favs.findIndex((f) => samePath(f.path, p, sep));
    return [
      { label: fav ? "Remove from favourites" : "Add to favourites", icon: fav ? "star-off" : "star", key: samePath(p, browser.path, sep) ? "Ctrl+D" : undefined, action: () => toggleFav(p) },
      ...(fav && bprefs.favs.length > 1
        ? [{ label: "Move up in favourites", disabled: i === 0, action: () => moveFav(p, -1) }, { label: "Move down in favourites", disabled: i === bprefs.favs.length - 1, action: () => moveFav(p, 1) }]
        : []),
      { label: "Make thumbnails now", icon: "thumbs", action: () => prescanFolder(p) },
      samePath(bprefs.root, p, sep)
        ? { label: "Show all folders in the tree", icon: "pc", action: () => setRoot("") }
        : { label: "Show only this folder in the tree", icon: "tree-left", action: () => setRoot(p) },
    ];
  };

  function columns() {
    if (bprefs.view === "details" || bprefs.view === "list") return 1;
    const first = area?.querySelector(".cell");
    if (!first) return 1;
    return Math.max(1, Math.floor(area.clientWidth / first.getBoundingClientRect().width));
  }

  async function moveSel(delta, ev) {
    if (!entries.length) return;
    const ids = entries.map((x) => x.id);
    const cur = ids.indexOf(browser.anchor ?? browser.selected.at(-1));
    const next = Math.min(ids.length - 1, Math.max(0, cur < 0 ? 0 : cur + delta));
    const e = entries[next];
    if (ev.shiftKey) {
      const keep = browser.anchor;
      select(e, { shiftKey: true });
      browser.anchor = keep;
      // move the "focus" with the range end
      browser.anchor = keep ?? e.id;
    } else select(e);
    await tick();
    area?.querySelector(`[data-id="${e.id}"]`)?.scrollIntoView({ block: "nearest" });
  }

  function onAreaKey(ev) {
    if (menu || viewer || compare || slideshow || batch || create) return;
    const k = ev.key;
    const ctrl = ev.ctrlKey || ev.metaKey;
    if (k === "ArrowRight" && bprefs.view !== "details") moveSel(1, ev);
    else if (k === "ArrowLeft" && bprefs.view !== "details") moveSel(-1, ev);
    else if (k === "ArrowDown") moveSel(columns(), ev);
    else if (k === "ArrowUp") moveSel(-columns(), ev);
    else if (k === "Home") moveSel(-1e9, ev);
    else if (k === "End") moveSel(1e9, ev);
    else if (k === "Enter" && one) openEntry(one);
    else if (k === "Backspace") goUp();
    else if (ctrl && (k === "a" || k === "A")) browser.selected = entries.map((e) => e.id);
    else if ((k === "p" || k === "P") && !ctrl) startCompare();
    else if ((k === "s" || k === "S") && !ctrl) startSlideshow();
    else if (!browser.path || job) return;
    // housekeeping keys
    else if (/^[0-5]$/.test(k) && !ev.altKey && !ev.shiftKey) rate(+k);
    else if ((k === "q" || k === "Q") && !ctrl) toggleTag();
    else if (k === "F2") doRename();
    else if (k === "Delete") doDelete(ev.shiftKey);
    else if (k === "F8" || (ctrl && ev.shiftKey && (k === "N" || k === "n"))) doNewFolder();
    else if (ctrl && (k === "c" || k === "C")) toClip("copy");
    else if (ctrl && (k === "x" || k === "X")) toClip("move");
    else if (ctrl && (k === "v" || k === "V")) paste();
    else return;
    ev.preventDefault();
  }

  function onWindowKey(ev) {
    if (!active || menu || viewer || compare || slideshow || batch || create || document.querySelector("dialog[open]")) return;
    if (ev.key === "F5") { ev.preventDefault(); refresh(); }
    else if (ev.key === "F3") { ev.preventDefault(); startBatch("convert"); }
    else if ((ev.ctrlKey || ev.metaKey) && (ev.key === "d" || ev.key === "D") && browser.path) { ev.preventDefault(); toggleFav(browser.path); }
    else if (ev.key === "F4") { ev.preventDefault(); startBatch("rename"); }
    else if (ev.altKey && ev.key === "ArrowLeft") { ev.preventDefault(); goBack(); }
    else if (ev.altKey && ev.key === "ArrowRight") { ev.preventDefault(); goForward(); }
    else if (ev.altKey && ev.key === "ArrowUp") { ev.preventDefault(); goUp(); }
    else if ((ev.ctrlKey || ev.metaKey) && (ev.key === "z" || ev.key === "Z") && !ev.target.closest?.("input")) { ev.preventDefault(); doUndo(); }
  }

  // ---------- folder tiles: up to 4 sample photos, fetched when the tile scrolls into view ----------
  function lazySamples(node, e) {
    if (!e.dir || samples[e.id]) return;
    const io = new IntersectionObserver(async (seen) => {
      if (!seen.some((s) => s.isIntersecting)) return;
      io.disconnect();
      try {
        const ids = await folderSamples(e.id);
        samples = { ...samples, [e.id]: ids };
      } catch {
        samples = { ...samples, [e.id]: [] };
      }
    }, { root: area, rootMargin: "300px" });
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  // ---------- tool shortcuts (right-click a folder) ----------
  async function sortHere(path) {
    if (app.busy) return say("Please wait until the current job has finished.");
    if (sort.analysis && !samePath(sort.settings.source, path, sep)) {
      const ok = await confirmBox(`You are part-way through sorting “${nameOf(sort.settings.source)}”.\n\nStart again with “${nameOf(path)}” instead?`, "Sort this folder", "Start again", "Keep the old one");
      if (!ok) return go("sort");
    }
    resetSort(true);
    sort.settings.source = path;
    go("sort");
  }
  function dupsHere(path) {
    if (app.busy) return say("Please wait until the current job has finished.");
    resetDups();
    dups.folders = [path];
    go("dups");
  }
  function peopleHere(path) {
    if (app.busy) return say("Please wait until the current job has finished.");
    resetPeople();
    people.folder = path;
    go("people");
  }
  function placesHere(path) {
    if (app.busy) return say("Please wait until the current job has finished.");
    resetPlaces();
    places.folder = path;
    go("places");
  }
  const toolItems = (path) => [
    { label: "Sort this folder into years…", icon: "calendar", bold: true, action: () => sortHere(path) },
    { label: "Find duplicates here", icon: "dup", action: () => dupsHere(path) },
    { label: "Find people here", icon: "people", action: () => peopleHere(path) },
    { label: "Add a place to these photos", icon: "pin", action: () => placesHere(path) },
  ];

  function onItemMenu(ev, e) {
    ev.preventDefault();
    if (!selSet.has(e.id)) select(e);
    const items = [];
    if (e.dir) {
      items.push({ label: "Open", icon: "open", key: "Enter", bold: true, action: () => openEntry(e) });
      items.push("-", ...toolItems(pathOf(e)), "-");
      items.push({ label: "Show in Explorer", icon: "explorer", action: () => revealItem(e.id).catch((x) => say(String(x))) });
      items.push("-", ...favItems(pathOf(e)));
    } else {
      if (MEDIA.has(e.kind)) items.push({ label: "View full screen", icon: "fullscreen", key: "Enter", bold: true, action: () => openEntry(e) });
      if (MEDIA.has(e.kind)) {
        const n = selEntries.filter((x) => MEDIA.has(x.kind)).length;
        items.push({ label: n >= 2 ? `Slideshow of these ${n}` : "Slideshow from here", icon: "slideshow", key: "S", action: startSlideshow });
      }
      const nf = selEntries.filter((x) => !x.dir).length;
      items.push({ label: nf > 1 ? `Convert / resize these ${nf}…` : "Convert / resize…", icon: "resize", key: "F3", action: () => startBatch("convert") });
      items.push({ label: nf > 1 ? `Rename these ${nf}…` : "Batch rename…", icon: "list", key: "F4", action: () => startBatch("rename") });
      items.push({ label: "Contact sheet, PDF or ZIP…", icon: "thumbs", action: () => startCreate(MEDIA.has(e.kind) ? "sheet" : "zip") });
      if (MEDIA.has(e.kind)) {
        items.push({ label: "Remove the place (GPS)…", icon: "pin", action: () => removeInfo("place") });
        items.push({ label: "Remove all photo information…", icon: "close", action: () => removeInfo("all") });
      }
      const m = compareable().length;
      if (m >= 2) items.push({ label: m <= 4 ? `Compare ${m} side by side` : "Compare side by side (2 to 4)", icon: "compare", key: "P", disabled: m > 4, action: startCompare });
      const r = ratable();
      if (r.length) {
        const same = r.every((x) => marks[x.id].rating === marks[r[0].id].rating) ? marks[r[0].id].rating : -1;
        items.push({ stars: same, label: r.length > 1 ? `Stars for ${r.length} photos` : "Stars", action: rate });
      }
      const tagged = selEntries.some((x) => !x.dir) && selEntries.filter((x) => !x.dir).every((x) => marks[x.id]?.tagged);
      items.push({ label: tagged ? "Remove the tag" : "Tag", icon: "tag", key: "Q", action: toggleTag });
      items.push({ label: "Show in Explorer", icon: "explorer", action: () => revealItem(e.id).catch((x) => say(String(x))) });
      items.push("-", ...toolItems(browser.path).map((t) => ({ ...t, label: t.label.replace("this folder", "the whole folder").replace(" here", " in this folder") })));
    }
    items.push("-", ...housekeepingItems(e));
    menu = { x: ev.clientX, y: ev.clientY, items };
  }
  function onBackgroundMenu(ev) {
    if (ev.target.closest("[data-id]")) return;
    ev.preventDefault();
    if (!browser.path) return;
    browser.selected = [];
    menu = {
      x: ev.clientX, y: ev.clientY,
      items: [
        ...toolItems(browser.path),
        { label: "Slideshow of this folder", icon: "slideshow", key: "S", action: startSlideshow },
        { label: "Convert / resize all…", icon: "resize", key: "F3", action: () => startBatch("convert") },
        { label: "Rename all…", icon: "list", key: "F4", action: () => startBatch("rename") },
        { label: "Contact sheet, PDF or ZIP of this folder…", icon: "thumbs", action: () => startCreate("sheet") }, "-",
        { label: clip ? `Paste ${plural(clip.ids.length, "item", "items")} here` : "Paste", key: "Ctrl+V", disabled: !clip, action: () => paste() },
        { label: "New folder", icon: "folder", key: "F8", action: doNewFolder },
        { label: undoLabel ? `Undo: ${undoLabel}` : "Undo", key: "Ctrl+Z", disabled: !undoLabel, action: doUndo },
        "-",
        { label: "Refresh", icon: "refresh", key: "F5", action: refresh },
        ...favItems(browser.path),
        { label: "Select all", key: "Ctrl+A", action: () => (browser.selected = entries.map((e) => e.id)) },
        { label: "Open in Explorer", icon: "explorer", action: () => openFolder(browser.path).catch((x) => say(String(x))) },
      ],
    };
  }
  function onTreeMenu(ev, node) {
    ev.preventDefault();
    menu = {
      x: ev.clientX, y: ev.clientY,
      items: [
        { label: "Open", icon: "open", bold: true, action: () => navigate(node.path) },
        "-", ...toolItems(node.path), "-",
        ...(clip ? [{ label: `Paste ${plural(clip.ids.length, "item", "items")} into “${node.label}”`, action: () => paste(node.path) }, "-"] : []),
        { label: "Open in Explorer", icon: "explorer", action: () => openFolder(node.path).catch((x) => say(String(x))) },
        "-", ...favItems(node.path),
      ],
    };
  }

  // ---------- housekeeping ----------
  /** Re-read the folder and select the given paths (e.g. a new folder or renamed file). */
  async function refreshSelect(paths = []) {
    treeRefresh++;
    await navigate(browser.path, { push: false });
    const want = new Set(paths.map((p) => nameOf(p).toLowerCase()));
    const ids = entries.filter((e) => want.has(e.name.toLowerCase())).map((e) => e.id);
    if (ids.length) {
      browser.selected = ids;
      browser.anchor = ids[0];
      await tick();
      area?.querySelector(`[data-id="${ids[0]}"]`)?.scrollIntoView({ block: "nearest" });
    }
    undoLabel = await lastUndo().catch(() => null);
    area?.focus();
  }

  function freeName(base) {
    const taken = new Set((browser.listing?.entries ?? []).map((e) => e.name.toLowerCase()));
    if (!taken.has(base.toLowerCase())) return base;
    for (let i = 2; ; i++) if (!taken.has(`${base} (${i})`.toLowerCase())) return `${base} (${i})`;
  }

  function askName(title, value, okLabel, onok, selectStem = false) {
    ask = { kind: "name", title, value, okLabel, onok, error: "", selectStem };
  }

  function doNewFolder() {
    if (!browser.path) return;
    askName("New folder", freeName("New folder"), "Create", async (name) => {
      const p = await newFolder(browser.path, name);
      await refreshSelect([p]);
    });
  }

  function doRename(e = one) {
    if (!e) return;
    askName(`Rename ${e.dir ? "folder" : "file"}`, e.name, "Rename", async (name) => {
      const p = await renameItem(e.id, name);
      await refreshSelect([p]);
    }, !e.dir);
  }

  async function submitName() {
    const a = ask;
    try {
      const name = await checkName(a.value);
      await a.onok(name);
      ask = null;
    } catch (err) {
      ask = { ...a, error: String(err) };
    }
  }

  function report(r, what) {
    const parts = [];
    if (r.done) parts.push(`${plural(r.done, "item", "items")} ${what}`);
    if (r.stopped) parts.push("stopped part-way");
    if (r.failed.length) parts.push(`${r.failed.length} could not: ${r.failed.slice(0, 3).map(([n, w]) => `${n} (${w})`).join("; ")}`);
    if (r.warnings.length) parts.push(r.warnings[0]);
    say(parts.join(" · ") || "Nothing was changed.");
  }

  async function runTransfer(ids, dest, mode) {
    if (!ids.length) return;
    job = { title: mode === "move" ? "Moving…" : "Copying…", prog: null, cancel: true };
    app.busy = true;
    try {
      const r = await transferItems(ids, dest, mode);
      report(r, mode === "move" ? "moved" : "copied");
      if (mode === "move" && clip?.mode === "move") clip = null;
      await refreshSelect(samePath(dest, browser.path, sep) ? r.created : []);
    } catch (err) {
      say(String(err));
    } finally {
      job = null;
      app.busy = false;
    }
  }

  const selIds = () => (browser.selected.length ? [...browser.selected] : []);
  function toClip(mode) {
    const ids = selIds();
    if (!ids.length) return;
    clip = { ids, mode, from: browser.path, names: selEntries.map((e) => e.name) };
    say(`${plural(ids.length, "item", "items")} ready to ${mode === "move" ? "move" : "copy"}: open the target folder and press Ctrl+V (or right-click → Paste).`);
  }
  function paste(dest = browser.path) {
    if (!clip || !dest) return;
    runTransfer(clip.ids, dest, clip.mode);
  }
  async function transferTo(mode) {
    const ids = selIds();
    if (!ids.length) return;
    const dest = await pickFolder(mode === "move" ? "Move to which folder?" : "Copy to which folder?");
    if (dest) runTransfer(ids, dest, mode);
  }

  async function doDelete(permanent) {
    const items = selEntries;
    if (!items.length) return;
    const size = formatBytes(items.reduce((n, e) => n + e.size, 0));
    const what = items.length === 1 ? `“${items[0].name}”` : plural(items.length, "item", "items");
    const folders = items.filter((e) => e.dir).length;
    if (!permanent && bin.available) {
      const ok = await confirmBox(`Move ${what} to the Recycle Bin?${folders ? `\n\n${plural(folders, "folder", "folders")} with everything inside.` : ""}\n\nYou can get ${items.length === 1 ? "it" : "them"} back with Undo (Ctrl+Z) or from the Recycle Bin.`, "Delete", "Move to Recycle Bin", "Cancel");
      if (ok) runDelete(items.map((e) => e.id), false);
      return;
    }
    ask = { kind: "delete", what, size, folders, count: items.length, ids: items.map((e) => e.id), noBin: !bin.available ? bin.reason : null };
  }

  async function runDelete(ids, permanent) {
    ask = null;
    job = { title: permanent ? "Deleting permanently…" : "Moving to the Recycle Bin…", prog: null, cancel: false };
    app.busy = true;
    try {
      const r = await deleteItems(ids, permanent);
      report(r, permanent ? "deleted permanently" : "moved to the Recycle Bin");
      await refreshSelect();
    } catch (err) {
      say(String(err));
    } finally {
      job = null;
      app.busy = false;
    }
  }

  async function doUndo() {
    if (!undoLabel || job) return;
    job = { title: "Undoing…", prog: null, cancel: false };
    try {
      say(await undoFileOp());
    } catch (err) {
      say(String(err));
    } finally {
      job = null;
      await refreshSelect();
    }
  }

  const housekeepingItems = (e) => {
    const n = browser.selected.length || 1;
    const items = [
      { label: "Copy", key: "Ctrl+C", action: () => toClip("copy") },
      { label: "Cut", key: "Ctrl+X", action: () => toClip("move") },
    ];
    if (e?.dir && clip) items.push({ label: `Paste into “${e.name}”`, key: "", action: () => paste(pathOf(e)) });
    items.push(
      { label: "Copy to folder…", action: () => transferTo("copy") },
      { label: "Move to folder…", action: () => transferTo("move") },
      "-",
      { label: "Rename", key: "F2", disabled: n !== 1, action: () => doRename() },
    );
    if (bin.available) items.push({ label: "Delete (Recycle Bin)", key: "Del", action: () => doDelete(false) });
    items.push({ label: "Delete permanently…", key: bin.available ? "Shift+Del" : "Del", danger: true, action: () => doDelete(true) });
    return items;
  };

  // ---------- drag and drop (inside ChronoSort): drop on a folder = move, Ctrl = copy ----------
  const DND = "application/x-chronosort-ids";
  function onDragStart(ev, e) {
    if (!selSet.has(e.id)) select(e);
    ev.dataTransfer.setData(DND, JSON.stringify(browser.selected));
    ev.dataTransfer.effectAllowed = "copyMove";
  }
  function dragTarget(ev, key) {
    if (!ev.dataTransfer.types.includes(DND)) return;
    ev.preventDefault();
    ev.dataTransfer.dropEffect = ev.ctrlKey ? "copy" : "move";
    dragOver = key;
  }
  function dropOn(ev, dest) {
    dragOver = null;
    const raw = ev.dataTransfer.getData(DND);
    if (!raw || !dest) return;
    ev.preventDefault();
    const ids = JSON.parse(raw).filter((id) => !samePath(pathOf(entries.find((x) => x.id === id) ?? { name: "" }), dest, sep));
    if (ids.length) runTransfer(ids, dest, ev.ctrlKey ? "copy" : "move");
  }

  // Modal helpers: Esc closes; the name box selects "IMG_0001" but not ".jpg".
  function modal(node) {
    const onKey = (e) => {
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); ask = null; }
    };
    node.addEventListener("keydown", onKey);
    node.querySelector(".primary, input")?.focus();
    return { destroy: () => { node.removeEventListener("keydown", onKey); area?.focus(); } };
  }
  function focusName(node, stemOnly) {
    node.focus();
    const dot = node.value.lastIndexOf(".");
    if (stemOnly && dot > 0) node.setSelectionRange(0, dot);
    else node.select();
  }

  // ---------- splitter ----------
  function dragSplit(ev) {
    const startX = ev.clientX, startW = bprefs.treeW ?? 290;
    const sign = bprefs.treeSide === "left" ? 1 : -1;
    const moveFn = (m) => (bprefs.treeW = Math.min(600, Math.max(180, startW + sign * (m.clientX - startX))));
    const up = () => { window.removeEventListener("pointermove", moveFn); window.removeEventListener("pointerup", up); saveBrowserPrefs(); };
    window.addEventListener("pointermove", moveFn);
    window.addEventListener("pointerup", up);
  }

  const fmtDate = (s) => (s ? new Date(s * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "");
  const typeLabel = (e) => (e.dir ? "Folder" : e.ext ? e.ext.toUpperCase() : "File");
</script>

<svelte:window onkeydown={onWindowKey} />

<div class="browser" class:right={bprefs.treeSide === "right"} style="--tree-w:{bprefs.treeW ?? 290}px; --cell:{SIZE_PX[bprefs.size]}px">
  <!-- toolbar -->
  <div class="tb">
    <div class="grp">
      <button class="ib" title="Back (Alt+Left)" disabled={!browser.back.length} onclick={goBack}><Icon name="back" /></button>
      <button class="ib" title="Forward (Alt+Right)" disabled={!browser.forward.length} onclick={goForward}><Icon name="forward" /></button>
      <button class="ib" title="Up one folder (Backspace)" disabled={!browser.path} onclick={goUp}><Icon name="up" /></button>
      <button class="ib" title="Refresh (F5)" onclick={refresh}><Icon name="refresh" /></button>
      <button class="ib" title={undoLabel ? `Undo: ${undoLabel} (Ctrl+Z)` : "Nothing to undo"} disabled={!undoLabel || !!job} onclick={doUndo}><Icon name="undo" /></button>
    </div>
    <form class="addr" onsubmit={(e) => { e.preventDefault(); navigate(addr.trim()); }}>
      <span class="aic"><Icon name={browser.path ? "folder" : "pc"} size={16} /></span>
      <input bind:value={addr} spellcheck="false" placeholder="This PC — type or paste a folder and press Enter" aria-label="Folder" />
    </form>
    <div class="grp seg" role="group" aria-label="View">
      <button class="ib" class:on={bprefs.view === "thumbs" && bprefs.size === "s"} title="Small thumbnails" onclick={() => { bprefs.view = "thumbs"; setPref("size", "s"); }}><span class="sz">S</span></button>
      <button class="ib" class:on={bprefs.view === "thumbs" && bprefs.size === "m"} title="Medium thumbnails" onclick={() => { bprefs.view = "thumbs"; setPref("size", "m"); }}><span class="sz">M</span></button>
      <button class="ib" class:on={bprefs.view === "thumbs" && bprefs.size === "l"} title="Large thumbnails" onclick={() => { bprefs.view = "thumbs"; setPref("size", "l"); }}><span class="sz">L</span></button>
      <button class="ib" class:on={bprefs.view === "tiles"} title="Tiles" onclick={() => setPref("view", "tiles")}><Icon name="tiles" /></button>
      <button class="ib" class:on={bprefs.view === "list"} title="List" onclick={() => setPref("view", "list")}><Icon name="list" /></button>
      <button class="ib" class:on={bprefs.view === "details"} title="Details" onclick={() => setPref("view", "details")}><Icon name="details" /></button>
    </div>
    <button class="ib" title="Slideshow (S): the selected photos, or the whole folder" onclick={startSlideshow} disabled={!browser.path}><Icon name="slideshow" /></button>
    <select class="sel" value={bprefs.show} onchange={(e) => setPref("show", e.currentTarget.value)} title="Which files to show">
      <option value="all">All files</option>
      <option value="media">Photos & videos</option>
    </select>
    <select class="sel" class:on={rateFilter !== "any"} value={rateFilter} onchange={(e) => (rateFilter = e.currentTarget.value)} title="Show only photos with these stars, or tagged files">
      <option value="any">Any stars</option>
      {#each [1, 2, 3, 4] as n}<option value={String(n)}>{"★".repeat(n)} or more</option>{/each}
      <option value="5">★★★★★ only</option>
      <option value="none">No stars yet</option>
      <option value="tagged">Tagged</option>
    </select>
    <select class="sel" value={bprefs.sortBy} onchange={(e) => { bprefs.sortBy = e.currentTarget.value; saveBrowserPrefs(); }} title="Sort by">
      <option value="name">Name</option>
      <option value="date">Date modified</option>
      <option value="size">Size</option>
      <option value="type">Type</option>
      <option value="rating">Stars</option>
    </select>
    <button class="ib" title={bprefs.sortDesc ? "Descending (click for ascending)" : "Ascending (click for descending)"} onclick={() => setPref("sortDesc", !bprefs.sortDesc)}>
      <Icon name={bprefs.sortDesc ? "sort-desc" : "sort-asc"} />
    </button>
    <label class="search" title="Show only names containing this text">
      <Icon name="search" size={15} />
      <input bind:value={browser.filter} placeholder="Filter names" spellcheck="false" />
    </label>
    <div class="grp">
      <button class="ib" class:on={bprefs.preview} title="Preview pane" onclick={() => setPref("preview", !bprefs.preview)}><Icon name="preview" /></button>
      <button class="ib" title={bprefs.treeSide === "left" ? "Folders on the right" : "Folders on the left"} onclick={() => setPref("treeSide", bprefs.treeSide === "left" ? "right" : "left")}>
        <Icon name={bprefs.treeSide === "left" ? "tree-right" : "tree-left"} />
      </button>
    </div>
  </div>

  {#if notice}<div class="notice" role="status">{notice}</div>{/if}

  <div class="body">
    <!-- left pane: folder tree + preview -->
    <aside class="side">
      <nav class="tree" aria-label="Folders">
        <ul role="tree">
          <li role="treeitem" aria-selected={!browser.path}>
            <div class="row pc" class:here={!browser.path} onclick={() => navigate("")} onkeydown={(e) => e.key === "Enter" && navigate("")} role="button" tabindex="-1">
              <span class="ic"><Icon name="pc" size={16} /></span><span>This PC</span>
            </div>
          </li>
          {#if bprefs.root}
            <!-- "Show only this folder": one tree root, and a clear way back to all folders. -->
            <li class="rootbar">
              <span title={bprefs.root}>Only this folder</span>
              <button onclick={() => setRoot("")} title="Show all drives and folders again">Show all</button>
            </li>
            <TreeNode node={{ path: bprefs.root, label: nameOf(bprefs.root) || bprefs.root, icon: "folder" }} current={browser.path} {sep} follow={true}
              onopen={navigate} oncontext={onTreeMenu} refreshKey={treeRefresh} ondragto={(ev) => dragTarget(ev, null)} ondropto={(ev, n) => dropOn(ev, n.path)} />
          {:else}
            {#each bprefs.favs as f (f.path)}
              <TreeNode node={{ path: f.path, label: f.label, icon: "star", hint: f.path }} current={browser.path} {sep} follow={samePath(f.path, followRoot, sep)}
                onopen={navigate} oncontext={onTreeMenu} refreshKey={treeRefresh} ondragto={(ev) => dragTarget(ev, null)} ondropto={(ev, n) => dropOn(ev, n.path)} />
            {/each}
            {#if bprefs.favs.length}<li class="gap" aria-hidden="true"></li>{/if}
            {#each roots.known as k (k.id)}
              <TreeNode node={{ path: k.path, label: KNOWN_LABEL[k.id] ?? k.id, icon: k.id }} current={browser.path} {sep} follow={samePath(k.path, followRoot, sep)}
                onopen={navigate} oncontext={onTreeMenu} refreshKey={treeRefresh} ondragto={(ev) => dragTarget(ev, null)} ondropto={(ev, n) => dropOn(ev, n.path)} />
            {/each}
            <li class="gap" aria-hidden="true"></li>
            {#each roots.volumes as v (v.path)}
              <TreeNode node={{ path: v.path, label: volLabel(v), icon: volIcon(v), hint: v.total ? `${formatBytes(v.free)} free of ${formatBytes(v.total)}` : "No disk in the drive" }}
                current={browser.path} {sep} follow={samePath(v.path, followRoot, sep)} onopen={navigate} oncontext={onTreeMenu} refreshKey={treeRefresh} ondragto={(ev) => dragTarget(ev, null)} ondropto={(ev, n) => dropOn(ev, n.path)} />
            {/each}
          {/if}
        </ul>
      </nav>
      {#if bprefs.preview}
        <section class="pv" aria-label="Preview">
          <div class="pvimg">
            {#if one && thumbable(one)}
              {#key one.id}<img src={fthumbUrl(one.id, "l")} alt={one.name} onerror={() => thumbFailed(one.id)} />{/key}
            {:else if one}
              <Icon name={one.dir ? "folder" : one.kind} size={72} />
            {:else if selEntries.length > 1}
              <div class="multi">
                <span class="muted">{plural(selEntries.length, "item", "items")} selected</span>
                {#if compareable().length >= 2 && compareable().length <= 4}
                  <button onclick={startCompare} title="Compare side by side (P)"><Icon name="compare" size={16} /> Compare side by side</button>
                {/if}
              </div>
            {:else}
              <span class="muted">Select a photo to preview it</span>
            {/if}
          </div>
          {#if one}
            <div class="pvinfo" title={one.name}>
              <strong>{one.name}</strong>
              <span>{typeLabel(one)}{one.dir ? "" : " · " + formatBytes(one.size)} · {fmtDate(one.modified)}</span>
            </div>
          {/if}
        </section>
      {/if}
    </aside>
    <div class="split" role="separator" aria-orientation="vertical" onpointerdown={dragSplit} title="Drag to resize"></div>

    <!-- file area -->
    <div class="files view-{bprefs.view} size-{bprefs.size}" bind:this={area} tabindex="0" aria-label="Files" role="listbox" aria-multiselectable="true"
      onkeydown={onAreaKey} oncontextmenu={onBackgroundMenu}
      onclick={(e) => { if (!e.target.closest("[data-id]")) browser.selected = []; }}>
      {#if browser.error}
        <div class="msg error">{browser.error}</div>
      {:else if !browser.path}
        <!-- This PC -->
        {#if bprefs.favs.length}
          <h2 class="hd">Favourites</h2>
          <div class="drives">
            {#each bprefs.favs as f (f.path)}
              <button class="drive" onclick={() => navigate(f.path)} title={f.path} oncontextmenu={(ev) => onTreeMenu(ev, { path: f.path, label: f.label })}>
                <Icon name="star" size={30} /><span><strong>{f.label}</strong><small class="muted">{f.path}</small></span>
              </button>
            {/each}
          </div>
        {/if}
        <h2 class="hd">Folders</h2>
        <div class="drives">
          {#each roots.known as k (k.id)}
            <button class="drive" ondblclick={() => navigate(k.path)} onclick={() => navigate(k.path)} title={k.path}>
              <Icon name={k.id} size={30} /><span><strong>{KNOWN_LABEL[k.id] ?? k.id}</strong><small class="muted">{k.path}</small></span>
            </button>
          {/each}
        </div>
        <h2 class="hd">Drives</h2>
        <div class="drives">
          {#each roots.volumes as v (v.path)}
            <button class="drive" onclick={() => navigate(v.path)} title={v.path}>
              <Icon name={volIcon(v)} size={30} />
              <span>
                <strong>{volLabel(v)}</strong>
                {#if v.total}
                  <span class="bar" title="{Math.round((100 * (v.total - v.free)) / v.total)}% used"><i style="width:{(100 * (v.total - v.free)) / v.total}%"></i></span>
                  <small class="muted">{formatBytes(v.free)} free of {formatBytes(v.total)}</small>
                {:else}<small class="muted">No disk in the drive</small>{/if}
              </span>
            </button>
          {/each}
        </div>
      {:else if browser.listing && !entries.length}
        <div class="msg muted">
          {#if browser.listing.entries.length}Nothing here matches the filter.
          {:else}This folder is empty.{/if}
          {#if browser.listing.hidden_count}<br />({plural(browser.listing.hidden_count, "hidden or system item is", "hidden or system items are")} not shown){/if}
        </div>
      {:else if bprefs.view === "details"}
        <table class="det">
          <thead>
            <tr>
              {#each [["name", "Name"], ["type", "Type"], ["size", "Size"], ["date", "Date modified"], ["rating", "Stars"]] as [k, t]}
                <th class:num={k === "size"} onclick={() => sortByCol(k)}>
                  {t}{#if bprefs.sortBy === k}<span class="arrow">{bprefs.sortDesc ? "▼" : "▲"}</span>{/if}
                </th>
              {/each}
            </tr>
          </thead>
          <tbody>
            {#each entries as e (e.id)}
              <tr data-id={e.id} class:sel={selSet.has(e.id)} class:cut={clip?.mode === "move" && clip.ids.includes(e.id)} class:drop={dragOver === e.id}
                draggable="true" ondragstart={(ev) => onDragStart(ev, e)}
                ondragover={(ev) => e.dir && dragTarget(ev, e.id)} ondragleave={() => dragOver === e.id && (dragOver = null)} ondrop={(ev) => e.dir && dropOn(ev, pathOf(e))}
                onclick={(ev) => select(e, ev)} ondblclick={() => openEntry(e)} oncontextmenu={(ev) => onItemMenu(ev, e)}>
                <td class="nm"><Icon name={e.dir ? "folder" : e.kind} size={17} /><span>{e.name}</span></td>
                <td>{typeLabel(e)}</td>
                <td class="num">{e.dir ? "" : formatBytes(e.size)}</td>
                <td>{fmtDate(e.modified)}</td>
                <td class="stars-td">{#if marks[e.id]?.rating}<span class="stars" title="{marks[e.id].rating} of 5 stars">{"★".repeat(marks[e.id].rating)}</span>{/if}{#if marks[e.id]?.tagged}<span class="tagdot" title="Tagged">●</span>{/if}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        <div class="grid">
          {#each entries as e (e.id)}
            <div class="cell" class:sel={selSet.has(e.id)} class:cut={clip?.mode === "move" && clip.ids.includes(e.id)} class:drop={dragOver === e.id} data-id={e.id} title={e.name}
              draggable="true" ondragstart={(ev) => onDragStart(ev, e)}
              ondragover={(ev) => e.dir && dragTarget(ev, e.id)} ondragleave={() => dragOver === e.id && (dragOver = null)} ondrop={(ev) => e.dir && dropOn(ev, pathOf(e))}
              onclick={(ev) => select(e, ev)} ondblclick={() => openEntry(e)} oncontextmenu={(ev) => onItemMenu(ev, e)}
              role="option" aria-selected={selSet.has(e.id)} tabindex="-1" onkeydown={() => {}}>
              {#if bprefs.view === "thumbs"}
                <div class="frame">
                  <div class="pic" use:lazySamples={e}>
                    {#if e.dir}
                      <div class="fold">
                        <Icon name="folder" size={SIZE_PX[bprefs.size] * 0.8} />
                        {#if samples[e.id]?.length}
                          <div class="mini n{samples[e.id].length}">
                            {#each samples[e.id] as sid (sid)}<img src={fthumbUrl(sid, "s")} alt="" loading="lazy" onerror={(x) => (x.currentTarget.style.visibility = "hidden")} />{/each}
                          </div>
                        {/if}
                      </div>
                    {:else if thumbable(e)}
                      <img src={fthumbUrl(e.id, bprefs.size)} alt="" loading="lazy" decoding="async" onerror={() => thumbFailed(e.id)} />
                      {#if e.kind === "video"}<span class="play" aria-hidden="true">▶</span>{/if}
                      {#if marks[e.id]?.rating}<span class="ostars" title="{marks[e.id].rating} of 5 stars">{"★".repeat(marks[e.id].rating)}</span>{/if}
                    {:else}
                      <Icon name={e.kind} size={SIZE_PX[bprefs.size] * 0.55} />
                    {/if}
                    {#if marks[e.id]?.tagged}<span class="otag" title="Tagged (Q)"><Icon name="tag" size={14} /></span>{/if}
                  </div>
                  {#if !e.dir && bprefs.size !== "s"}
                    <div class="strip"><span>{e.size ? formatBytes(e.size) : ""}</span><span>{typeLabel(e)}</span></div>
                  {/if}
                </div>
                <div class="name">{e.name}</div>
              {:else if bprefs.view === "tiles"}
                <div class="tile">
                  <div class="tpic">
                    {#if !e.dir && thumbable(e)}<img src={fthumbUrl(e.id, "s")} alt="" loading="lazy" onerror={() => thumbFailed(e.id)} />
                    {:else}<Icon name={e.dir ? "folder" : e.kind} size={40} />{/if}
                  </div>
                  <div class="tt">
                    <span class="name">{e.name}</span>
                    <small class="muted">{typeLabel(e)}{e.dir ? "" : " · " + formatBytes(e.size)}</small>
                    <small class="muted">{fmtDate(e.modified)}</small>
                    {#if marks[e.id]?.rating || marks[e.id]?.tagged}<small>{#if marks[e.id]?.rating}<span class="stars">{"★".repeat(marks[e.id].rating)}</span>{/if}{#if marks[e.id]?.tagged}<span class="tagdot" title="Tagged">● tagged</span>{/if}</small>{/if}
                  </div>
                </div>
              {:else}
                <div class="li"><Icon name={e.dir ? "folder" : e.kind} size={16} /><span class="name">{e.name}</span>{#if marks[e.id]?.rating}<span class="stars">{"★".repeat(marks[e.id].rating)}</span>{/if}{#if marks[e.id]?.tagged}<span class="tagdot" title="Tagged">●</span>{/if}</div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
      {#if browser.loading}<div class="loading"><span class="spin"></span> Reading the folder…</div>{/if}
    </div>
  </div>

  <!-- status bar -->
  <footer class="status">
    {#if browser.path && browser.listing}
      <span>{plural(counts.folders, "folder", "folders")}</span>
      <span>{plural(counts.files, "file", "files")} ({formatBytes(counts.bytes)})</span>
      <span>{browser.selected.length ? `${browser.selected.length} selected (${formatBytes(selBytes)})` : "Nothing selected"}</span>
      {#if counts.hiddenByFilter > 0}<span class="muted">{counts.hiddenByFilter} not shown (filter)</span>{/if}
      {#if browser.listing.hidden_count}<span class="muted">{browser.listing.hidden_count} hidden/system</span>{/if}
    {:else}
      <span>{plural(roots.volumes.length, "drive", "drives")}</span>
    {/if}
    <span class="spacer"></span>
    <span class="muted">Right-click a folder to sort it, find duplicates, people or places</span>
  </footer>
</div>

{#if menu}<ContextMenu {...menu} onclose={() => { menu = null; area?.focus(); }} />{/if}

{#if ask?.kind === "name"}
  <dialog class="modal" open use:modal onclose={() => (ask = null)}>
    <form method="dialog" onsubmit={(e) => { e.preventDefault(); submitName(); }}>
      <h2>{ask.title}</h2>
      <input bind:this={askInput} bind:value={ask.value} spellcheck="false" aria-label="Name"
        use:focusName={ask.selectStem} oninput={() => ask.error && (ask.error = "")} />
      {#if ask.error}<p class="err">{ask.error}</p>{/if}
      <div class="btns">
        <button type="button" onclick={() => (ask = null)}>Cancel</button>
        <button class="primary" type="submit">{ask.okLabel}</button>
      </div>
    </form>
  </dialog>
{:else if ask?.kind === "delete"}
  <dialog class="modal danger" open use:modal onclose={() => (ask = null)}>
    <h2>Delete {ask.what} permanently?</h2>
    {#if ask.noBin}<p class="why"><strong>There is no Recycle Bin here.</strong> {ask.noBin}</p>{/if}
    <p>
      {plural(ask.count, "item", "items")} ({ask.size}){ask.folders ? `, including ${plural(ask.folders, "folder", "folders")} with everything inside` : ""}
      will be <strong>deleted for good</strong>. This <strong>cannot be undone</strong>.
    </p>
    <div class="btns">
      <button class="primary" onclick={() => (ask = null)}>Keep {ask.count === 1 ? "it" : "them"}</button>
      <button class="dangerbtn" onclick={() => runDelete(ask.ids, true)}>Delete permanently</button>
    </div>
  </dialog>
{/if}

{#if job}
  <div class="jobwrap" role="status">
    <div class="jobcard">
      <strong>{job.title}</strong>
      {#if job.prog && job.prog.bytes_total}
        <div class="pbar"><i style="width:{(100 * job.prog.bytes_done) / job.prog.bytes_total}%"></i></div>
        <span class="muted">{job.prog.files_done} of {job.prog.files_total} files · {formatBytes(job.prog.bytes_done)} of {formatBytes(job.prog.bytes_total)}</span>
        <span class="muted cur">{job.prog.current}</span>
      {:else if job.prog && job.prog.files_total}
        <div class="pbar"><i style="width:{(100 * job.prog.files_done) / job.prog.files_total}%"></i></div>
        <span class="muted">{job.prog.files_done} of {job.prog.files_total} photos</span>
        <span class="muted cur">{job.prog.current}</span>
      {:else}
        <div class="pbar busy"><i></i></div>
      {/if}
      {#if job.cancel}<button onclick={typeof job.cancel === "function" ? job.cancel : cancelFileOp}>{job.stop ?? "Stop after this file"}</button>{/if}
    </div>
  </div>
{/if}
{#if create}<CreateDialog items={create.items} tab={create.tab} folder={nameOf(browser.path)} onclose={async (changed, name) => {
  create = null;
  if (changed) await refreshSelect(name ? [joinIn(browser.path, name, sep)] : []);
  area?.focus();
}} />{/if}
{#if batch}<BatchDialog items={batch.items} tab={batch.tab} onclose={async (changed, go) => {
  batch = null;
  if (go) await navigate(go);
  else if (changed) await refresh();
  area?.focus();
}} />{/if}
{#if slideshow}<Slideshow items={slideshow.items} start={slideshow.start} onclose={(last) => {
  slideshow = null;
  const e = entries.find((x) => x.id === last);
  if (e) { browser.selected = [last]; browser.anchor = last; tick().then(() => area?.querySelector(`[data-id="${last}"]`)?.scrollIntoView({ block: "nearest" })); }
  area?.focus();
}} />{/if}
{#if compare}<Compare items={compare} onclose={() => { compare = null; area?.focus(); }} />{/if}
{#if viewer}<Viewer items={viewer.items} index={viewer.index} onchanged={() => (viewerChanged = true)} onclose={async (last) => {
  viewer = null;
  if (viewerChanged) {
    // A picture was saved or its information changed: re-read the folder (new copies, Undo label).
    viewerChanged = false;
    const e = entries.find((x) => x.id === last);
    await refreshSelect(e ? [pathOf(e)] : []);
    return;
  }
  if (last != null) { browser.selected = [last]; browser.anchor = last; tick().then(() => area?.querySelector(`[data-id="${last}"]`)?.scrollIntoView({ block: "nearest" })); }
  area?.focus();
}} />{/if}

<style>
  .ostars { position: absolute; bottom: 8px; left: 8px; background: rgba(0, 0, 0, 0.6); color: #ffc83d; font-size: 12px; letter-spacing: 1px; border-radius: 4px; padding: 0 5px; pointer-events: none; }
  .otag { position: absolute; top: 6px; right: 6px; display: grid; place-items: center; width: 22px; height: 22px; border-radius: 50%; background: var(--accent); color: var(--accent-text); pointer-events: none; }
  .stars { color: #e0a800; letter-spacing: 1px; white-space: nowrap; }
  .tagdot { color: var(--accent); margin-left: 4px; }
  .li .stars { margin-left: 6px; font-size: 11px; }
  .stars-td { white-space: nowrap; }
  select.sel.on { border-color: var(--accent); background: var(--accent-soft); }
  .multi { display: flex; flex-direction: column; align-items: center; gap: 10px; }
  .multi button { display: inline-flex; align-items: center; gap: 6px; }
  .browser { display: flex; flex-direction: column; height: 100%; min-height: 0; background: var(--bg); }
  /* toolbar */
  .tb { display: flex; align-items: center; gap: 8px; padding: 6px 10px; background: var(--surface); border-bottom: 1px solid var(--border); flex-wrap: wrap; }
  .grp { display: flex; gap: 2px; }
  .seg { background: var(--surface-2); border-radius: 7px; padding: 2px; }
  .ib { display: grid; place-items: center; width: 32px; height: 30px; padding: 0; border: 1px solid transparent; background: none; border-radius: 6px; color: var(--text); }
  .ib:hover:not(:disabled) { background: var(--surface-2); border-color: var(--border); }
  .ib.on { background: var(--accent-soft); border-color: var(--accent); }
  .ib:disabled { opacity: 0.35; }
  .sz { font-weight: 700; font-size: 13px; }
  .addr { flex: 1 1 220px; display: flex; align-items: center; gap: 6px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg); padding: 0 8px; min-width: 200px; }
  .addr input { flex: 1; border: none; background: none; padding: 6px 0; font: 13.5px var(--font); color: var(--text); outline: none; }
  .aic { color: var(--muted); display: grid; }
  .sel { padding: 5px 6px; font-size: 13px; border-radius: 6px; }
  .search { display: flex; align-items: center; gap: 5px; border: 1px solid var(--border); border-radius: 6px; padding: 0 7px; color: var(--muted); background: var(--bg); }
  .search input { border: none; background: none; width: 100px; padding: 6px 0; font: 13px var(--font); color: var(--text); outline: none; }
  .notice { padding: 6px 14px; background: var(--amber-soft); color: var(--amber); font-weight: 600; font-size: 13.5px; }

  /* body: tree | splitter | files (mirrored when the tree is on the right) */
  .body { flex: 1; display: grid; grid-template-columns: var(--tree-w) 5px 1fr; min-height: 0; }
  .right .body { grid-template-columns: 1fr 5px var(--tree-w); }
  .right .side { order: 3; }
  .right .split { order: 2; }
  .right .files { order: 1; }
  .side { display: grid; grid-template-rows: 1fr auto; min-height: 0; background: var(--surface); }
  .split { cursor: col-resize; background: var(--border); }
  .split:hover { background: var(--accent); }
  .tree { overflow: auto; padding: 6px 4px; min-height: 120px; }
  .tree ul { margin: 0; padding: 0; }
  .tree li { list-style: none; }
  .tree .gap { height: 8px; }
  .row.pc { display: flex; align-items: center; gap: 6px; height: 24px; padding-left: 21px; border-radius: 4px; font-size: 13.5px; cursor: default; }
  .row.pc:hover { background: var(--surface-2); }
  .row.pc.here { background: var(--accent); color: var(--accent-text); }
  .row.pc .ic { display: grid; color: inherit; }
  .pv { border-top: 1px solid var(--border); display: grid; grid-template-rows: 230px auto; }
  .pvimg { display: grid; place-items: center; background: var(--surface-2); overflow: hidden; color: var(--muted); font-size: 13px; padding: 6px; }
  .pvimg img { max-width: 100%; max-height: 218px; object-fit: contain; box-shadow: 0 1px 4px rgba(0, 0, 0, 0.25); }
  .pvinfo { display: grid; gap: 1px; padding: 5px 8px; font-size: 12px; border-top: 1px solid var(--border); }
  .pvinfo strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12.5px; }
  .pvinfo span { color: var(--muted); }

  /* file area */
  .files { position: relative; overflow: auto; padding: 10px; outline: none; min-width: 0; }
  .files:focus-visible { box-shadow: inset 0 0 0 2px var(--accent-soft); }
  .msg { padding: 40px; text-align: center; font-size: 15px; }
  .msg.error { color: var(--red); }
  .loading { position: absolute; top: 10px; right: 14px; display: flex; gap: 8px; align-items: center; font-size: 13px; color: var(--muted); background: var(--surface); padding: 4px 10px; border-radius: 6px; box-shadow: var(--shadow); }
  .hd { font-size: 14px; color: var(--muted); font-weight: 600; margin: 6px 4px 8px; }
  .drives { display: grid; grid-template-columns: repeat(auto-fill, minmax(250px, 1fr)); gap: 10px; margin-bottom: 18px; }
  .drive { display: flex; gap: 12px; align-items: center; text-align: left; padding: 10px 12px; font-weight: 400; color: var(--accent); }
  .drive span { display: grid; gap: 3px; min-width: 0; color: var(--text); }
  .drive small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
  .bar { height: 6px; background: var(--surface-2); border-radius: 3px; overflow: hidden; width: 160px; }
  .bar i { display: block; height: 100%; background: var(--accent); }

  .grid { display: flex; flex-wrap: wrap; gap: 6px; align-content: flex-start; }
  .cell { content-visibility: auto; contain-intrinsic-size: auto calc(var(--cell) + 60px); border-radius: 8px; cursor: default; user-select: none; }
  /* thumbnails */
  .view-thumbs .cell { width: calc(var(--cell) + 22px); padding: 4px; }
  .frame { border: 1px solid var(--border); border-radius: 9px; background: var(--surface); overflow: hidden; }
  .cell:hover .frame { border-color: var(--muted); }
  .cell.sel .frame { border: 2px solid var(--accent); margin: -1px; box-shadow: 0 0 0 1px var(--accent-soft); }
  .pic { position: relative; box-sizing: border-box; height: calc(var(--cell) + 12px); display: grid; place-items: center; padding: 6px; color: var(--muted); }
  .pic img { max-width: var(--cell); max-height: var(--cell); object-fit: contain; box-shadow: 0 1px 3px rgba(0, 0, 0, 0.25); }
  .play { position: absolute; bottom: 8px; left: 8px; background: rgba(0, 0, 0, 0.6); color: #fff; font-size: 11px; border-radius: 4px; padding: 1px 5px; }
  .fold { position: relative; display: grid; place-items: center; }
  .mini { position: absolute; inset: 30% 15% 14% 15%; display: grid; grid-template-columns: 1fr 1fr; gap: 3px; }
  .mini.n1 { grid-template-columns: 1fr; }
  .mini img { width: 100%; height: 100%; object-fit: cover; box-shadow: none; border: 1px solid #fff; }
  .strip { display: flex; justify-content: space-between; font: 11px var(--mono); color: var(--muted); padding: 2px 7px 3px; border-top: 1px solid var(--border); background: var(--surface-2); }
  .view-thumbs .name { text-align: center; font-size: 12.5px; padding: 4px 2px 2px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .view-thumbs .cell.sel .name { color: var(--accent); font-weight: 600; }
  /* tiles */
  .view-tiles .cell { width: 280px; padding: 4px; border: 1px solid transparent; }
  .view-tiles .cell.sel, .view-list .cell.sel { background: var(--accent-soft); border-color: var(--accent); }
  .view-tiles .cell:hover:not(.sel), .view-list .cell:hover:not(.sel) { background: var(--surface-2); }
  .tile { display: grid; grid-template-columns: 64px 1fr; gap: 10px; align-items: center; }
  .tpic { width: 64px; height: 64px; display: grid; place-items: center; color: var(--muted); }
  .tpic img { max-width: 64px; max-height: 64px; object-fit: contain; }
  .tt { display: grid; min-width: 0; }
  .tt .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13.5px; }
  .tt small { font-size: 12px; }
  /* list: names in columns */
  .view-list .grid { flex-direction: column; flex-wrap: wrap; max-height: calc(100vh - 230px); align-content: flex-start; gap: 0 12px; }
  .view-list .cell { width: 260px; border: 1px solid transparent; border-radius: 4px; }
  .li { display: flex; align-items: center; gap: 6px; padding: 2px 6px; font-size: 13.5px; }
  .li .name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* details */
  .det { width: 100%; border-collapse: collapse; font-size: 13.5px; }
  .det th { position: sticky; top: -10px; background: var(--surface); text-align: left; font-weight: 600; padding: 6px 10px; border-bottom: 1px solid var(--border); cursor: pointer; white-space: nowrap; }
  .det th:hover { background: var(--surface-2); }
  .det td { padding: 3px 10px; white-space: nowrap; }
  .det tr { cursor: default; user-select: none; }
  .det tbody tr:hover { background: var(--surface-2); }
  .det tr.sel { background: var(--accent); color: var(--accent-text); }
  .det .num { text-align: right; }
  .det .nm { display: flex; align-items: center; gap: 6px; max-width: 520px; }
  .det .nm span { overflow: hidden; text-overflow: ellipsis; }
  .arrow { font-size: 10px; padding-left: 5px; }

  /* housekeeping */
  .cell.cut, tr.cut { opacity: 0.5; }
  .cell.drop .frame, .cell.drop, tr.drop { outline: 2px dashed var(--accent); outline-offset: 1px; background: var(--accent-soft); }
  .modal {
    position: fixed; inset: 0; margin: auto; z-index: 1100; width: min(460px, 92vw); padding: 20px 22px;
    border: 1px solid var(--border); border-radius: 12px; background: var(--surface); color: var(--text);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.35);
  }
  .modal h2 { font-size: 18px; margin: 0 0 12px; }
  .modal input { width: 100%; box-sizing: border-box; font-size: 15px; padding: 8px 10px; }
  .modal .err { color: var(--red); margin: 8px 0 0; font-size: 14px; }
  .modal .btns { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
  .modal.danger { border: 2px solid var(--red); }
  .modal .why { background: var(--amber-soft); color: var(--text); padding: 8px 10px; border-radius: 6px; }
  .dangerbtn { background: var(--red); border-color: var(--red); color: #fff; font-weight: 650; }
  .jobwrap { position: fixed; inset: 0; z-index: 1050; display: grid; place-items: center; background: rgba(0, 0, 0, 0.25); }
  .jobcard { display: grid; gap: 8px; min-width: 360px; padding: 18px 20px; background: var(--surface); border-radius: 12px; box-shadow: 0 20px 60px rgba(0, 0, 0, 0.35); }
  .jobcard .cur { font-size: 12.5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; max-width: 420px; }
  .jobcard button { justify-self: end; }
  .pbar { height: 10px; border-radius: 5px; background: var(--surface-2); overflow: hidden; }
  .pbar i { display: block; height: 100%; background: var(--accent); transition: width 0.2s; }
  .pbar.busy i { width: 35%; animation: slide 1.2s ease-in-out infinite; }
  @keyframes slide { from { transform: translateX(-100%); } to { transform: translateX(300%); } }
  .status { display: flex; gap: 0; align-items: center; font-size: 12.5px; background: var(--surface); border-top: 1px solid var(--border); padding: 3px 6px; }
  .status > span { padding: 0 12px; border-right: 1px solid var(--border); }
  .status > span:last-child, .status .spacer { border: none; }
  .spacer { flex: 1; }
  .rootbar { display: flex; align-items: center; justify-content: space-between; gap: 6px; margin: 2px 6px 6px; padding: 4px 8px;
    background: var(--accent-soft); border-radius: var(--radius-sm); font-size: 12.5px; }
  .rootbar span { font-weight: 600; }
  .rootbar button { padding: 2px 8px; font-size: 12px; }
</style>
