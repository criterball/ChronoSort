// App-wide state. Lives outside the components so a sort or duplicate check
// in progress survives a visit to Settings or Help.

export const app = $state({
  view: "browse", // browse (start) | home (Tools) | sort | dups | people | places | settings | help
  busy: false, // a job is running: block navigation that would lose it
  helpTopic: null,
});

// A running AI download (Settings -> AI models). Kept here so the header can show
// it on every screen and Exit can warn: a long download must never look frozen.
export const dl = $state({ pack: null, name: "", prog: null, rate: 0 });
let last = null; // { t, done, file }
export function onDownloadProgress(p) {
  const now = performance.now();
  if (p.phase === "download" && last && last.file === p.file && p.done > last.done && now > last.t) {
    const r = ((p.done - last.done) * 1000) / (now - last.t);
    dl.rate = dl.rate ? dl.rate * 0.85 + r * 0.15 : r; // smoothed bytes per second
  }
  if (!last || now - last.t > 500 || last.file !== p.file) last = { t: now, done: p.done, file: p.file };
  dl.prog = p;
}
export function downloadEnded() {
  Object.assign(dl, { pack: null, name: "", prog: null, rate: 0 });
  last = null;
}
/** "about 12 min left" for the current file, or "" while the speed is unknown. */
export function timeLeft() {
  const p = dl.prog;
  if (!p || p.phase !== "download" || !dl.rate || !p.total) return "";
  const s = (p.total - p.done) / dl.rate;
  if (s < 60) return "less than a minute left";
  if (s < 3600) return `about ${Math.ceil(s / 60)} min left`;
  return `about ${Math.floor(s / 3600)} h ${Math.round((s % 3600) / 60)} min left`;
}

export const sort = $state({
  step: 0, // 0 folders, 1 overview, 2 check, 3 names, 4 final, 5 copy
  settings: { source: "", dest: "", mode: "copy", naming: "keep", months: "ask" },
  analysis: null,
  resolutions: {},
  plan: null,
  namingChoice: "keep",
  template: "{year}-{month}-{day}_{original}",
  runKey: 0, // bumps for each new copy run
  autostart: false, // --analyze on the command line
  restored: "", // message when earlier Check decisions were picked up
  ai: {}, // rel_path -> { guess } | { error }: the local AI's year estimate (a suggestion only)
});

export function resetSort(keepFolders = true) {
  const { source, dest, mode } = sort.settings;
  sort.step = 0;
  sort.analysis = null;
  sort.resolutions = {};
  sort.plan = null;
  sort.ai = {};
  sort.settings = keepFolders
    ? { source, dest, mode, naming: sort.settings.naming, months: "ask" }
    : { source: "", dest: "", mode: "copy", naming: "keep", months: "ask" };
}

export const dups = $state({
  phase: "pick", // pick | scanning | results | working | done
  folders: [],
  includeAll: false,
  similar: false, // also find near-duplicates (resized / re-saved photos)
  scan: null,
  keep: {}, // group hash -> index of the copy to keep
  action: "move", // move | trash | report
  target: "",
  report: null,
  undone: null,
});

export function resetDups() {
  Object.assign(dups, { phase: "pick", scan: null, keep: {}, action: "move", target: "", report: null, undone: null });
}

export const people = $state({
  phase: "pick", // pick | scanning | results
  folder: "",
  result: null, // find_faces output
  groups: [], // [{ id, faces: [face index], suggestion, name, saved, pick }]
  known: [], // named people
  showSingles: false,
  autostart: false, // start a face search as soon as the screen opens (from the sort flow)
});

export const places = $state({
  phase: "pick", // pick | choose | done
  folder: "",
  scan: null, // { folder, files: [{ rel, kind, key, has_place }], runs }
  chosen: {}, // rel -> ticked
  target: null, // { kind: "city", place } | { kind: "region", name, country, code } | { kind: "country", name, code }
  report: null,
  undone: null,
});

export function resetPlaces() {
  Object.assign(places, { phase: "pick", scan: null, chosen: {}, target: null, report: null, undone: null });
}

export function resetPeople() {
  Object.assign(people, { phase: "pick", result: null, groups: [], showSingles: false });
}

// File browser (the start screen). View choices are remembered between runs.
const BKEY = "chronosort.browser.v1";
const BDEF = { view: "thumbs", size: "m", show: "all", sortBy: "name", sortDesc: false, treeSide: "left", preview: true, showHidden: false, last: "" };
function loadBrowserPrefs() {
  try {
    return { ...BDEF, ...JSON.parse(localStorage.getItem(BKEY) ?? "{}") };
  } catch {
    return { ...BDEF };
  }
}
export const bprefs = $state(loadBrowserPrefs());
export function saveBrowserPrefs() {
  try {
    localStorage.setItem(BKEY, JSON.stringify(bprefs));
  } catch {
    /* not saved: still works this session */
  }
}

export const browser = $state({
  path: "", // current folder ("" = "This PC")
  listing: null, // list_dir result
  loading: false,
  error: "",
  back: [],
  forward: [],
  selected: [], // entry ids
  anchor: null, // id for Shift-click ranges
  filter: "", // name filter (search box)
});
