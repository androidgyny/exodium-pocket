//! Game preview videos.
//!
//! eXoDOS ships one MP4 per game inside that game's `GameData/<Title>.zip`,
//! next to the manual. Those archives run from 2 MB to 1.1 GB, so playing a
//! 2.5 MB preview must not mean fetching the archive: `torrent::zip_range`
//! reads the archive's directory from its tail, then only the video's own
//! bytes, over a torrent stream that fetches pieces on demand. Measured on the
//! real catalogue: 27 MB pulled out of a 1163 MB archive.
//!
//! Fetching runs as a background job because a torrent read can block for a
//! minute waiting for peers, and the panel starts one automatically when a game
//! is opened - so it must be pollable and cancellable, exactly like downloads.
//!
//! Resolution order, cheapest first:
//!   1. the extracted cache from a previous call
//!   2. the archive already on disk (installed game, or a partial download
//!      that happens to cover the video)
//!   3. the torrent stream

use std::collections::HashMap;
use std::time::Duration;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::State;
use tokio::sync::RwLock;

use crate::db::queries;
use crate::torrent::zip_range;

use super::{DbState, TorrentState};

// ── Job state ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize)]
pub struct VideoStatus {
    /// "probing" | "fetching" | "ready" | "none" | "error"
    ///
    /// The split matters for the UI: "probing" means we are reading the
    /// archive's index and do not yet know whether a video exists at all, so
    /// there is nothing honest to announce. Only "fetching" means a video was
    /// found and its bytes are on the way.
    pub phase: String,
    /// 0..1 while fetching.
    pub progress: f64,
    /// Total bytes to transfer, so the UI can say "of 27 MB".
    pub total_bytes: u64,
    pub path: Option<String>,
    pub error: Option<String>,
}

impl VideoStatus {
    fn phase(phase: &str) -> Self {
        Self {
            phase: phase.to_string(),
            progress: 0.0,
            total_bytes: 0,
            path: None,
            error: None,
        }
    }
}

struct VideoJob {
    status: VideoStatus,
    cancel: Arc<AtomicBool>,
}

/// Tauri-managed state for in-flight video fetches. The field is private
/// because `VideoJob` is - a `pub` field of a private type is an error under
/// `-D warnings`, and nothing outside this module ever needed the access.
pub struct VideoState(Arc<RwLock<HashMap<i64, VideoJob>>>);

impl VideoState {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }
}

impl Default for VideoState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Paths ────────────────────────────────────────────────────────────────────

fn video_cache_dir(data_dir: &str) -> PathBuf {
    // No leading dot - the asset-protocol scope glob skips hidden components,
    // which is why the first version served nothing (see setup::gallery_cache_dir).
    PathBuf::from(data_dir).join("content").join("videocache")
}

fn cache_path(data_dir: &str, collection: &str, file_index: usize) -> PathBuf {
    video_cache_dir(data_dir).join(format!("{}_{}.mp4", collection, file_index))
}

/// Marks an archive as having no video.
///
/// Whether a game has one is only knowable from the archive's own index, which
/// sits at the end of a file in the torrent - so the answer costs a piece
/// download (8 MB), every time, for a game that turns out to have nothing. The
/// catalogue cannot help: its `MissingVideo` flag said "true" for 16 of the 24
/// sampled games that do have one. So the answer is written down and the
/// question asked once per archive, ever.
fn no_video_marker(data_dir: &str, collection: &str, file_index: usize) -> PathBuf {
    video_cache_dir(data_dir).join(format!("{}_{}.novideo", collection, file_index))
}

async fn write_cache(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| e.to_string())?;
    }
    // Write beside the target first: a half-written MP4 left by a crash would
    // otherwise be served as a valid cache entry forever.
    let tmp = path.with_extension("part");
    tokio::fs::write(&tmp, bytes).await.map_err(|e| e.to_string())?;
    tokio::fs::rename(&tmp, path).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Videos are 2-27 MB each, so browsing a few hundred games would otherwise
/// fill a disk quietly. Only the videos are pruned: the `.novideo` markers are
/// empty files that cost nothing and save a piece download each.
const VIDEO_CACHE_MAX_BYTES: u64 = 1024 * 1024 * 1024;

pub fn prune_video_cache(data_dir: &str) {
    let dir = video_cache_dir(data_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut videos: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    let mut total = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("mp4") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        total += meta.len();
        videos.push((path, meta.len(), meta.modified().unwrap_or(std::time::UNIX_EPOCH)));
    }
    if total <= VIDEO_CACHE_MAX_BYTES {
        return;
    }
    videos.sort_by_key(|(_, _, modified)| *modified);
    let target = VIDEO_CACHE_MAX_BYTES / 5 * 4;
    let mut freed = 0u64;
    for (path, len, _) in videos {
        if total - freed <= target {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            freed += len;
        }
    }
    log::info!("Video cache pruned: {:.1} MB freed", freed as f64 / 1_048_576.0);
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Kick off (or join) the fetch for a game's preview video. Returns
/// immediately; poll `get_video_status`.
#[tauri::command]
pub async fn start_game_video(
    app: tauri::AppHandle,
    db_state: State<'_, DbState>,
    torrent_state: State<'_, TorrentState>,
    video_state: State<'_, VideoState>,
    id: i64,
) -> Result<VideoStatus, String> {
    let (gamedata_idx, source, data_dir) = {
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        let game = queries::fetch_game_by_id(&conn, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Game {} not found", id))?;
        let data_dir = queries::get_config(&conn, "data_dir")
            .map_err(|e| e.to_string())?
            .ok_or("Data directory not configured")?;

        // Extras live in the English archive only: EVERY localized row has a
        // NULL gamedata index (DE 484/484, ES 413/413, PL 56/56), so a German
        // selection would otherwise report "no video" for a game that has one.
        let (idx, source) = match game.gamedata_torrent_index {
            Some(idx) => (
                Some(idx),
                game.torrent_source.unwrap_or_else(|| "eXoDOS".to_string()),
            ),
            None => {
                let sibling = game.shortcode.as_deref().and_then(|sc| {
                    conn.query_row(
                        "SELECT gamedata_torrent_index, torrent_source FROM games \
                         WHERE shortcode = ?1 AND gamedata_torrent_index IS NOT NULL \
                         ORDER BY CASE WHEN language = 'EN' THEN 0 ELSE 1 END LIMIT 1",
                        rusqlite::params![sc],
                        |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<String>>(1)?)),
                    )
                    .ok()
                });
                match sibling {
                    Some((idx, src)) => (idx, src.unwrap_or_else(|| "eXoDOS".to_string())),
                    None => (None, "eXoDOS".to_string()),
                }
            }
        };
        (idx, source, data_dir)
    };

    let Some(gamedata_idx) = gamedata_idx else {
        return Ok(VideoStatus::phase("none"));
    };
    let gamedata_idx = gamedata_idx as usize;

    // Asked before, answer was no - do not pay for the same piece twice.
    if no_video_marker(&data_dir, &source, gamedata_idx).is_file() {
        return Ok(VideoStatus::phase("none"));
    }

    // Already extracted - the common case after the first visit.
    let cached = cache_path(&data_dir, &source, gamedata_idx);
    if cached.is_file() {
        let mut status = VideoStatus::phase("ready");
        status.progress = 1.0;
        status.path = Some(crate::commands::setup::path_to_fwd_slash(&cached));
        return Ok(status);
    }

    // Join an in-flight job rather than starting a second one.
    {
        let jobs = video_state.0.read().await;
        if let Some(job) = jobs.get(&id) {
            if job.status.phase == "probing" || job.status.phase == "fetching" {
                return Ok(job.status.clone());
            }
        }
    }

    let manager = {
        let guard = torrent_state.0.read().await;
        guard.get(&source).cloned()
    };
    let Some(manager) = manager else {
        // Offline is a legitimate state, not an error worth a red toast.
        return Ok(VideoStatus::phase("none"));
    };

    let file = manager
        .index()
        .files
        .get(gamedata_idx)
        .ok_or_else(|| format!("GameData index {} out of range", gamedata_idx))?
        .clone();

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut jobs = video_state.0.write().await;
        jobs.insert(
            id,
            VideoJob {
                status: VideoStatus::phase("probing"),
                cancel: Arc::clone(&cancel),
            },
        );
    }

    let jobs_arc = Arc::clone(&video_state.0);
    let marker = no_video_marker(&data_dir, &source, gamedata_idx);
    let local_archive = PathBuf::from(&data_dir).join("eXoDOS").join(&file.path);
    let archive_len = file.size;
    let archive_path = file.path.clone();

    tauri::async_runtime::spawn(async move {
        let result = fetch_video(
            &jobs_arc,
            id,
            &manager,
            gamedata_idx,
            &local_archive,
            archive_len,
            &archive_path,
            &cached,
            &cancel,
        )
        .await;

        let mut jobs = jobs_arc.write().await;
        let Some(job) = jobs.get_mut(&id) else { return };
        match result {
            Ok(Some(path)) => {
                job.status.phase = "ready".into();
                job.status.progress = 1.0;
                job.status.path = Some(path);
            }
            Ok(None) => {
                job.status.phase = "none".into();
                // A read that completed and found nothing is a real answer;
                // a timeout is not, and lands in the Err arm below.
                if let Some(parent) = marker.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                let _ = tokio::fs::write(&marker, b"").await;
            }
            Err(e) if e == "cancelled" => {
                jobs.remove(&id);
            }
            Err(e) => {
                log::warn!("Video fetch for game {} failed: {}", id, e);
                job.status.phase = "error".into();
                job.status.error = Some(e);
            }
        }
    });

    let _ = app; // AppHandle reserved for future event emission
    Ok(VideoStatus::phase("probing"))
}

#[allow(clippy::too_many_arguments)]
async fn fetch_video(
    jobs: &Arc<RwLock<HashMap<i64, VideoJob>>>,
    id: i64,
    manager: &Arc<crate::torrent::manager::DownloadManager>,
    gamedata_idx: usize,
    local_archive: &Path,
    archive_len: u64,
    archive_path: &str,
    cached: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<Option<String>, String> {
    // 1) A local archive costs nothing: installed games keep their GameData,
    //    and even a partial download can already cover the video.
    if local_archive.is_file() {
        if let Ok(mut handle) = tokio::fs::File::open(local_archive).await {
            match extract(&mut handle, archive_len, jobs, id, cancel).await {
                Ok(Some(bytes)) => {
                    write_cache(cached, &bytes).await?;
                    log::info!("Video for game {} read from the local archive", id);
                    return Ok(Some(crate::commands::setup::path_to_fwd_slash(cached)));
                }
                Ok(None) => return Ok(None),
                Err(e) if e == "cancelled" => return Err(e),
                Err(e) => log::info!("Local archive unusable for game {} ({}), streaming", id, e),
            }
        }
    }

    // 2) Stream: seeks become piece requests, so the transfer is bounded by the
    //    video's size rather than the archive's.
    log::info!(
        "Streaming video for game {} from {} ({:.1} MB archive)",
        id,
        archive_path,
        archive_len as f64 / 1_048_576.0
    );
    let mut stream = manager
        .stream_file(gamedata_idx)
        .await
        .map_err(|e| format!("Could not open the archive stream: {}", e))?;
    let Some(bytes) = extract(&mut stream, archive_len, jobs, id, cancel).await? else {
        log::info!("Archive for game {} contains no video", id);
        return Ok(None);
    };
    write_cache(cached, &bytes).await?;
    log::info!(
        "Video for game {} extracted: {:.1} MB",
        id,
        bytes.len() as f64 / 1_048_576.0
    );
    Ok(Some(crate::commands::setup::path_to_fwd_slash(cached)))
}

/// A stream waits for pieces indefinitely, and pieces nobody seeds never
/// arrive - one such game would otherwise hold a slot for the whole session
/// (observed: AH-3 ThunderStrike sat in "fetching" for 20 minutes). Both reads
/// get a deadline; the directory's is shorter because it is a few kilobytes
/// from the archive's tail, so slowness there means the pieces are unavailable
/// rather than large.
const DIRECTORY_TIMEOUT: Duration = Duration::from_secs(45);
const ENTRY_TIMEOUT: Duration = Duration::from_secs(300);

/// Read the archive directory, then the video entry, publishing progress as it
/// goes so the panel can show something other than a spinner.
async fn extract<R>(
    reader: &mut R,
    archive_len: u64,
    jobs: &Arc<RwLock<HashMap<i64, VideoJob>>>,
    id: i64,
    cancel: &Arc<AtomicBool>,
) -> Result<Option<Vec<u8>>, String>
where
    R: tokio::io::AsyncRead + tokio::io::AsyncSeek + Unpin,
{
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let entries = tokio::time::timeout(
        DIRECTORY_TIMEOUT,
        zip_range::read_central_directory(reader, archive_len),
    )
    .await
    .map_err(|_| "timed out reading the archive index".to_string())?
    .map_err(|e| e.to_string())?;
    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".into());
    }
    let Some(video) = zip_range::find_video(&entries) else {
        log::info!(
            "No video in the archive for game {} ({} entries, folders: {})",
            id,
            entries.len(),
            zip_range::top_level_folders(&entries).join(", ")
        );
        return Ok(None);
    };

    let total = video.compressed_size;
    {
        // A video exists - from here the UI has something true to show.
        let mut guard = jobs.write().await;
        if let Some(job) = guard.get_mut(&id) {
            job.status.phase = "fetching".into();
            job.status.total_bytes = video.uncompressed_size;
        }
    }

    // Progress is published from a blocking callback, so it uses the sync
    // try_write path - a missed tick is fine, the next chunk catches up.
    let cancel_flag = Arc::clone(cancel);
    let jobs_for_cb = Arc::clone(jobs);
    let bytes = tokio::time::timeout(ENTRY_TIMEOUT, zip_range::read_entry_with(reader, video, move |read, _| {
        if cancel_flag.load(Ordering::Relaxed) {
            return false;
        }
        if let Ok(mut guard) = jobs_for_cb.try_write() {
            if let Some(job) = guard.get_mut(&id) {
                job.status.progress = if total > 0 { read as f64 / total as f64 } else { 0.0 };
            }
        }
        true
    }))
    .await
    .map_err(|_| "timed out fetching the video".to_string())?
    .map_err(|e| {
        if e.to_string().contains("cancelled") { "cancelled".to_string() } else { e.to_string() }
    })?;
    Ok(Some(bytes))
}

#[tauri::command]
pub async fn get_video_status(
    video_state: State<'_, VideoState>,
    id: i64,
) -> Result<Option<VideoStatus>, String> {
    Ok(video_state.0.read().await.get(&id).map(|j| j.status.clone()))
}

/// Stop an in-flight fetch - the panel calls this when the user moves on, so
/// browsing through games doesn't leave a queue of torrent reads behind.
#[tauri::command]
pub async fn cancel_game_video(video_state: State<'_, VideoState>, id: i64) -> Result<(), String> {
    let jobs = video_state.0.read().await;
    if let Some(job) = jobs.get(&id) {
        job.cancel.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::SeekFrom;
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};

    /// A torrent stream whose pieces never arrive: reads park forever. This is
    /// what an unseeded region looks like from the reader's side, and what made
    /// one game hold a fetch slot for 20 minutes.
    struct StalledReader;

    impl AsyncRead for StalledReader {
        fn poll_read(self: Pin<&mut Self>, _: &mut Context<'_>, _: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
            Poll::Pending
        }
    }

    impl AsyncSeek for StalledReader {
        fn start_seek(self: Pin<&mut Self>, _: SeekFrom) -> std::io::Result<()> {
            Ok(())
        }
        fn poll_complete(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<u64>> {
            Poll::Ready(Ok(0))
        }
    }

    /// Markers are the reason a game with no video is asked about once rather
    /// than on every visit - pruning must never take them.
    #[test]
    fn pruning_keeps_the_no_video_markers() {
        let dir = std::env::temp_dir().join(format!("exodium_vidprune_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = dir.join("content").join("videocache");
        std::fs::create_dir_all(&cache).unwrap();

        std::fs::write(cache.join("eXoDOS_1.novideo"), b"").unwrap();
        for i in 0..3 {
            std::fs::write(cache.join(format!("eXoDOS_{}.mp4", 10 + i)), vec![0u8; 4096]).unwrap();
        }

        prune_video_cache(dir.to_str().unwrap());

        // Well under the cap: nothing should go.
        assert!(cache.join("eXoDOS_1.novideo").exists());
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(start_paused = true)]
    async fn a_stalled_stream_gives_up_instead_of_holding_the_slot() {
        let jobs: Arc<RwLock<HashMap<i64, VideoJob>>> = Arc::new(RwLock::new(HashMap::new()));
        jobs.write().await.insert(
            1,
            VideoJob {
                status: VideoStatus::phase("fetching"),
                cancel: Arc::new(AtomicBool::new(false)),
            },
        );
        let cancel = Arc::new(AtomicBool::new(false));

        let err = extract(&mut StalledReader, 10_000_000, &jobs, 1, &cancel)
            .await
            .expect_err("a stream that never delivers must fail");

        assert!(err.contains("timed out"), "unexpected error: {}", err);
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_is_reported_as_such_not_as_a_failure() {
        let jobs: Arc<RwLock<HashMap<i64, VideoJob>>> = Arc::new(RwLock::new(HashMap::new()));
        let cancel = Arc::new(AtomicBool::new(true));
        let err = extract(&mut StalledReader, 10_000_000, &jobs, 2, &cancel)
            .await
            .expect_err("cancelled");
        // Cancellation is a user action, so it must never surface as an error
        // phase - the caller distinguishes on this exact string. It must also
        // cost no I/O: a job cancelled while queued should not read anything.
        assert_eq!(err, "cancelled");
    }
}
