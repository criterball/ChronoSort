//! The offline place list (ExifTool's GeoNames cities, population ≥ 2000)
//! and a forgiving search: case- and accent-insensitive, best matches first.

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub city: String,
    pub region: String,
    pub country_code: String,
    pub country: String,
    pub time_zone: String,
    pub population: u64,
    pub lat: f64,
    pub lon: f64,
}

impl Place {
    /// "Paris, Île-de-France, France"
    pub fn label(&self) -> String {
        let mut parts = vec![self.city.as_str()];
        if !self.region.is_empty() && self.region != self.city {
            parts.push(&self.region);
        }
        parts.push(&self.country);
        parts.join(", ")
    }
}

/// A country (for "country only" places): name and code.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Country {
    pub name: String,
    pub code: String,
}

/// Lower case, accents removed: "Zürich" -> "zurich", "Kraków" -> "krakow".
pub fn fold(s: &str) -> String {
    s.nfd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect::<String>().to_lowercase()
}

/// Split one CSV line (ExifTool quotes fields that contain commas).
fn split_csv(line: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// Parse `exiftool -api geolocation -listgeo` output.
pub fn parse_listgeo(text: &str) -> Vec<Place> {
    text.lines()
        .filter_map(|l| {
            let f = split_csv(l.trim_end_matches('\r'));
            if f.len() < 10 || f[0] == "City" {
                return None;
            }
            Some(Place {
                city: f[0].clone(),
                region: f[1].clone(),
                country_code: f[3].clone(),
                country: f[4].clone(),
                time_zone: f[5].clone(),
                population: f[7].parse().unwrap_or(0),
                lat: f[8].parse().ok().filter(|v: &f64| (-90.0..=90.0).contains(v))?,
                lon: f[9].parse().ok().filter(|v: &f64| (-180.0..=180.0).contains(v))?,
            })
        })
        .collect()
}

pub struct Places {
    all: Vec<Place>,
    /// Folded "city region country" per place, for searching.
    keys: Vec<(String, String)>,
}

impl Places {
    pub fn new(all: Vec<Place>) -> Self {
        let keys = all
            .iter()
            .map(|p| (fold(&p.city), fold(&format!("{} {} {}", p.region, p.country, p.country_code))))
            .collect();
        Places { all, keys }
    }

    pub fn len(&self) -> usize {
        self.all.len()
    }

    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    /// Up to `n` places for what the user typed: "paris", "paris france",
    /// "springfield, il". City name matches first (exact, then starts-with,
    /// then contains); more words narrow by region/country; bigger places first.
    pub fn search(&self, query: &str, n: usize) -> Vec<&Place> {
        let q = fold(query.trim());
        let mut words = q.split([',', ' ']).filter(|w| !w.is_empty());
        let Some(first) = words.next() else { return vec![] };
        let rest: Vec<&str> = words.collect();
        // The city part may be several words ("new york"): try the longest prefix of the query that matches a city.
        let mut hits: Vec<(u8, u64, usize)> = vec![];
        for (i, (city, more)) in self.keys.iter().enumerate() {
            let full = format!("{city} {more}");
            let rank = if city.as_str() == q.split(',').next().unwrap_or("").trim() {
                0
            } else if city.as_str() == first {
                1
            } else if city.starts_with(first) {
                2
            } else if city.contains(first) {
                3
            } else {
                continue;
            };
            if !rest.iter().all(|w| full.contains(w)) {
                continue;
            }
            hits.push((rank, u64::MAX - self.all[i].population, i));
        }
        hits.sort_unstable();
        hits.into_iter().take(n).map(|(_, _, i)| &self.all[i]).collect()
    }

    /// Countries matching what the user typed (for "country only").
    pub fn countries(&self, query: &str, n: usize) -> Vec<Country> {
        let q = fold(query.trim());
        let mut seen = std::collections::BTreeMap::new();
        for p in &self.all {
            if q.is_empty() || fold(&p.country).starts_with(&q) || fold(&p.country_code) == q {
                seen.entry(p.country.clone()).or_insert_with(|| p.country_code.clone());
            }
        }
        seen.into_iter().take(n).map(|(name, code)| Country { name, code }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Geolocation database:\n\
City,Region,Subregion,CountryCode,Country,TimeZone,FeatureCode,Population,Latitude,Longitude\n\
Paris,Texas,Lamar County,US,United States,America/Chicago,PPLA2,25000,33.6609,-95.5556\n\
Paris,Île-de-France,Paris,FR,France,Europe/Paris,PPLC,2100000,48.8534,2.3488\n\
Zürich,Zurich,Bezirk Zürich,CH,Switzerland,Europe/Zurich,PPLA,420000,47.3667,8.5500\n\
New York City,New York,,US,United States,America/New_York,PPL,8800000,40.7143,-74.0060\n\
Newport,\"Wales, UK\",Newport,GB,United Kingdom,Europe/London,PPLA2,150000,51.5877,-2.9984\n\
Broken,Row\n";

    #[test]
    fn parses_and_searches() {
        let p = Places::new(parse_listgeo(SAMPLE));
        assert_eq!(p.len(), 5, "header and broken rows are skipped");
        assert_eq!(p.search("paris", 5)[0].country, "France", "bigger place first");
        assert_eq!(p.search("Paris, texas", 5)[0].region, "Texas");
        assert_eq!(p.search("zurich", 5)[0].city, "Zürich", "accents do not matter");
        assert_eq!(p.search("new york", 5)[0].city, "New York City");
        assert_eq!(p.search("newport", 5)[0].region, "Wales, UK", "quoted CSV field");
        assert!(p.search("", 5).is_empty() && p.search("xyzzy", 5).is_empty());
        assert_eq!(p.search("paris", 5)[0].label(), "Paris, Île-de-France, France");
        assert_eq!(p.countries("fr", 5)[0], Country { name: "France".into(), code: "FR".into() });
    }
}
