"""Download a small public-domain face test set from Wikimedia Commons.

Test data only (not shipped). Keeps files whose Commons licence is public
domain (mostly US federal government works: NASA, White House, Congress).
Every file's source page, author and licence go into manifest.json.

Portrait searches (not whole person categories): categories also hold photos
*taken by* a person or objects about them, which makes bad labels.

Usage: python -I fetch_face_testdata.py OUT_DIR [per_person]
"""
import json
import os
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

UA = "ChronoSort-testdata/0.1 (offline photo sorter, local test fixtures)"
API = "https://commons.wikimedia.org/w/api.php"

# person folder -> Commons search queries (files namespace)
PEOPLE = {
    "john_glenn": ['"John Glenn" portrait', '"John Glenn" 1962 astronaut', '"John Glenn" STS-95'],
    "buzz_aldrin": ['"Buzz Aldrin" portrait', '"Edwin Aldrin" portrait'],
    "sally_ride": ['"Sally Ride" portrait'],
    "scott_kelly": ['"Scott Kelly" astronaut portrait'],
    "mark_kelly": ['"Mark Kelly" portrait', '"Mark Kelly" astronaut'],
    "barack_obama": ['"Barack Obama" official portrait', '"Barack Obama" portrait'],
    "joe_biden": ['"Joe Biden" official portrait', '"Joe Biden" portrait'],
    "peggy_whitson": ['"Peggy Whitson" portrait'],
    "neil_armstrong": ['"Neil Armstrong" portrait'],
}


def get(url, timeout=60):
    """GET with polite retries: honours Retry-After on 429/503, backs off otherwise."""
    for attempt in range(6):
        req = urllib.request.Request(url, headers={"User-Agent": UA})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return r.read()
        except urllib.error.HTTPError as e:
            if e.code not in (429, 503) or attempt == 5:
                raise
            wait = int(e.headers.get("Retry-After") or 0) or 5 * 2 ** attempt
            print(f"  server busy ({e.code}), waiting {wait}s", flush=True)
            time.sleep(min(wait, 120))


def api(**params):
    params.update(format="json")
    time.sleep(1.0)
    return json.loads(get(API + "?" + urllib.parse.urlencode(params), timeout=30))


def search(q):
    d = api(action="query", list="search", srnamespace=6, srlimit=50, srsearch=q + " filetype:bitmap")
    return [r["title"] for r in d.get("query", {}).get("search", [])]


def is_pd(meta):
    lic = (meta.get("LicenseShortName", {}).get("value", "") + " " + meta.get("License", {}).get("value", "")).lower()
    return "public domain" in lic or lic.strip().startswith("pd")


def info(titles):
    d = api(action="query", titles="|".join(titles), prop="imageinfo", iiprop="url|extmetadata|mime|size", iiurlwidth=900)
    for p in d.get("query", {}).get("pages", {}).values():
        ii = (p.get("imageinfo") or [None])[0]
        if ii:
            yield p["title"], ii


def strip_html(s):
    return re.sub(r"<[^>]+>", "", s or "").strip()


def main():
    out = sys.argv[1]
    per = int(sys.argv[2]) if len(sys.argv) > 2 else 10
    os.makedirs(out, exist_ok=True)
    mpath = os.path.join(out, "manifest.json")
    manifest = json.load(open(mpath, encoding="utf-8")) if os.path.exists(mpath) else []
    have_titles = {e["title"] for e in manifest}
    for person, queries in PEOPLE.items():
        got = sum(1 for e in manifest if e["person"] == person)
        if got >= per:
            print(f"{person}: already {got}", flush=True)
            continue
        titles = []
        for q in queries:
            titles += search(q)
        titles = [t for t in dict.fromkeys(titles) if t.lower().endswith((".jpg", ".jpeg")) and t not in have_titles]
        os.makedirs(os.path.join(out, person), exist_ok=True)
        for i in range(0, len(titles), 40):
            if got >= per:
                break
            for title, ii in info(titles[i:i + 40]):
                if got >= per:
                    break
                meta = ii.get("extmetadata", {})
                if ii.get("mime") != "image/jpeg" or not is_pd(meta) or ii.get("width", 0) < 400:
                    continue
                name = f"{person}_{got + 1:02d}.jpg"
                try:
                    data = get(ii.get("thumburl") or ii["url"])
                except Exception as e:
                    print(f"  skip {title}: {e}", flush=True)
                    continue
                with open(os.path.join(out, person, name), "wb") as f:
                    f.write(data)
                got += 1
                manifest.append({
                    "file": f"{person}/{name}",
                    "person": person,
                    "source": ii.get("descriptionurl"),
                    "title": title,
                    "licence": strip_html(meta.get("LicenseShortName", {}).get("value")),
                    "author": strip_html(meta.get("Artist", {}).get("value"))[:200],
                    "date": strip_html(meta.get("DateTimeOriginal", {}).get("value"))[:40],
                })
                time.sleep(2.0)  # be polite
        with open(mpath, "w", encoding="utf-8") as f:
            json.dump(manifest, f, indent=1, ensure_ascii=False)
        print(f"{person}: {got} (from {len(titles)} candidates)", flush=True)
    print("total", len(manifest))


if __name__ == "__main__":
    main()
