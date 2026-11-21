//! "Ask the AI": the local vision model estimates the year of uncertain photos.
//!
//! The engine (llama-server, see cs-vision) is started on first use, listens on
//! 127.0.0.1 only and runs offline; it is stopped by `ai_stop` and on exit.
//! Its answer is only a suggestion shown next to the photo: the user decides.

use crate::{AppState, Res, err};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager, State};

/// Longest side of the picture the model gets (more costs time, not accuracy).
const MODEL_SIDE: u32 = 896;

#[derive(Default)]
pub struct Vision {
    engine: Mutex<Option<(String, cs_vision::Engine)>>,
    cancel: AtomicBool,
}

impl Vision {
    pub fn stop(&self) {
        if let Ok(mut e) = self.engine.lock() {
            *e = None; // Drop kills the server
        }
    }
}

#[derive(Serialize, Clone)]
struct AiDateEvent {
    rel: String,
    guess: Option<cs_vision::DateGuess>,
    error: Option<String>,
    /// Named people with a birth year recognised in the photo: the photo
    /// cannot be older than the youngest of them.
    people: Vec<PresentPerson>,
    done: usize,
    total: usize,
}

#[derive(Serialize, Clone)]
struct PresentPerson {
    name: String,
    born: i16,
}

/// A recognition this alike is trusted for the "not before" rule (the People
/// screen's confident level; weaker matches were seen to be wrong).
const SURE: f32 = 0.5;

/// Named people with a birth year, and the face models to find them.
struct BirthCheck {
    models: cs_faces::FaceModels,
    names: Vec<(String, i16)>,
    samples: Vec<Vec<Vec<f32>>>,
}

fn birth_check(app: &tauri::AppHandle) -> Option<BirthCheck> {
    let db = crate::people::people_db(app).ok()?;
    let born: std::collections::HashMap<i64, i16> =
        db.people().ok()?.into_iter().filter_map(|p| p.birth_year.map(|y| (p.id, y))).collect();
    if born.is_empty() {
        return None;
    }
    let (mut names, mut samples) = (vec![], vec![]);
    for (id, name, s) in db.all_samples().ok()? {
        if let Some(&y) = born.get(&id) {
            names.push((name, y));
            samples.push(s);
        }
    }
    let models = crate::people::load_models(app).ok()?;
    (!names.is_empty()).then_some(BirthCheck { models, names, samples })
}

impl BirthCheck {
    fn present(&self, img: &image::RgbImage) -> Vec<PresentPerson> {
        let Ok(faces) = self.models.detect(img, cs_faces::MIN_SCORE) else { return vec![] };
        let mut out: Vec<PresentPerson> = vec![];
        for f in faces.iter().filter(|f| f.bbox[2].min(f.bbox[3]) >= cs_faces::folder::MIN_FACE_PX) {
            let Ok(e) = self.models.embed(img, f) else { continue };
            if let Some((i, s)) = cs_faces::identify(&e, &self.samples)
                && s >= SURE
                && !out.iter().any(|p| p.name == self.names[i].0)
            {
                out.push(PresentPerson { name: self.names[i].0.clone(), born: self.names[i].1 });
            }
        }
        out
    }
}

/// The vision pack and engine to use: the recommended ready pack for this PC,
/// else any ready one; the GPU engine when there is a usable graphics card.
fn pick(app: &tauri::AppHandle) -> Res<(cs_vision::EngineFilesOwned, bool, String)> {
    let cat = cs_ai::catalogue();
    let linked = cs_ai::Linked::load(&crate::data_dir(app)?.join("models.json")).map_err(err)?;
    let hw = cs_ai::Hardware::detect();
    let tier = cs_ai::assess(&hw).tier;
    let ready =
        |id: &str| cat.packs.iter().find(|p| p.id == id).is_some_and(|p| linked.status(p) == cs_ai::PackStatus::Ready);
    let rec = cs_ai::recommend(&cat, tier, &ready);
    let vision = rec
        .get("vision")
        .filter(|id| ready(id))
        .cloned()
        .or_else(|| {
            cat.packs.iter().filter(|p| p.purpose == "vision" && ready(&p.id)).map(|p| p.id.clone()).next_back()
        })
        .ok_or(
            "No AI model for pictures is set up yet. Open Settings → AI models and get one of the “Vision” models.",
        )?;
    let path = |pack: &str, role: &str| linked.packs.get(pack).and_then(|m| m.get(role)).map(|f| f.path.clone());
    let gpu = cs_ai::fit(cat.packs.iter().find(|p| p.id == vision).ok_or("unknown model")?, &hw) == cs_ai::Fit::Gpu;
    let engines = if gpu { ["llama-cpp-vulkan", "llama-cpp-cpu"] } else { ["llama-cpp-cpu", "llama-cpp-vulkan"] };
    let (engine, server) = engines
        .iter()
        .find_map(|e| ready(e).then(|| path(e, "archive")).flatten().map(|p| (*e, p)))
        .ok_or("The AI engine (llama.cpp) is not set up yet. Open Settings → AI models and get the “Engine”.")?;
    let files = cs_vision::EngineFilesOwned {
        server,
        model: path(&vision, "model").ok_or("model file missing")?,
        projector: path(&vision, "projector").ok_or("projector file missing")?,
        work_dir: crate::data_dir(app)?.join("logs"),
    };
    Ok((files, gpu && engine == "llama-cpp-vulkan", vision))
}

/// Estimate the year of these photos (relative paths of the current analysis).
/// Results arrive one by one as `ai-date` events; returns how many were done.
#[tauri::command]
pub async fn ai_date(app: tauri::AppHandle, state: State<'_, AppState>, rels: Vec<String>) -> Res<usize> {
    let (root, formats) = {
        let s = state.session.lock().map_err(err)?;
        let root = s.source.clone().ok_or("analyse a folder first")?;
        let formats: std::collections::HashMap<String, String> =
            s.items.iter().map(|i| (i.rel_path.clone(), i.format.clone())).collect();
        (root, formats)
    };
    let jobs: Vec<(String, String)> =
        rels.into_iter().filter_map(|r| formats.get(&r).map(|f| (r.clone(), f.clone()))).collect();
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || -> Res<usize> {
        let st = app2.state::<AppState>();
        let v = &st.vision;
        v.cancel.store(false, Ordering::Relaxed);
        let mut guard = v.engine.lock().map_err(err)?;
        if guard.is_none() {
            let (files, gpu, pack) = pick(&app2)?;
            let _ = app2.emit("ai-status", "starting");
            let eng =
                cs_vision::Engine::start(&files.as_ref(), gpu, std::time::Duration::from_secs(300)).map_err(err)?;
            *guard = Some((pack, eng));
        }
        let (_, eng) = guard.as_ref().ok_or("engine missing")?;
        let births = birth_check(&app2);
        let _ = app2.emit("ai-status", "running");
        let total = jobs.len();
        let mut done = 0;
        for (rel, format) in jobs {
            if v.cancel.load(Ordering::Relaxed) {
                break;
            }
            let mut people = vec![];
            let r = (|| -> Res<cs_vision::DateGuess> {
                let abs: PathBuf = cs_exec::join_rel(&root, &rel).map_err(err)?;
                let fmt = cs_scan::Format::from_name(&format);
                let full = cs_thumb::load(&abs, fmt, cs_faces::folder::LOAD_SIDE).map_err(err)?.to_rgb8();
                if let Some(b) = &births {
                    people = b.present(&full);
                }
                let img = if full.width().max(full.height()) > MODEL_SIDE {
                    image::DynamicImage::ImageRgb8(full).thumbnail(MODEL_SIDE, MODEL_SIDE).to_rgb8()
                } else {
                    full
                };
                let mut jpeg = Vec::new();
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 88).encode_image(&img).map_err(err)?;
                let mut g = eng.date_photo(&jpeg).map_err(err)?;
                // Hard rule: not before the youngest recognised person was born.
                if let Some(min) = people.iter().map(|p| i32::from(p.born)).max()
                    && g.best_year < min
                {
                    g.best_year = min;
                    g.earliest_year = g.earliest_year.max(min);
                    g.latest_year = g.latest_year.max(min);
                }
                Ok(g)
            })();
            done += 1;
            let (guess, error) = match r {
                Ok(g) => (Some(g), None),
                Err(e) => (None, Some(e)),
            };
            let _ = app2.emit("ai-date", AiDateEvent { rel, guess, error, people, done, total });
        }
        Ok(done)
    })
    .await
    .map_err(err)?
}

/// Stop after the current photo.
#[tauri::command]
pub fn ai_cancel(state: State<'_, AppState>) {
    state.vision.cancel.store(true, Ordering::Relaxed);
}

/// Stop the engine and free the graphics memory (waits for the current photo).
#[tauri::command]
pub async fn ai_stop(app: tauri::AppHandle) -> Res<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let st = app.state::<AppState>();
        st.vision.cancel.store(true, Ordering::Relaxed);
        st.vision.stop();
    })
    .await
    .map_err(err)
}
