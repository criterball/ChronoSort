//! Compare an analysis against the corpus manifest.

use crate::{Manifest, Route};
use cs_model::{Decision, ItemReport};
use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct EvalReport {
    pub checked: usize,
    pub passed: usize,
    /// (path, expected, got)
    pub failures: Vec<(String, String, String)>,
}

impl EvalReport {
    pub fn ok(&self) -> bool {
        self.failures.is_empty() && self.checked > 0
    }
}

fn describe(route: Route, year: Option<i16>) -> String {
    match (route, year) {
        (Route::Skip, _) => "skip".into(),
        (r, Some(y)) => format!("{r:?} {y}").to_lowercase(),
        (r, None) => format!("{r:?} (no year)").to_lowercase(),
    }
}

fn actual(d: &Decision) -> (Route, Option<i16>) {
    match d {
        Decision::Auto { year, .. } => (Route::Auto, Some(*year)),
        Decision::Review { suggested, .. } => (Route::Review, *suggested),
        Decision::Skip { .. } => (Route::Skip, None),
    }
}

/// Every manifest entry must be present with the expected route and year,
/// duplicates must be detected, and no unexpected files may appear.
pub fn evaluate(m: &Manifest, items: &[ItemReport]) -> EvalReport {
    let by_path: HashMap<&str, &ItemReport> = items.iter().map(|i| (i.rel_path.as_str(), i)).collect();
    let mut r = EvalReport::default();
    for e in &m.entries {
        r.checked += 1;
        let want = describe(e.route, if e.route == Route::Skip { None } else { e.year });
        let Some(item) = by_path.get(e.path.as_str()) else {
            r.failures.push((e.path.clone(), want, "MISSING from scan".into()));
            continue;
        };
        let (route, year) = actual(&item.decision);
        let mut got = describe(route, year);
        let mut ok = route == e.route && (route == Route::Skip || year == e.year);
        if item.duplicate_of != e.duplicate_of {
            ok = false;
            got = format!("{got}; duplicate_of={:?} (expected {:?})", item.duplicate_of, e.duplicate_of);
        }
        if ok {
            r.passed += 1;
        } else {
            r.failures.push((e.path.clone(), want, got));
        }
    }
    let known: std::collections::HashSet<&str> = m.entries.iter().map(|e| e.path.as_str()).collect();
    for i in items.iter().filter(|i| !known.contains(i.rel_path.as_str())) {
        r.failures.push((i.rel_path.clone(), "not in manifest".into(), "unexpected file".into()));
    }
    r
}
