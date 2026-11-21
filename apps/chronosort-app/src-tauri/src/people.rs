//! People: find faces in a folder, group them, and remember the people the
//! user names, so they are recognised again next time.
//!
//! Privacy: everything runs on this PC. The people library (`people.db` in the
//! app data folder) holds names, face fingerprints, years and small face crops,
//! never photo paths. Photos are only read.

use crate::{AppState, Progress, Res, err};
use cs_faces::folder::{FaceScan, LOAD_SIDE};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State};

/// Face crops shown in the app (and stored with a named person).
const CROP_SIDE: u32 = 128;

/// The last face search. Indices into `scan.faces` are the face ids the UI uses.
pub struct FaceSession {
    root: PathBuf,
    scan: FaceScan,
    /// Photo path (relative) -> year, when the photo's date is known.
    years: HashMap<String, i16>,
    /// face index -> JPEG crop, made on first use.
    crops: Mutex<HashMap<usize, Vec<u8>>>,
}

pub type Faces = Mutex<Option<Arc<FaceSession>>>;

pub(crate) fn people_db(app: &tauri::AppHandle) -> Res<cs_db::Db> {
    let dir = crate::data_dir(app)?;
    std::fs::create_dir_all(&dir).map_err(err)?;
    cs_db::Db::open(&dir.join("people.db")).map_err(err)
}

/// Face models: the linked/downloaded "faces-opencv" pack.
pub(crate) fn load_models(app: &tauri::AppHandle) -> Res<cs_faces::FaceModels> {
    let linked = cs_ai::Linked::load(&crate::data_dir(app)?.join("models.json")).map_err(err)?;
    let role = |r: &str, name: &str| {
        linked
            .packs
            .get("faces-opencv")
            .and_then(|m| m.get(r))
            .map(|f| f.path.clone())
            .or_else(|| cs_ai::pack_dir("faces-opencv").map(|d| d.join(name)))
            .filter(|p| p.is_file())
    };
    let (Some(det), Some(rec)) = (
        role("detector", "face_detection_yunet_2023mar.onnx"),
        role("recogniser", "face_recognition_sface_2021dec.onnx"),
    ) else {
        return Err("Face recognition is not set up yet. Open Settings → AI models and get “Faces” (a small download, about 40 MB).".into());
    };
    cs_faces::FaceModels::load(&det, &rec).map_err(err)
}

#[derive(Serialize)]
pub struct FaceOut {
    rel: String,
    faces_in_photo: usize,
    year: Option<i16>,
}

#[derive(Serialize)]
pub struct Suggestion {
    person_id: i64,
    name: String,
    /// 0..1, how alike (cosine similarity).
    similarity: f32,
}

#[derive(Serialize)]
pub struct GroupOut {
    faces: Vec<usize>,
    suggestion: Option<Suggestion>,
}

#[derive(Serialize)]
pub struct FacesOut {
    root: String,
    photos: usize,
    photos_with_faces: usize,
    faces: Vec<FaceOut>,
    groups: Vec<GroupOut>,
    errors: Vec<String>,
}

#[derive(Serialize)]
pub struct PersonOut {
    id: i64,
    name: String,
    samples: usize,
    years: Vec<i16>,
    birth_year: Option<i16>,
}

fn people_out(db: &cs_db::Db) -> Res<Vec<PersonOut>> {
    Ok(db
        .people()
        .map_err(err)?
        .into_iter()
        .map(|p| PersonOut { id: p.id, name: p.name, samples: p.samples, years: p.years, birth_year: p.birth_year })
        .collect())
}

/// Best known person for some faces (their average fingerprint).
fn suggest(known: &[cs_db::KnownPerson], embs: &[&[f32]]) -> Option<Suggestion> {
    if known.is_empty() || embs.is_empty() {
        return None;
    }
    let c = cs_faces::centroid(embs);
    let samples: Vec<Vec<Vec<f32>>> = known.iter().map(|k| k.2.clone()).collect();
    cs_faces::identify(&c, &samples).map(|(i, s)| Suggestion {
        person_id: known[i].0,
        name: known[i].1.clone(),
        similarity: s,
    })
}

/// Find and group the faces in a folder (read-only).
#[tauri::command]
pub async fn find_faces(app: tauri::AppHandle, state: State<'_, AppState>, root: String) -> Res<FacesOut> {
    let root = PathBuf::from(root.trim());
    if !root.is_dir() {
        return Err("That folder cannot be found.".into());
    }
    // Years of the photos, if this folder was just analysed for sorting.
    let years: HashMap<String, i16> = {
        let s = state.session.lock().map_err(err)?;
        if s.source.as_deref() == Some(root.as_path()) {
            s.items
                .iter()
                .filter_map(|i| match i.decision {
                    cs_model::Decision::Auto { year, .. } => Some((i.rel_path.clone(), year)),
                    _ => None,
                })
                .collect()
        } else {
            HashMap::new()
        }
    };
    let app2 = app.clone();
    let root2 = root.clone();
    let scan = tauri::async_runtime::spawn_blocking(move || -> Res<FaceScan> {
        let models = load_models(&app2)?;
        let emit = |done, total| {
            let _ = app2.emit("faces-progress", Progress { done, total, bytes: 0 });
        };
        cs_faces::folder::scan_folder(&root2, &models, &emit).map_err(err)
    })
    .await
    .map_err(err)??;

    let known = people_db(&app)?.all_samples().map_err(err)?;
    let out = FacesOut {
        root: root.display().to_string(),
        photos: scan.photos,
        photos_with_faces: scan.photos_with_faces,
        faces: scan
            .faces
            .iter()
            .map(|f| FaceOut { rel: f.rel.clone(), faces_in_photo: f.faces_in_photo, year: years.get(&f.rel).copied() })
            .collect(),
        groups: scan
            .groups
            .iter()
            .map(|g| GroupOut {
                faces: g.clone(),
                suggestion: suggest(&known, &g.iter().map(|&i| scan.faces[i].embedding.as_slice()).collect::<Vec<_>>()),
            })
            .collect(),
        errors: scan.errors.iter().map(|(p, e)| format!("{p}: {e}")).collect(),
    };
    *state.faces.lock().map_err(err)? =
        Some(Arc::new(FaceSession { root, scan, years, crops: Mutex::new(HashMap::new()) }));
    Ok(out)
}

/// Which known person these faces look like (after the user merged or split groups).
#[tauri::command]
pub fn suggest_person(app: tauri::AppHandle, state: State<'_, AppState>, faces: Vec<usize>) -> Res<Option<Suggestion>> {
    let fs = state.faces.lock().map_err(err)?.clone().ok_or("find faces first")?;
    let embs: Vec<&[f32]> =
        faces.iter().filter_map(|&i| fs.scan.faces.get(i)).map(|f| f.embedding.as_slice()).collect();
    Ok(suggest(&people_db(&app)?.all_samples().map_err(err)?, &embs))
}

/// JPEG crop of one found face (cached for the session).
fn crop_jpeg(fs: &FaceSession, i: usize) -> Res<Vec<u8>> {
    if let Some(b) = fs.crops.lock().map_err(err)?.get(&i) {
        return Ok(b.clone());
    }
    let f = fs.scan.faces.get(i).ok_or("unknown face")?;
    let abs = cs_exec::join_rel(&fs.root, &f.rel).map_err(err)?;
    let img = cs_thumb::load(&abs, cs_scan::Format::from_name(&f.format), LOAD_SIDE).map_err(err)?.to_rgb8();
    let crop = cs_faces::face_crop(&img, &f.face, CROP_SIDE);
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85).encode_image(&crop).map_err(err)?;
    fs.crops.lock().map_err(err)?.insert(i, jpeg.clone());
    Ok(jpeg)
}

/// Remember these faces as a person: a new one (`name`) or someone already
/// known (`person_id`, or a known `name`).
#[tauri::command]
pub async fn save_person(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: Option<String>,
    person_id: Option<i64>,
    faces: Vec<usize>,
) -> Res<Vec<PersonOut>> {
    let fs = state.faces.lock().map_err(err)?.clone().ok_or("find faces first")?;
    if faces.is_empty() {
        return Err("No faces chosen.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let db = people_db(&app)?;
        let id = match (person_id, name) {
            (Some(id), _) => {
                db.person_name(id).map_err(err)?.ok_or("That person was forgotten in the meantime.")?;
                id
            }
            (None, Some(n)) => {
                let n = cs_db::clean_name(&n)?;
                match db.person_by_name(&n).map_err(err)? {
                    Some(id) => id,
                    None => db.add_person(&n).map_err(err)?,
                }
            }
            (None, None) => return Err("Please type a name.".to_string()),
        };
        let samples: Vec<cs_db::Sample> = faces
            .iter()
            .filter_map(|&i| fs.scan.faces.get(i).map(|f| (i, f)))
            .map(|(i, f)| cs_db::Sample {
                embedding: f.embedding.clone(),
                year: fs.years.get(&f.rel).copied(),
                crop: crop_jpeg(&fs, i).ok(),
            })
            .collect();
        db.add_samples(id, &samples).map_err(err)?;
        people_out(&db)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
pub fn list_people(app: tauri::AppHandle) -> Res<Vec<PersonOut>> {
    people_out(&people_db(&app)?)
}

/// Set (or clear with None) a person's birth year.
#[tauri::command]
pub fn set_birth_year(app: tauri::AppHandle, id: i64, year: Option<i16>) -> Res<Vec<PersonOut>> {
    let this_year = cs_model::civil::from_unix(crate::now_unix()).0 as i16;
    if let Some(y) = year
        && !(1850..=this_year).contains(&y)
    {
        return Err(format!("Please type a birth year between 1850 and {this_year}."));
    }
    let db = people_db(&app)?;
    db.set_birth_year(id, year).map_err(err)?;
    people_out(&db)
}

#[tauri::command]
pub fn rename_person(app: tauri::AppHandle, id: i64, name: String) -> Res<Vec<PersonOut>> {
    let db = people_db(&app)?;
    let n = cs_db::clean_name(&name)?;
    db.rename_person(id, &n).map_err(err)?;
    people_out(&db)
}

/// Forget a person: their name and every stored face are deleted. Photos are not touched.
#[tauri::command]
pub fn forget_person(app: tauri::AppHandle, id: i64) -> Res<Vec<PersonOut>> {
    let db = people_db(&app)?;
    db.delete_person(id).map_err(err)?;
    people_out(&db)
}

/// Forget everyone: the people file itself is deleted (also its journal files),
/// so no names or face fingerprints remain on disk. Photos are not touched.
#[tauri::command]
pub fn forget_everyone(app: tauri::AppHandle) -> Res<Vec<PersonOut>> {
    let dir = crate::data_dir(&app)?;
    for name in ["people.db", "people.db-wal", "people.db-shm"] {
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(vec![])
}

#[tauri::command]
pub fn merge_people(app: tauri::AppHandle, keep: i64, other: i64) -> Res<Vec<PersonOut>> {
    let db = people_db(&app)?;
    db.merge_people(keep, other).map_err(err)?;
    people_out(&db)
}

/// Show the photo a face was found in, selected in Explorer.
#[tauri::command]
pub fn show_face_photo(state: State<'_, AppState>, face: usize) -> Res<()> {
    let fs = state.faces.lock().map_err(err)?.clone().ok_or("find faces first")?;
    let f = fs.scan.faces.get(face).ok_or("unknown face")?;
    cs_platform::show_file_in_file_manager(&cs_exec::join_rel(&fs.root, &f.rel).map_err(err)?).map_err(err)
}

fn jpeg_response(r: Res<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    let b = tauri::http::Response::builder().header("Cache-Control", "no-store");
    match r {
        Ok(bytes) => b.header("Content-Type", "image/jpeg").status(200).body(bytes),
        Err(e) => b.header("Content-Type", "text/plain").status(404).body(e.into_bytes()),
    }
    .expect("static response")
}

/// `face://<n>`: crop of face n of the current search (nothing else can be read).
pub fn face_response(app: &tauri::AppHandle, path: &str) -> tauri::http::Response<Vec<u8>> {
    let r = (|| {
        let i: usize = path.trim_matches('/').parse().map_err(err)?;
        let fs = app.state::<AppState>().faces.lock().map_err(err)?.clone().ok_or("no face search")?;
        crop_jpeg(&fs, i)
    })();
    jpeg_response(r)
}

/// `person://<id>_<version>`: the stored face crop of a named person (the version only
/// changes the URL so a new picture is not taken from the cache).
pub fn person_response(app: &tauri::AppHandle, path: &str) -> tauri::http::Response<Vec<u8>> {
    let r = (|| {
        let id: i64 = path.trim_matches('/').split('_').next().unwrap_or("").parse().map_err(err)?;
        people_db(app)?.person_cover(id).map_err(err)?.ok_or_else(|| "no picture".to_string())
    })();
    jpeg_response(r)
}
