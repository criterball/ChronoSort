// Path helpers for the File browser. `sep` is the OS separator from the backend
// ("\\" on Windows, "/" elsewhere). Windows paths compare case-insensitively.

const norm = (p, sep) => {
  if (!p) return "";
  let s = sep === "\\" ? p.replace(/\//g, "\\").toLowerCase() : p;
  // Drop trailing separators but keep roots ("C:\", "/").
  while (s.length > 1 && s.endsWith(sep) && !(sep === "\\" && /^[a-z]:\\$/.test(s))) s = s.slice(0, -1);
  return s;
};

export const samePath = (a, b, sep) => !!a && !!b && norm(a, sep) === norm(b, sep);

/** Is `p` the folder `dir` or somewhere below it? */
export function isInside(p, dir, sep) {
  const a = norm(p, sep), d = norm(dir, sep);
  if (!a || !d) return false;
  if (a === d) return true;
  return a.startsWith(d.endsWith(sep) ? d : d + sep);
}

export const joinIn = (dir, name, sep) => (dir.endsWith(sep) ? dir + name : dir + sep + name);

export function nameOf(p) {
  const t = p.replace(/[\\/]+$/, "");
  return t.split(/[\\/]/).pop() || p;
}

/** "C:\Users\anna\Pictures" -> [{label:"C:\", path:"C:\"}, {label:"Users", path:"C:\Users"}, …] */
export function crumbs(p, sep) {
  if (!p) return [];
  const out = [];
  if (sep === "\\") {
    const m = p.match(/^([A-Za-z]:\\|\\\\[^\\]+\\[^\\]+\\?)/);
    const root = m ? (m[1].endsWith("\\") ? m[1] : m[1] + "\\") : "";
    out.push({ label: root.replace(/\\$/, "") || p, path: root || p });
    let cur = root;
    for (const part of p.slice(root.length).split("\\").filter(Boolean)) {
      cur = joinIn(cur, part, sep);
      out.push({ label: part, path: cur });
    }
  } else {
    out.push({ label: "/", path: "/" });
    let cur = "/";
    for (const part of p.split("/").filter(Boolean)) {
      cur = joinIn(cur, part, sep);
      out.push({ label: part, path: cur });
    }
  }
  return out;
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
/** Natural order: IMG_2 before IMG_10. Folders always first. */
export function sortEntries(list, by, desc) {
  const dir = desc ? -1 : 1;
  const key = {
    name: (a, b) => collator.compare(a.name, b.name),
    date: (a, b) => a.modified - b.modified || collator.compare(a.name, b.name),
    size: (a, b) => a.size - b.size || collator.compare(a.name, b.name),
    type: (a, b) => collator.compare(a.ext, b.ext) || collator.compare(a.name, b.name),
  }[by] ?? ((a, b) => collator.compare(a.name, b.name));
  return [...list].sort((a, b) => (b.dir - a.dir) || dir * key(a, b));
}
