// Thin wrappers over the Rust commands. The UI has no direct file access.
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save, ask } from "@tauri-apps/plugin-dialog";

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
export const findDuplicates = (roots, includeAll) => invoke("find_duplicates", { roots, includeAll });
export const resolveDuplicates = (keep, mode, target) => invoke("resolve_duplicates", { keep, mode, target });
export const saveDuplicateReport = (path) => invoke("save_duplicate_report", { path });
export const undoDuplicates = (target) => invoke("undo_duplicates", { target });
export const openFolder = (path) => invoke("open_folder", { path });
export const showDuplicate = (hash, index) => invoke("show_duplicate", { hash, index });
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
/** mode "copy" | "replace"; format "JPEG" | "PNG" | "TIFF" | "WebP". */
export const editSave = (mode, format, quality, name = null) => invoke("edit_save", { mode, format, quality, name });
export const editFormats = () => invoke("edit_formats");
export const editUrl = (version) => convertFileSrc(String(version), "edit");
export const metaRead = (id) => invoke("meta_read", { id });
export const metaWrite = (id, changes) => invoke("meta_write", { id, changes });
