//! Remember the user's Check decisions (and the AI's estimates) per source
//! folder, so a long review survives closing ChronoSort.
//!
//! Stored in `<data>/reviews/<key>.json`. Every year is saved with the file's
//! content hash and only restored if the file still has that content: a
//! decision never jumps to a different photo that took the same name.

use crate::{AppState, Res, data_dir, err};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use tauri::State;

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    source: String,
    saved_at: i64,
    /// rel path -> (year, content hash)
    years: BTreeMap<String, (i16, Option<String>)>,
    /// rel path -> (AI estimate as the UI shows it, content hash)
    #[serde(default)]
    ai: BTreeMap<String, (serde_json::Value, Option<String>)>,
}

fn file_for(app: &tauri::AppHandle, source: &std::path::Path) -> Res<PathBuf> {
    let key = source.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_lowercase();
    let h = blake3::hash(key.as_bytes()).to_hex();
    Ok(data_dir(app)?.join("reviews").join(format!("{}.json", &h[..24])))
}

#[tauri::command]
pub fn save_review(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    resolutions: HashMap<String, i16>,
    ai: HashMap<String, serde_json::Value>,
) -> Res<()> {
    let s = state.session.lock().map_err(err)?;
    let source = s.source.clone().ok_or("analyse a folder first")?;
    let hash: HashMap<&str, &Option<String>> = s.items.iter().map(|i| (i.rel_path.as_str(), &i.hash)).collect();
    let saved = Saved {
        source: source.display().to_string(),
        saved_at: crate::now_unix(),
        years: resolutions
            .into_iter()
            .filter_map(|(r, y)| hash.get(r.as_str()).map(|h| (r, (y, (*h).clone()))))
            .collect(),
        ai: ai.into_iter().filter_map(|(r, v)| hash.get(r.as_str()).map(|h| (r, (v, (*h).clone())))).collect(),
    };
    drop(s);
    let f = file_for(&app, &source)?;
    if saved.years.is_empty() && saved.ai.is_empty() {
        let _ = std::fs::remove_file(&f);
        return Ok(());
    }
    std::fs::create_dir_all(f.parent().ok_or("no folder")?).map_err(err)?;
    let tmp = f.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&saved).map_err(err)?).map_err(err)?;
    std::fs::rename(&tmp, &f).map_err(err)
}

#[derive(Serialize, Default)]
pub struct Restored {
    resolutions: HashMap<String, i16>,
    ai: HashMap<String, serde_json::Value>,
    /// Saved decisions that no longer fit (file gone or changed).
    dropped: usize,
    saved_at: Option<i64>,
}

/// Earlier decisions for the folder that was just analysed.
#[tauri::command]
pub fn load_review(app: tauri::AppHandle, state: State<'_, AppState>) -> Res<Restored> {
    let s = state.session.lock().map_err(err)?;
    let source = s.source.clone().ok_or("analyse a folder first")?;
    let Ok(bytes) = std::fs::read(file_for(&app, &source)?) else { return Ok(Restored::default()) };
    let saved: Saved = serde_json::from_slice(&bytes).map_err(err)?;
    let hash: HashMap<&str, &Option<String>> = s.items.iter().map(|i| (i.rel_path.as_str(), &i.hash)).collect();
    let fits = |r: &str, h: &Option<String>| hash.get(r).is_some_and(|now| *now == h && h.is_some());
    let mut out = Restored { saved_at: Some(saved.saved_at), ..Default::default() };
    for (r, (y, h)) in saved.years {
        if fits(&r, &h) {
            out.resolutions.insert(r, y);
        } else {
            out.dropped += 1;
        }
    }
    for (r, (v, h)) in saved.ai {
        if fits(&r, &h) {
            out.ai.insert(r, v);
        }
    }
    Ok(out)
}
