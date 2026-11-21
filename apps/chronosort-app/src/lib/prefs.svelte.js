// User preferences (appearance), kept in localStorage. Nothing leaves the PC.

export const THEMES = [
  { id: "system", label: "System", hint: "Follows Windows light/dark" },
  { id: "light", label: "Light", hint: "Bright and calm" },
  { id: "dark", label: "Dark", hint: "Easy on the eyes at night" },
  { id: "navy", label: "Navy", hint: "Deep blue" },
  { id: "pastel", label: "Darker Pastel", hint: "Soft purple-grey" },
  { id: "contrast", label: "High contrast", hint: "Black and yellow, strongest contrast" },
];

export const FONTS = [
  { id: "system", label: "System default", css: '"Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif' },
  { id: "segoe", label: "Segoe UI", css: '"Segoe UI", sans-serif' },
  { id: "verdana", label: "Verdana (very readable)", css: "Verdana, Geneva, sans-serif" },
  { id: "tahoma", label: "Tahoma", css: "Tahoma, Verdana, sans-serif" },
  { id: "georgia", label: "Georgia (serif)", css: "Georgia, 'Times New Roman', serif" },
  { id: "consolas", label: "Consolas (fixed width)", css: "Consolas, 'Cascadia Mono', monospace" },
];

export const SIZES = [
  { id: "normal", label: "Normal", zoom: 1 },
  { id: "large", label: "Large", zoom: 1.15 },
  { id: "xlarge", label: "Extra large", zoom: 1.3 },
  { id: "huge", label: "Huge", zoom: 1.5 },
];

const KEY = "chronosort.prefs.v1";
const DEFAULTS = { theme: "system", font: "system", size: "normal" };

function load() {
  try {
    const p = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    return {
      theme: THEMES.some((t) => t.id === p.theme) ? p.theme : DEFAULTS.theme,
      font: FONTS.some((f) => f.id === p.font) ? p.font : DEFAULTS.font,
      size: SIZES.some((s) => s.id === p.size) ? p.size : DEFAULTS.size,
    };
  } catch {
    return { ...DEFAULTS };
  }
}

export const prefs = $state(load());

export function applyPrefs() {
  const root = document.documentElement;
  root.dataset.theme = prefs.theme;
  root.style.setProperty("--font", FONTS.find((f) => f.id === prefs.font).css);
  root.style.setProperty("--zoom", String(SIZES.find((s) => s.id === prefs.size).zoom));
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    /* storage full or disabled: the choice still applies for this session */
  }
}

export function resetPrefs() {
  Object.assign(prefs, DEFAULTS);
}
