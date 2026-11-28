// Thin wrappers over the Rust commands. The UI has no direct file access.
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save, ask } from "@tauri-apps/plugin-dialog";

export const pickPicture = (title) =>
  open({ directory: false, multiple: false, title, filters: [{ name: "Pictures", extensions: ["png", "jpg", "jpeg", "webp", "tif", "tiff", "bmp", "gif"] }] });
export const pickMusic = (title) =>
  open({ directory: false, multiple: true, title, filters: [{ name: "Music", extensions: ["mp3", "m4a", "aac", "wav", "ogg", "oga", "opus", "flac"] }] });
export const pickFolder = (title) => open({ directory: true, multiple: false, title });
export const pickModelFile = (title, ext) =>
  open({ directory: false, multiple: false, title, filters: [{ name: ext.toUpperCase() + " model", extensions: [ext] }] });
export const pickSaveFile = (title, defaultPath) =>
  save({ title, defaultPath, filters: [{ name: "CSV (Excel)", extensions: ["csv"] }] });
export const confirmBox = (message, title = "ChronoSort", okLabel = "Yes", cancelLabel = "No") =>
  ask(message, { title, kind: "warning", okLabel, cancelLabel });

export const analyze = (source) => invoke("analyze", { source });
export const makePlan = (dest, months, naming, resolutions) =>
  invoke("make_plan", { dest, months, naming, resolutions });
export const run = (mode, threads = 4) => invoke("run", { mode, threads });
export const undo = (dest) => invoke("undo", { dest });
export const checkNaming = (naming) => invoke("check_naming", { naming });
export const findDuplicates = (roots, includeAll, similar = false) => invoke("find_duplicates", { roots, includeAll, similar });
export const resolveDuplicates = (keep, mode, target) => invoke("resolve_duplicates", { keep, mode, target });
export const saveDuplicateReport = (path) => invoke("save_duplicate_report", { path });
export const undoDuplicates = (target) => invoke("undo_duplicates", { target });
export const openFolder = (path) => invoke("open_folder", { path });
export const showDuplicate = (hash, index) => invoke("show_duplicate", { hash, index });
export const duplicateItems = (hash) => invoke("duplicate_items", { hash });
export const exitApp = () => invoke("exit_app");
export const aiOverview = () => invoke("ai_overview");
export const linkModelFile = (pack, role, path) => invoke("link_model_file", { pack, role, path });
export const unlinkModel = (pack) => invoke("unlink_model", { pack });
export const downloadPack = (pack) => invoke("download_pack", { pack });
export const cancelDownload = () => invoke("cancel_download");
export const deleteModelFiles = (pack) => invoke("delete_model_files", { pack });
export const platformInfo = () => invoke("platform_info");
export const startupArgs = () => invoke("startup_args");
export const onProgress = (event, cb) => listen(event, (e) => cb(e.payload));
export const thumbUrl = (hash) => convertFileSrc(hash, "thumb");
export const previewUrl = (hash) => convertFileSrc(hash, "preview");

export function formatBytes(b) {
  const u = ["bytes", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return i === 0 ? `${b} bytes` : `${b.toFixed(1)} ${u[i]}`;
}

export const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;

/** Last path component, for either slash style. */
export const baseName = (p) => p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;
/** Parent folder, keeping the original separator style. */
export const parentDir = (p) => {
  const t = p.replace(/[\\/]+$/, "");
  const i = Math.max(t.lastIndexOf("\\"), t.lastIndexOf("/"));
  return i > 0 ? t.slice(0, i) : t;
};
export const joinPath = (dir, name) => dir.replace(/[\\/]+$/, "") + (dir.includes("/") && !dir.includes("\\") ? "/" : "\\") + name;

export const SOURCE_LABELS = {
  exif_original: "EXIF", exif_digitized: "EXIF", exif_modify: "EXIF edit date",
  quick_time: "Video", xmp_sidecar: "XMP", takeout_json: "Takeout",
  file_name: "File name", folder_path: "Folder", fs_modified: "File date",
  fs_created: "File date", user: "You",
};

// People (faces). Everything stays on this PC.
export const findFaces = (root) => invoke("find_faces", { root });
export const suggestPerson = (faces) => invoke("suggest_person", { faces });
export const savePerson = (faces, name, personId = null) => invoke("save_person", { faces, name, personId });
export const listPeople = () => invoke("list_people");
export const renamePerson = (id, name) => invoke("rename_person", { id, name });
export const forgetPerson = (id) => invoke("forget_person", { id });
export const setBirthYear = (id, year) => invoke("set_birth_year", { id, year });
export const forgetEveryone = () => invoke("forget_everyone");
export const mergePeople = (keep, other) => invoke("merge_people", { keep, other });
export const saveReview = (resolutions, ai) => invoke("save_review", { resolutions, ai });
export const loadReview = () => invoke("load_review");
export const aiDate = (rels) => invoke("ai_date", { rels });
export const aiCancel = () => invoke("ai_cancel");
export const aiStop = () => invoke("ai_stop");
export const showFacePhoto = (face) => invoke("show_face_photo", { face });
export const faceUrl = (i) => convertFileSrc(String(i), "face");
export const personUrl = (id, v = 0) => convertFileSrc(`${id}_${v}`, "person");

// File browser. Lists names/sizes/dates only; thumbnails by listing id (never by path).
export const browseRoots = () => invoke("browse_roots");
export const listDir = (path, showHidden = false) => invoke("list_dir", { path, showHidden });
export const listSubdirs = (path, showHidden = false) => invoke("list_subdirs", { path, showHidden });
export const folderSamples = (id) => invoke("folder_samples", { id });
export const revealItem = (id) => invoke("reveal_item", { id });
/** {w, h, original} of the full picture (header only), or null if unknown. */
export const imageSize = (id) => invoke("image_size", { id });
/** size: "s" | "m" | "l" (thumbnails) or "p" (viewer preview). */
export const fthumbUrl = (id, size = "m") => convertFileSrc(`${id}_${size}`, "fthumb");

// Housekeeping (File browser). Nothing is overwritten; everything but a permanent delete can be undone.
export const binStatus = (path) => invoke("fs_bin_status", { path });
export const checkName = (name) => invoke("fs_check_name", { name });
export const newFolder = (dir, name) => invoke("fs_new_folder", { dir, name });
export const renameItem = (id, name) => invoke("fs_rename", { id, name });
/** mode: "copy" | "move"; progress arrives as "fileop-progress" events. */
export const transferItems = (ids, dest, mode) => invoke("fs_transfer", { ids, dest, mode });
export const deleteItems = (ids, permanent) => invoke("fs_delete", { ids, permanent });
export const cancelFileOp = () => invoke("fs_cancel");
export const lastUndo = () => invoke("fs_last_undo");
export const undoFileOp = () => invoke("fs_undo");

// Viewer edits (photocraft codecs) and the information panel (ExifTool). Saves never overwrite.
export const editOpen = (id) => invoke("edit_open", { id });
export const editApply = (edit) => invoke("edit_apply", { edit });
export const editStepBack = () => invoke("edit_step_back");
export const editClose = () => invoke("edit_close");
export const editAdjust = (adjust) => invoke("edit_adjust", { adjust });
export const editAuto = () => invoke("edit_auto");
export const editControls = () => invoke("edit_controls");
export const editSetDpi = (dpi) => invoke("edit_set_dpi", { dpi });
export const editRetouch = (op) => invoke("edit_retouch", { op });
export const editEffects = (effects) => invoke("edit_effects", { effects });
export const editWatermark = (png, path) => invoke("edit_watermark", { png, path });
/** mode "copy" | "replace"; format "JPEG" | "PNG" | "TIFF" | "WebP". */
export const editSave = (mode, format, quality, name = null) => invoke("edit_save", { mode, format, quality, name });
export const editFormats = () => invoke("edit_formats");
export const imageHistogram = (id, edit) => invoke("image_histogram", { id, edit });
export const setWallpaper = (id, style) => invoke("set_wallpaper", { id, style });
// Lossless JPEG crop to a new file; remove information ("all" | "place") from photos.
export const jpegLosslessCrop = (id, r) => invoke("jpeg_lossless_crop", { id, x: r.x, y: r.y, w: r.w, h: r.h });
export const stripMeta = (ids, what) => invoke("strip_meta", { ids, what });
// Thumbnail cache tools (progress: "cache-progress").
export const thumbCacheInfo = () => invoke("thumb_cache_info");
export const thumbCacheClear = () => invoke("thumb_cache_clear");
export const thumbCacheTrim = (maxMb) => invoke("thumb_cache_trim", { maxMb });
export const thumbCacheClearOld = () => invoke("thumb_cache_clear_old");
export const thumbCachePrescan = (path, recursive, sizes) => invoke("thumb_cache_prescan", { path, recursive, sizes });
export const thumbCacheCancel = () => invoke("thumb_cache_cancel");
// Create: contact sheet / joined picture, PDF, ZIP (progress: "create-progress").
export const createSizes = (ids) => invoke("create_sizes", { ids });
export const createSheet = (ids, opts) => invoke("create_sheet", { ids, opts });
export const createPdf = (ids, opts) => invoke("create_pdf", { ids, opts });
export const createZip = (ids, name) => invoke("create_zip", { ids, name });
// Batch convert / resize and rename (progress: "batch-progress").
export const batchConvert = (ids, opts) => invoke("batch_convert", { ids, opts });
export const batchRenamePreview = (ids, opts) => invoke("batch_rename_preview", { ids, opts });
export const batchRename = (ids, opts) => invoke("batch_rename", { ids, opts });
export const batchCancel = () => invoke("batch_cancel");
// Slideshow: videos and music stream from media://<id> (only ids the app handed out).
export const mediaUrl = (id) => convertFileSrc(String(id), "media");
export const slideshowMusic = (paths) => invoke("slideshow_music", { paths });
export const slideshowAwake = (on) => invoke("slideshow_awake", { on });
export const slideshowPlayable = (ids) => invoke("slideshow_playable", { ids });
export const editUrl = (version) => convertFileSrc(String(version), "edit");
export const metaRead = (id) => invoke("meta_read", { id });
export const metaWrite = (id, changes) => invoke("meta_write", { id, changes });

// Star ratings (written into the photo, with backup + Undo) and the tag mark (ChronoSort's own list).
export const readMarks = (ids) => invoke("marks", { ids });
export const setRating = (ids, stars) => invoke("set_rating", { ids, stars });
export const setTagged = (ids, on) => invoke("set_tagged", { ids, on });
export const starText = (n) => "★".repeat(n) + "☆".repeat(5 - n);
