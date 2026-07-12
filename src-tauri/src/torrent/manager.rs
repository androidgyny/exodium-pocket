use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ManagedTorrent, Session, SessionOptions,
    SessionPersistenceConfig,
};
use serde::Serialize;
use tokio::sync::RwLock;

use walkdir::WalkDir;

use super::TorrentIndex;

/// Convert a path to its NT extended-length form on Windows (`\\?\C:\...` or
/// `\\?\UNC\server\share\...`). On other platforms this is a no-op.
///
/// The prefix tells the Win32 API to skip path normalization and the
/// MAX_PATH (260) check, allowing paths up to 32 767 characters. librqbit
/// passes the output folder verbatim to the file writer, so prefixing it
/// here is enough - every file it later opens inherits the long-path mode.
#[cfg(target_os = "windows")]
fn to_long_path(p: &Path) -> String {
    // \\?\ disables path normalization, so we must hand it backslash-only paths.
    // Tauri's dialog and PathBuf::join sometimes leave forward slashes from
    // user-provided strings; normalize before prefixing.
    let s = p.to_string_lossy().replace('/', r"\");
    if s.starts_with(r"\\?\") {
        return s;
    }
    if !p.is_absolute() {
        return s;
    }
    if let Some(rest) = s.strip_prefix(r"\\") {
        // UNC path: \\server\share\... -> \\?\UNC\server\share\...
        return format!(r"\\?\UNC\{}", rest);
    }
    format!(r"\\?\{}", s)
}

#[cfg(not(target_os = "windows"))]
fn to_long_path(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Remove 0-byte zip files in `root` that are NOT part of the current torrent
/// - i.e. true orphans from a previous run or unrelated user files.
///
/// `keep_paths` must contain the **full set** of the torrent's file paths
/// (forward-slashed, as produced by `TorrentIndex::from_file`), not just the
/// user's current selection. librqbit's `init()` creates a 0-byte placeholder
/// for **every** file declared by the torrent, regardless of `only_files`.
/// With fastresume's piece-cache (v0.6.4+), pieces shared between files get
/// marked "had" once any selected file's pieces arrive - and librqbit will
/// then refuse to re-download those pieces even if some of their target files
/// were deleted. Deleting a tracked placeholder therefore puts librqbit's
/// in-memory state at odds with disk: `file_progress` reports 100% complete
/// while `<file>.zip` is gone, and the user is stuck in an extraction loop
/// that never resolves (observed v0.6.6 with Dominium 762/762 bytes "100%"
/// but the zip never on disk).
///
/// To make the match work on Windows - where `WalkDir` yields backslash-
/// separated paths - we normalize each on-disk entry's string form to forward
/// slashes before comparing.
fn cleanup_placeholder_files(root: &Path, keep_paths: &[String]) -> std::io::Result<()> {
    let mut removed = 0;
    let mut kept = 0;
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let meta = match path.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.len() != 0 {
            continue;
        }
        if path.extension().map(|e| e != "zip").unwrap_or(true) {
            continue;
        }
        // Forward-slash form of the absolute path on disk. `keep_paths`
        // entries are torrent-relative ("eXoDOS/Content/.../Foo.zip"), so a
        // suffix match is enough - and it is slash-direction-agnostic now.
        let path_fwd = path.to_string_lossy().replace('\\', "/");
        let in_torrent = keep_paths.iter().any(|sp| path_fwd.ends_with(sp));
        if in_torrent {
            kept += 1;
            log::debug!("Cleanup: keeping librqbit-tracked placeholder {}", path.display());
            continue;
        }
        log::info!("Cleanup: deleting orphan 0-byte placeholder {}", path.display());
        let _ = std::fs::remove_file(path);
        removed += 1;
    }
    // Remove empty directories left behind
    for entry in WalkDir::new(root).contents_first(true).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() && path != root {
            let _ = std::fs::remove_dir(path);
        }
    }
    log::info!(
        "Cleanup: deleted {} orphan placeholder(s), kept {} torrent-tracked (torrent size {})",
        removed, kept, keep_paths.len()
    );
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub file_index: usize,
    pub file_name: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub progress: f64,
    pub finished: bool,
    /// Set by the command layer after checking DB - true once extracted and marked installed.
    #[serde(default)]
    pub installed: bool,
    /// Optional error/status message from the command layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Torrent lifecycle state from librqbit. During the `initializing` phase
    /// librqbit hashes the entire torrent's existing on-disk content before
    /// any peer pieces are requested - on Windows with thousands of placeholder
    /// files this can take several minutes, and per-file `progress` will stay
    /// at 0 the whole time. The frontend uses this to show a meaningful
    /// "Validating…" status instead of a frozen 0%.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent_state: Option<String>,
    /// Whole-torrent validation/download progress (0.0..1.0). During init this
    /// reflects librqbit's hash-check progress; once live, it tracks downloaded
    /// bytes across all selected files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent_progress: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadManagerStatus {
    pub active_downloads: Vec<DownloadProgress>,
    pub download_speed: Option<String>,
    pub upload_speed: Option<String>,
}

/// Manages BitTorrent downloads using librqbit with selective file support.
/// Must be Send+Sync for use in Tauri's managed state.
///
/// The torrent is only added to the session on first download request,
/// avoiding the creation of 14,000+ placeholder files at startup.
pub struct DownloadManager {
    session: Arc<Session>,
    handle: RwLock<Option<Arc<ManagedTorrent>>>,
    torrent_index: TorrentIndex,
    torrent_bytes: Arc<Vec<u8>>,
    selected_files: RwLock<HashSet<usize>>,
    data_dir: PathBuf,
    /// Torrent-relative paths that placeholder cleanup must never delete.
    /// All four eXoDOS torrents overlay into the same root, so this must be
    /// the UNION of every enabled collection's file list (set after all
    /// managers are built) - cleaning with only this torrent's list deletes
    /// placeholders that sibling torrents still track (cross-collection
    /// variant of the v0.6.6 stuck-download bug). Falls back to this
    /// torrent's own list when unset.
    cleanup_keep_paths: std::sync::RwLock<Option<Arc<Vec<String>>>>,
}

/// Default location for librqbit's fastresume persistence (`<info_hash>.bitv`,
/// `<info_hash>.torrent`, `session.json`). Co-located with the session dir so
/// it shares the same lifecycle (cleared by factory_reset).
pub(crate) fn fastresume_dir(session_dir: &Path) -> PathBuf {
    session_dir.join("librqbit-fastresume")
}

impl DownloadManager {
    /// Create a shared librqbit session. Call once, then pass to `new_with_session`.
    /// `session_dir` is where librqbit stores its internal state (.librqbit/).
    /// This should be the app config directory, NOT the game data directory.
    ///
    /// `persistence_dir` is where fastresume bitfields, per-torrent .torrent
    /// copies and session.json live. Pre-seeding `<info_hash>.bitv` files in
    /// here before this call lets librqbit skip its initial checksum pass on
    /// fresh installs - see `setup::seed_fastresume_bitvs`.
    pub async fn create_session(
        session_dir: &Path,
        persistence_dir: &Path,
    ) -> anyhow::Result<Arc<Session>> {
        std::fs::create_dir_all(session_dir)?;
        std::fs::create_dir_all(persistence_dir)?;
        let session = Session::new_with_opts(
            session_dir.to_path_buf(),
            SessionOptions {
                disable_dht: false,
                disable_dht_persistence: true,
                // fastresume + JSON persistence: librqbit caches the per-torrent
                // have-pieces bitfield to `<persistence_dir>/<info_hash>.bitv`.
                // On subsequent adds (or after we plant an empty bitfield for a
                // fresh install) librqbit skips the initial_check pass entirely
                // - turning a 5-10 minute Windows wait into seconds.
                fastresume: true,
                persistence: Some(SessionPersistenceConfig::Json {
                    folder: Some(persistence_dir.to_path_buf()),
                }),
                ..Default::default()
            },
        )
        .await?;
        Ok(session)
    }

    /// Initialize a download manager using a shared session.
    pub fn new_with_session(
        session: Arc<Session>,
        torrent_path: &Path,
        data_dir: &Path,
    ) -> anyhow::Result<Self> {
        let torrent_bytes = Arc::new(std::fs::read(torrent_path)?);
        let torrent_index = TorrentIndex::from_file(torrent_path)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        log::info!(
            "Download manager initialized: {} files in torrent, data dir: {}",
            torrent_index.files.len(),
            data_dir.display()
        );

        Ok(Self {
            session,
            handle: RwLock::new(None),
            torrent_index,
            torrent_bytes,
            selected_files: RwLock::new(HashSet::new()),
            data_dir: data_dir.to_path_buf(),
            cleanup_keep_paths: std::sync::RwLock::new(None),
        })
    }

    /// Set the union keep-list for placeholder cleanup (see field docs).
    pub fn set_cleanup_keep_paths(&self, paths: Arc<Vec<String>>) {
        if let Ok(mut guard) = self.cleanup_keep_paths.write() {
            *guard = Some(paths);
        }
    }

    /// Convenience: create session + manager in one call (for single-torrent use).
    pub async fn new(torrent_path: &Path, data_dir: &Path) -> anyhow::Result<Self> {
        let session = Self::create_session(data_dir, &fastresume_dir(data_dir)).await?;
        Self::new_with_session(session, torrent_path, data_dir)
    }

    /// Get the torrent file index.
    pub fn index(&self) -> &TorrentIndex {
        &self.torrent_index
    }

    /// Returns true if the given file index has been queued for download.
    pub async fn is_file_selected(&self, file_index: usize) -> bool {
        self.selected_files.read().await.contains(&file_index)
    }

    /// Get the torrent root directory: <data_dir>/<torrent_name>/
    pub fn torrent_root(&self) -> PathBuf {
        self.data_dir.join(&self.torrent_index.name)
    }

    /// Call update_only_files with retries.
    /// librqbit returns "can't update initializing torrent" if the torrent handle was
    /// freshly loaded from session state and hasn't finished its init phase.
    async fn update_files_retrying(
        &self,
        handle: &Arc<ManagedTorrent>,
        selected: &HashSet<usize>,
    ) -> anyhow::Result<()> {
        const MAX_ATTEMPTS: u32 = 20;
        const DELAY_MS: u64 = 300;
        for attempt in 0..MAX_ATTEMPTS {
            match self.session.update_only_files(handle, selected).await {
                Ok(_) => return Ok(()),
                Err(e) if e.to_string().contains("initializing") => {
                    if attempt + 1 < MAX_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(DELAY_MS)).await;
                    } else {
                        return Err(e);
                    }
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    /// Queue file indices for download. Adds the torrent on first call.
    pub async fn download_files(&self, file_indices: Vec<usize>) -> anyhow::Result<()> {
        {
            let mut selected = self.selected_files.write().await;
            for idx in &file_indices {
                selected.insert(*idx);
            }
        }

        let mut handle_guard = self.handle.write().await;

        if let Some(ref handle) = *handle_guard {
            // Torrent already running - just update file selection
            let selected = self.selected_files.read().await;
            self.update_files_retrying(handle, &selected).await?;
            log::info!("Updated file selection, added: {:?}", file_indices);
        } else {
            // First download - add torrent to session now
            let selected = self.selected_files.read().await.clone();
            // Explicitly set output_folder to torrent_root so downloads land in data_dir,
            // not in the session's default output folder (which is app_data_dir).
            // On Windows, prefix with \\?\ so file writes use the NT extended-length
            // path API (32 768-char limit) instead of the legacy MAX_PATH (260).
            // Without this, deeply nested torrent entries silently fail to open and
            // the download appears stuck at 0%.
            let raw_root = self.torrent_root();
            let output_folder = to_long_path(&raw_root);

            // Diagnostic: surface exactly what we are handing to librqbit. On
            // Windows the `\\?\` prefix should appear here. If it doesn't, the
            // long-path fix isn't being applied for this run.
            log::info!(
                "Torrent add: output_folder={:?} (torrent_root={}, exists={}, is_dir={})",
                output_folder,
                raw_root.display(),
                raw_root.exists(),
                raw_root.is_dir()
            );
            log::info!(
                "Torrent add: selected file_indices={:?} (total={} of {})",
                file_indices, selected.len(), self.torrent_index.files.len()
            );

            let response = self
                .session
                .add_torrent(
                    AddTorrent::from_bytes((*self.torrent_bytes).clone()),
                    Some(AddTorrentOptions {
                        only_files: Some(selected.into_iter().collect()),
                        overwrite: true,
                        output_folder: Some(output_folder),
                        ..Default::default()
                    }),
                )
                .await
                .map_err(|e| {
                    log::error!("session.add_torrent failed: {}", e);
                    e
                })?;

            let (handle, already_managed) = match response {
                AddTorrentResponse::Added(_id, h) => {
                    log::info!("Torrent add: response=Added");
                    (h, false)
                }
                AddTorrentResponse::AlreadyManaged(_id, h) => {
                    log::info!("Torrent add: response=AlreadyManaged");
                    (h, true)
                }
                AddTorrentResponse::ListOnly(_) => {
                    log::error!("Torrent add: response=ListOnly (unexpected - file selection ignored)");
                    return Err(anyhow::anyhow!("Torrent added in list-only mode"));
                }
            };

            *handle_guard = Some(Arc::clone(&handle));
            log::info!("Torrent added (already_managed={already_managed}), downloading files: {:?}", file_indices);

            // If the session already had this torrent, apply our file selection.
            // Retry because the handle may still be in the Initializing state when
            // AlreadyManaged is returned (session loads torrent from disk asynchronously).
            if already_managed {
                let selected = self.selected_files.read().await;
                self.update_files_retrying(handle_guard.as_ref().unwrap(), &selected).await?;
            }

            // Periodic diagnostic stats: log peer count, live state, and
            // download speed every 2 s for 60 s after a fresh add. This is
            // the window during which Windows-stuck-at-0% manifests; without
            // these snapshots we have no signal to tell network/peer issues
            // (peers=0) apart from disk/librqbit issues (peers>0, speed=0).
            // Capture file paths up front so the spawned task does not need
            // a reference to self.
            let stats_handle = Arc::clone(&handle);
            let watched_files: Vec<(usize, String, u64)> = file_indices.iter()
                .filter_map(|&idx| {
                    self.torrent_index.files.get(idx).map(|f| (idx, f.path.clone(), f.size))
                })
                .collect();
            tokio::spawn(async move {
                let start = std::time::Instant::now();
                while start.elapsed() < Duration::from_secs(60) {
                    let s = stats_handle.stats();
                    // The Display impl gives us state + progress + (when live)
                    // download/upload speeds - the most diagnostic-dense
                    // single line we can emit. Augment with the per-file
                    // breakdown so we can tell partial progress apart.
                    let per_file: Vec<String> = watched_files.iter().map(|(idx, name, size)| {
                        let dl = s.file_progress.get(*idx).copied().unwrap_or(0);
                        let pct = if *size > 0 { (dl as f64 / *size as f64) * 100.0 } else { 0.0 };
                        format!("[{}]={}/{} ({:.1}%) {}", idx, dl, size, pct, name)
                    }).collect();
                    if let Some(ref err) = s.error {
                        log::error!("[stats] state={} error={:?}", s.state, err);
                    }
                    log::info!(
                        "[stats] {} | live={} | files: {}",
                        s, s.live.is_some(), per_file.join(" ")
                    );
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                log::debug!("[stats] periodic logger finished after 60 s");
            });

            // Cleanup: removes 0-byte zip files that are NOT in the torrent's
            // file list (true orphans from a previous run / unrelated files).
            //
            // Critical: pass the FULL torrent file list, not just the user's
            // current selection. librqbit's `init()` opens (creates) every
            // file declared by the torrent, so all 14k+ slots exist as 0-byte
            // sparse files immediately after add. With fastresume enabled
            // (v0.6.4+), pieces shared between files get marked "have" once
            // any selected file's pieces arrive - and librqbit then refuses
            // to re-download those pieces, even if some target files were
            // deleted. Deleting a tracked placeholder therefore makes
            // librqbit's in-memory state lie about disk state, leaving the
            // user stuck in a "100% but zip missing" loop on subsequent
            // downloads (observed v0.6.4-v0.6.6).
            // ... and because all collections share one overlay root, prefer
            // the union keep-list over this torrent's own file list.
            let root = self.torrent_root();
            let keep_paths: Arc<Vec<String>> = self
                .cleanup_keep_paths
                .read()
                .ok()
                .and_then(|g| g.clone())
                .unwrap_or_else(|| {
                    Arc::new(
                        self.torrent_index
                            .files
                            .iter()
                            .map(|f| f.path.clone())
                            .collect(),
                    )
                });
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(10)).await;
                if let Err(e) = cleanup_placeholder_files(&root, &keep_paths) {
                    log::warn!("Failed to clean up placeholder files: {}", e);
                }
            });
        }

        Ok(())
    }

    /// Get download progress for a specific file index.
    /// Returns None if the torrent hasn't been added yet.
    pub async fn file_progress(&self, file_index: usize) -> Option<DownloadProgress> {
        let handle_guard = self.handle.read().await;
        let handle = handle_guard.as_ref()?;
        let stats = handle.stats();

        let downloaded = stats.file_progress.get(file_index).copied().unwrap_or(0);
        let total = self.torrent_index.files.get(file_index)?.size;
        let finished = total > 0 && downloaded >= total;
        let progress = if total > 0 {
            (downloaded as f64 / total as f64).min(1.0)
        } else {
            0.0
        };

        let file_name = self.torrent_index.files.get(file_index)?.path.clone();

        // Whole-torrent progress mirrors librqbit's view: during `initializing`
        // it is the validation-pass progress; once live, the cumulative download.
        let torrent_progress = if stats.total_bytes > 0 {
            Some((stats.progress_bytes as f64 / stats.total_bytes as f64).min(1.0))
        } else {
            None
        };
        let torrent_state = Some(stats.state.to_string());

        Some(DownloadProgress {
            file_index,
            file_name,
            downloaded_bytes: downloaded,
            total_bytes: total,
            progress,
            finished,
            installed: false,
            error: None,
            torrent_state,
            torrent_progress,
        })
    }

    /// Get status for all active downloads.
    pub async fn status(&self) -> DownloadManagerStatus {
        let selected = self.selected_files.read().await;
        let handle_guard = self.handle.read().await;

        let mut active_downloads = Vec::new();

        if let Some(ref handle) = *handle_guard {
            let stats = handle.stats();
            for &idx in selected.iter() {
                if let Some(entry) = self.torrent_index.files.get(idx) {
                    let downloaded = stats.file_progress.get(idx).copied().unwrap_or(0);
                    let total = entry.size;
                    let finished = total > 0 && downloaded >= total;
                    let progress = if total > 0 {
                        (downloaded as f64 / total as f64).min(1.0)
                    } else {
                        0.0
                    };
                    active_downloads.push(DownloadProgress {
                        file_index: idx,
                        file_name: entry.path.clone(),
                        downloaded_bytes: downloaded,
                        total_bytes: total,
                        progress,
                        finished,
                        installed: false,
                        error: None,
                        torrent_state: Some(stats.state.to_string()),
                        torrent_progress: if stats.total_bytes > 0 {
                            Some((stats.progress_bytes as f64 / stats.total_bytes as f64).min(1.0))
                        } else {
                            None
                        },
                    });
                }
            }
        }

        let (download_speed, upload_speed) = handle_guard
            .as_ref()
            .map(|h| {
                let s = h.stats();
                (
                    s.live.as_ref().map(|l| l.download_speed.to_string()),
                    s.live.as_ref().map(|l| l.upload_speed.to_string()),
                )
            })
            .unwrap_or((None, None));

        DownloadManagerStatus {
            active_downloads,
            download_speed,
            upload_speed,
        }
    }

    /// Remove a file from the active selection, telling librqbit to stop prioritising it.
    /// Holds the write lock across the session update to keep selected_files and the
    /// torrent session in sync - no other caller can observe a partially-updated state.
    pub async fn deselect_file(&self, file_index: usize) {
        let mut selected = self.selected_files.write().await;
        selected.remove(&file_index);
        let handle_guard = self.handle.read().await;
        if let Some(ref handle) = *handle_guard {
            let _ = self.session.update_only_files(handle, &*selected).await;
        }
    }

    /// Check if a specific file has finished downloading.
    pub async fn is_file_complete(&self, file_index: usize) -> bool {
        self.file_progress(file_index)
            .await
            .map(|p| p.finished)
            .unwrap_or(false)
    }

    /// Wait for a specific file to complete downloading.
    pub async fn wait_for_file(&self, file_index: usize) -> anyhow::Result<()> {
        loop {
            if self.is_file_complete(file_index).await {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    /// Get the output path for a downloaded file.
    pub fn file_output_path(&self, file_index: usize) -> Option<PathBuf> {
        let entry = self.torrent_index.files.get(file_index)?;
        Some(self.torrent_root().join(&entry.path))
    }
}
