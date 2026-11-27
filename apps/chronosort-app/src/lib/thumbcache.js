// The thumbnail cache's size limit (megabytes; 0 = no limit), kept with the
// other settings in this browser profile — which, like the cache, lives next to
// ChronoSort in portable mode.
const KEY = "chronosort.thumbcache.v1";
export const LIMITS = [
  [500, "500 MB"], [1000, "1 GB"], [2000, "2 GB"], [5000, "5 GB"], [10000, "10 GB"], [0, "no limit"],
];
export function cacheLimit() {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "null");
    return LIMITS.some(([x]) => x === v?.limit) ? v.limit : 2000;
  } catch {
    return 2000;
  }
}
export function setCacheLimit(limit) {
  try {
    localStorage.setItem(KEY, JSON.stringify({ limit }));
  } catch {
    /* fine: the default is used next time */
  }
}
