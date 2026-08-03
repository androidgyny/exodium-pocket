//! eXoWin9x support-file pipeline.
//!
//! Win9x games boot Windows 95/98 from VHD images: the shared parent OS
//! images and the emulators that read them (eXo's DOSBox-X "x98" build,
//! 86Box + ROMs) ship in the torrent's `eXo/util/utilWin9x.zip` (2.5 GB),
//! nested inside its `EXTWin9x.zip` (2.47 GB) - the same matryoshka shape as
//! eXoDOS's util.zip. The payload is required on EVERY platform (the parent
//! VHDs are data, not binaries), so unlike the ECE build this is not
//! Windows-gated. `emulators/PCBox/` (Windows-only fork, unsupported) and
//! `emulators/audio/` (foobar2000) are never extracted.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::State;

use super::TorrentState;

static WIN9X_EXTRACTION_RUNNING: AtomicBool = AtomicBool::new(false);

/// The subtrees extracted from EXTWin9x.zip into `<torrent_root>/eXo/`.
/// `emulators/dosbox/` holds the x98 tree (parent VHDs, differencing
/// children, base conf) plus options9x.conf/config9x.bat at its root;
/// `emulators/86Box98/` holds 86Box, its ROMs and its parents.
const EXTRACT_PREFIXES: [&str; 2] = ["emulators/dosbox/", "emulators/86box98/"];

/// Support files a game of the given dosbox_variant needs before launch.
/// x98 (DOSBox-X) games read the x98 tree; every 86Box flavor reads 86Box98.
pub(crate) fn win9x_support_ready(torrent_root: &Path, variant: Option<&str>) -> bool {
    let x98_ready = torrent_root.join("eXo/emulators/dosbox/x98/parent").exists();
    match variant {
        Some(v) if v.starts_with("86box") => {
            torrent_root.join("eXo/emulators/86Box98/parent").exists()
        }
        _ => x98_ready,
    }
}

/// Extract the Win9x support payload from utilWin9x.zip (blocking).
fn extract_win9x_support(util_zip: &Path, torrent_root: &Path) -> Result<usize, String> {
    if WIN9X_EXTRACTION_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("extraction already running".to_string());
    }
    let result = do_extract_win9x_support(util_zip, torrent_root);
    WIN9X_EXTRACTION_RUNNING.store(false, Ordering::SeqCst);
    result
}

fn do_extract_win9x_support(util_zip: &Path, torrent_root: &Path) -> Result<usize, String> {
    // Unique temp names so a leftover from a killed run can't collide.
    let pid = std::process::id();
    let tmp_path = util_zip.with_extension(format!("extwin9x_tmp_{pid}"));
    let staging_root = torrent_root.join("eXo").join(format!(".win9x_staging_{pid}"));

    let result = (|| {
        let file = std::fs::File::open(util_zip).map_err(|e| e.to_string())?;
        let mut outer = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        {
            let mut inner_entry = outer
                .by_name("EXTWin9x.zip")
                .map_err(|e| format!("EXTWin9x.zip not found inside utilWin9x.zip: {}", e))?;
            let mut tmp = std::fs::File::create(&tmp_path).map_err(|e| e.to_string())?;
            std::io::copy(&mut inner_entry, &mut tmp).map_err(|e| e.to_string())?;
        }

        let tmp = std::fs::File::open(&tmp_path).map_err(|e| e.to_string())?;
        let mut inner = zip::ZipArchive::new(tmp).map_err(|e| e.to_string())?;
        let mut extracted = 0usize;
        for i in 0..inner.len() {
            let mut entry = inner.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().replace('\\', "/");
            let lower = name.to_ascii_lowercase();
            if !EXTRACT_PREFIXES.iter().any(|p| lower.starts_with(p))
                || name.contains("..")
                || entry.is_dir()
            {
                continue;
            }
            let out_path = staging_root.join(&name);
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            extracted += 1;
        }
        if extracted == 0 {
            return Err("no emulator entries found in EXTWin9x.zip".to_string());
        }

        // Move each fully-staged subtree into place with atomic renames -
        // the readiness gates test directory EXISTENCE, so a half-written
        // parent-VHD tree from a mid-extraction kill must never land.
        let dest_root = torrent_root.join("eXo");
        let staged_emulators = staging_root.join("emulators");
        let entries = std::fs::read_dir(&staged_emulators).map_err(|e| e.to_string())?;
        for entry in entries.filter_map(|e| e.ok()) {
            let rel = PathBuf::from("emulators").join(entry.file_name());
            let dst = dest_root.join(&rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            if dst.exists() {
                std::fs::remove_dir_all(&dst).map_err(|e| e.to_string())?;
            }
            std::fs::rename(entry.path(), &dst)
                .map_err(|e| format!("moving {} into place: {}", rel.display(), e))?;
        }
        Ok(extracted)
    })();

    let _ = std::fs::remove_file(&tmp_path);
    let _ = std::fs::remove_dir_all(&staging_root);
    result
}

/// Watch utilWin9x.zip until it finishes downloading, then extract the
/// support payload. Own task for the same reason as the MT-32 watcher: the
/// frontend only polls while a game download is active, and the 2.5 GB util
/// zip routinely finishes long after the game that triggered it.
pub(crate) fn spawn_win9x_support_watcher(
    mgr: std::sync::Arc<crate::torrent::manager::DownloadManager>,
    util_index: usize,
) {
    tauri::async_runtime::spawn(async move {
        let torrent_root = mgr.torrent_root();
        let expected_size = mgr.index().files.get(util_index).map(|f| f.size).unwrap_or(0);
        let mut failures = 0u32;
        // Generous ceiling: 6 h at 10 s per check for slow swarms.
        for _ in 0..2160 {
            if win9x_support_ready(&torrent_root, None)
                && win9x_support_ready(&torrent_root, Some("86box"))
            {
                return; // someone else finished the job
            }
            let Some(zip_path) = mgr.file_output_path(util_index) else {
                return;
            };
            // Stats-based completion PLUS a disk-size fallback (librqbit's
            // per-file stat can stall short of total; after a restart the
            // handle may be gone entirely) - see the MT-32 watcher.
            let stats_complete = mgr.is_file_complete(util_index).await;
            let disk_complete = expected_size > 0
                && std::fs::metadata(&zip_path).is_ok_and(|m| m.len() >= expected_size);
            if stats_complete || disk_complete {
                let root = torrent_root.clone();
                let zp = zip_path.clone();
                let outcome = tauri::async_runtime::spawn_blocking(move || {
                    extract_win9x_support(&zp, &root)
                })
                .await;
                match outcome {
                    Ok(Ok(n)) => {
                        log::info!("Extracted {} Win9x support files from utilWin9x.zip", n);
                        return;
                    }
                    Ok(Err(e)) if e == "extraction already running" => return,
                    Ok(Err(e)) => {
                        failures += 1;
                        log::error!(
                            "Failed to extract Win9x support files (attempt {}): {}",
                            failures, e
                        );
                        if failures >= 3 {
                            return;
                        }
                    }
                    Err(e) => {
                        log::error!("Win9x extraction task panicked: {}", e);
                        return;
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
    });
}

/// Queue utilWin9x.zip on the collection's manager (if not already selected)
/// and (re)arm the extraction watcher. Called from download_game when a
/// Win9x game is requested and the support tree is not on disk yet.
pub(crate) async fn ensure_win9x_support_queued(
    mgr: &std::sync::Arc<crate::torrent::manager::DownloadManager>,
) {
    let Some(util) = mgr.index().find_by_suffix("util/utilWin9x.zip") else {
        log::warn!("utilWin9x.zip not found in the eXoWin9x torrent index");
        return;
    };
    let util_index = util.index;
    if !mgr.is_file_selected(util_index).await {
        let _ = mgr.download_files(vec![util_index]).await;
        log::info!(
            "Also downloading utilWin9x.zip ({:.1} GB, one-time: Windows 9x OS images + emulators)",
            util.size as f64 / 1e9
        );
    }
    spawn_win9x_support_watcher(std::sync::Arc::clone(mgr), util_index);
}

/// Re-arm the extraction watcher after an app restart, so a utilWin9x.zip
/// that finishes downloading in a later session still gets extracted.
/// Called from init_download_manager once the eXoWin9x manager is hydrated.
pub(crate) async fn rearm_win9x_support(
    mgr: &std::sync::Arc<crate::torrent::manager::DownloadManager>,
) {
    let root = mgr.torrent_root();
    if win9x_support_ready(&root, None) && win9x_support_ready(&root, Some("86box")) {
        return;
    }
    let Some(util) = mgr.index().find_by_suffix("util/utilWin9x.zip") else {
        return;
    };
    let util_index = util.index;
    let selected = mgr.is_file_selected(util_index).await;
    let on_disk = mgr
        .file_output_path(util_index)
        .and_then(|p| std::fs::metadata(p).ok())
        .is_some_and(|m| m.len() > 0);
    if !selected && !on_disk {
        return; // support files were never requested - nothing to resume
    }
    log::info!(
        "Re-arming Win9x support extraction watcher (utilWin9x.zip {})",
        if selected { "still selected" } else { "present on disk" }
    );
    spawn_win9x_support_watcher(std::sync::Arc::clone(mgr), util_index);
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Win9xSupportStatus {
    /// "ready" | "downloading" | "missing"
    pub phase: String,
    /// Download progress 0..1 while phase == "downloading".
    pub progress: f32,
}

/// Support-file state for the detail panel: lets it show "Windows 9x support
/// files still downloading (N%)" instead of a bare launch failure.
#[tauri::command]
pub async fn get_win9x_support_status(
    torrent_state: State<'_, TorrentState>,
) -> Result<Win9xSupportStatus, String> {
    let mgr = {
        let guard = torrent_state.0.read().await;
        guard.get("eXoWin9x").cloned()
    };
    let Some(mgr) = mgr else {
        return Ok(Win9xSupportStatus { phase: "missing".into(), progress: 0.0 });
    };
    let root = mgr.torrent_root();
    if win9x_support_ready(&root, None) && win9x_support_ready(&root, Some("86box")) {
        return Ok(Win9xSupportStatus { phase: "ready".into(), progress: 1.0 });
    }
    let Some(util) = mgr.index().find_by_suffix("util/utilWin9x.zip") else {
        return Ok(Win9xSupportStatus { phase: "missing".into(), progress: 0.0 });
    };
    if mgr.is_file_selected(util.index).await {
        let on_disk = mgr
            .file_output_path(util.index)
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .unwrap_or(0);
        let progress = if util.size > 0 {
            (on_disk as f32 / util.size as f32).min(1.0)
        } else {
            0.0
        };
        return Ok(Win9xSupportStatus { phase: "downloading".into(), progress });
    }
    Ok(Win9xSupportStatus { phase: "missing".into(), progress: 0.0 })
}
