use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use tauri::AppHandle;

/// Per-game (last_retry_at, attempts) for stuck-download recovery in
/// `get_download_progress`. Module-scoped so the success branch can clear
/// the entry once the ZIP appears, preventing a stale counter from
/// surfacing a premature error if the same game gets stuck again.
static RETRY_STATE: OnceLock<
    Mutex<std::collections::HashMap<i64, (std::time::Instant, u32)>>,
> = OnceLock::new();

fn retry_state() -> &'static Mutex<std::collections::HashMap<i64, (std::time::Instant, u32)>> {
    RETRY_STATE.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

use rusqlite::Connection;
use serde::Serialize;
use tauri::State;

use crate::db;
use crate::db::queries;
use crate::models::Game;
use crate::torrent::manager::DownloadProgress;

use super::TorrentState;

/// Resolve the data directory for a collection.
/// All collections share the same data directory (overlay model - no collection subdirectories).
pub fn collection_data_dir(data_dir: &str, _source: &str) -> PathBuf {
    std::path::Path::new(data_dir).to_path_buf()
}

/// Get the inner folder name for a collection (the folder the torrent creates).
fn collection_inner_folder(source: &str) -> &'static str {
    crate::commands::setup::collection_def(source)
        .map(|c| c.inner_folder)
        .unwrap_or("eXoDOS")
}

/// Get the game directory prefix for a collection (path from inner_folder to game dirs).
fn collection_game_prefix(source: &str) -> &'static str {
    crate::commands::setup::collection_def(source)
        .map(|c| c.game_prefix)
        .unwrap_or("eXo/eXoDOS")
}

/// Get the language subdirectory for an LP collection, if any.
fn collection_lang_dir(source: &str) -> Option<&'static str> {
    crate::commands::setup::collection_def(source).and_then(|c| c.lang_dir)
}

/// Language subdirectories used in the eXoDOS file structure.
const LANG_DIRS: &[&str] = &["!german", "!polish", "!czech", "!slovak", "!spanish"];

pub struct DbState(pub Mutex<Connection>);

#[derive(Debug, Clone, Serialize)]
pub struct GameList {
    pub games: Vec<Game>,
    pub total: usize,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn get_games(
    state: State<DbState>,
    page: Option<usize>,
    per_page: Option<usize>,
    query: Option<String>,
    genre: Option<String>,
    sort_by: Option<String>,
    collection: Option<String>,
    favorites_only: Option<bool>,
) -> Result<GameList, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let page = page.unwrap_or(1);
    let per_page = per_page.unwrap_or(50).min(10000);
    let query = query.unwrap_or_default();
    let genre = genre.unwrap_or_default();
    let sort_by = sort_by.unwrap_or_default();
    let collection = collection.unwrap_or_default();

    let f = queries::GameFilter {
        query: &query,
        genre: &genre,
        sort_by: &sort_by,
        collection: &collection,
        favorites_only: favorites_only.unwrap_or(false),
    };

    let total = queries::count_games_filtered(&conn, &f).map_err(|e| e.to_string())?;
    let games = queries::fetch_games_filtered(&conn, page, per_page, &f).map_err(|e| e.to_string())?;

    Ok(GameList { games, total })
}

#[tauri::command]
pub fn get_genres(state: State<DbState>, collection: Option<String>) -> Result<Vec<String>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let collection = collection.unwrap_or_default();
    queries::get_genres(&conn, &collection).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_section_keys(
    state: State<DbState>,
    sort_by: Option<String>,
    query: Option<String>,
    genre: Option<String>,
    collection: Option<String>,
    favorites_only: Option<bool>,
) -> Result<Vec<String>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let sort_by = sort_by.unwrap_or_default();
    let query = query.unwrap_or_default();
    let genre = genre.unwrap_or_default();
    let collection = collection.unwrap_or_default();
    let f = queries::GameFilter {
        query: &query,
        genre: &genre,
        sort_by: &sort_by,
        collection: &collection,
        favorites_only: favorites_only.unwrap_or(false),
    };
    let result = queries::get_section_keys(&conn, &f).map_err(|e| e.to_string());
    log::debug!("get_section_keys: sort_by={:?} collection={:?} → {:?} keys", sort_by, collection, result.as_ref().map(|v| v.len()));
    result
}

#[tauri::command]
pub fn get_game_variants(state: State<'_, DbState>, shortcode: String) -> Result<Vec<Game>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    queries::fetch_game_variants(&conn, &shortcode).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_installed_games(state: State<DbState>) -> Result<Vec<Game>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    queries::fetch_installed_games(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn toggle_favorite(state: State<DbState>, id: i64) -> Result<bool, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    queries::toggle_favorite(&conn, id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_game(state: State<DbState>, id: i64) -> Result<Option<Game>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    queries::fetch_game_by_id(&conn, id).map_err(|e| e.to_string())
}


#[tauri::command]
pub fn get_config(state: State<DbState>, key: String) -> Result<Option<String>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    queries::get_config(&conn, &key).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_config(
    app: tauri::AppHandle,
    state: State<DbState>,
    key: String,
    value: String,
) -> Result<(), String> {
    {
        let conn = state.0.lock().map_err(|e| e.to_string())?;
        queries::set_config(&conn, &key, &value).map_err(|e| e.to_string())?;
    }
    // The static asset-protocol scope only covers $RESOURCE/$APPDATA; the
    // user-chosen game dir (thumbnails, screenshots, manuals served via the
    // asset protocol) is granted at runtime - here on change, and at startup
    // in lib.rs for the stored value.
    if key == "data_dir" {
        crate::allow_asset_dir(&app, std::path::Path::new(&value));
    }
    Ok(())
}

/// Toggle seeding (uploading to the swarm). Persists the choice and applies
/// it live to the shared torrent session.
#[tauri::command]
pub async fn set_seeding_enabled(
    db_state: State<'_, DbState>,
    torrent_state: State<'_, TorrentState>,
    enabled: bool,
) -> Result<(), String> {
    {
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        queries::set_config(&conn, "seeding_enabled", if enabled { "1" } else { "0" })
            .map_err(|e| e.to_string())?;
    }
    // All managers share one session - applying via any of them is enough.
    let mgr = { torrent_state.0.read().await.values().next().cloned() };
    if let Some(mgr) = mgr {
        mgr.set_seeding(enabled);
    }
    Ok(())
}

/// Queue a game for download via torrent.
#[tauri::command]
pub async fn download_game(
    db_state: State<'_, DbState>,
    torrent_state: State<'_, TorrentState>,
    id: i64,
) -> Result<String, String> {
    let game = {
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        queries::fetch_game_by_id(&conn, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Game {} not found", id))?
    };

    if game.installed {
        return Ok(format!("{} is already installed", game.title));
    }

    // Disk-space preflight: refusing upfront beats a multi-GB torrent (plus
    // ~equal-sized extraction) failing halfway with a partial install.
    if let Some(size) = game.download_size {
        let data_dir = {
            let conn = db_state.0.lock().map_err(|e| e.to_string())?;
            queries::get_config(&conn, "data_dir").ok().flatten()
        };
        if let Some(dir) = data_dir {
            // download ZIP + extracted contents + safety margin
            let needed = (size as u64).saturating_mul(2) + 500 * 1024 * 1024;
            if let Ok(free) = fs4::available_space(std::path::Path::new(&dir)) {
                if free < needed {
                    let gib = |b: u64| b as f64 / (1024.0 * 1024.0 * 1024.0);
                    return Err(format!(
                        "Not enough disk space for {}: needs about {:.1} GB free \
                         (download + extraction), but only {:.1} GB is available.",
                        game.title,
                        gib(needed),
                        gib(free)
                    ));
                }
            }
        }
    }

    // Mark as in library immediately
    {
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        queries::set_in_library(&conn, id).map_err(|e| e.to_string())?;
    }

    let game_idx = game
        .game_torrent_index
        .ok_or_else(|| format!("{} has no torrent index - cannot download", game.title))?
        as usize;

    let source = game.torrent_source.as_deref().unwrap_or("eXoDOS");

    // Clone Arc references and immediately drop the guard so we don't hold it across awaits.
    let (manager, main_mgr_opt) = {
        let guard = torrent_state.0.read().await;
        let manager = guard
            .get(source)
            .cloned()
            .ok_or_else(|| format!("Download manager for '{}' not initialized.", source))?;
        let main_mgr = guard.get("eXoDOS").cloned();
        (manager, main_mgr)
    };

    let mut files = vec![game_idx];
    if let Some(gd_idx) = game.gamedata_torrent_index {
        files.push(gd_idx as usize);
    }

    if let Some(ref main_mgr) = main_mgr_opt {
        // Queue !DOSmetadata.zip (DOSBox configs) if the configs tree is
        // missing (normally pre-created by the bundled configs zip).
        let main_prefix = collection_game_prefix("eXoDOS");
        let main_segment = crate::commands::setup::collection_def("eXoDOS")
            .map(|c| c.shortcode_segment)
            .unwrap_or("!dos");
        let dosbox_dir = main_mgr
            .torrent_root()
            .join(format!("{}/{}", main_prefix, main_segment));
        if !dosbox_dir.exists() {
            if let Some(dm) = main_mgr.index().find_dosbox_metadata_zip() {
                let _ = main_mgr.download_files(vec![dm.index]).await;
                log::info!("Also downloading !DOSmetadata.zip (DOSBox configs)");
            }
        }

        // Music support: the MT-32 ROMs + SoundCanvas soundfont live in
        // eXo/util/util.zip (~630 MB; NOT in !DOSmetadata.zip, which is
        // configs only). Fetch it once, when a game whose config actually
        // requests MIDI is downloaded - ~1/3 of the catalog does; the rest
        // never pays the download.
        let mt32_dir = main_mgr.torrent_root().join("eXo/mt32");
        let needs_midi_assets = !mt32_dir.exists()
            && game_requests_midi(&main_mgr.torrent_root(), game.dosbox_conf.as_deref());
        let needs_ece = cfg!(windows)
            && game
                .dosbox_variant
                .as_deref()
                .is_some_and(|v| v.starts_with("ece"))
            && !main_mgr
                .torrent_root()
                .join("eXo/emulators/dosbox/ece4230")
                .exists();
        if needs_midi_assets || needs_ece {
            if let Some(util) = main_mgr.index().find_by_suffix("util/util.zip") {
                let util_index = util.index;
                let util_size = util.size;
                if !main_mgr.is_file_selected(util_index).await {
                    let _ = main_mgr.download_files(vec![util_index]).await;
                    log::info!(
                        "Also downloading util.zip ({:.0} MB, one-time: MT-32 ROMs + SoundCanvas soundfont for MIDI music)",
                        util_size as f64 / 1e6
                    );
                }
                // Always (re)arm the watcher - it also covers the case where
                // util.zip finished in a previous run but extraction never
                // happened (nobody was polling when it completed).
                spawn_mt32_extraction_watcher(std::sync::Arc::clone(main_mgr), util_index);
            }
        }
    }

    manager
        .download_files(files)
        .await
        .map_err(|e| format!("Failed to queue download: {}", e))?;

    Ok(format!("Downloading: {}", game.title))
}

/// Get download progress for a game. If complete, extract and mark installed.
#[tauri::command]
pub async fn get_download_progress(
    db_state: State<'_, DbState>,
    torrent_state: State<'_, TorrentState>,
    id: i64,
) -> Result<Option<DownloadProgress>, String> {
    let (game_idx, title, already_installed, source) = {
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        let game = queries::fetch_game_by_id(&conn, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Game {} not found", id))?;
        match game.game_torrent_index {
            Some(idx) => (
                idx as usize,
                game.title,
                game.installed,
                game.torrent_source.unwrap_or_else(|| "eXoDOS".to_string()),
            ),
            None => return Ok(None),
        }
    };

    // Clone Arc references and drop the guard immediately - the guard must not be held
    // across any .await point to avoid blocking concurrent writers.
    let (manager, main_mgr_opt) = {
        let guard = torrent_state.0.read().await;
        let manager = match guard.get(&source).cloned() {
            Some(m) => m,
            None => return Ok(None),
        };
        let main_mgr = guard.get("eXoDOS").cloned();
        (manager, main_mgr)
    };

    let mut progress = manager.file_progress(game_idx).await;

    // Log progress details for debugging
    if let Some(ref p) = progress {
        log::debug!(
            "Progress {}: idx={} {}/{} bytes ({:.1}%) finished={} installed={}",
            title, game_idx, p.downloaded_bytes, p.total_bytes,
            p.progress * 100.0, p.finished, already_installed
        );
    }

    // Attach installed status from DB
    if let Some(ref mut p) = progress {
        p.installed = already_installed;
    }

    // Disk-based completion fallback: librqbit's in-memory file_progress can
    // stall short of total_bytes for files that are in fact fully written to
    // disk - observed when multiple parallel downloads share a torrent and
    // the per-file stat lags behind actual assembly. The bug manifests as
    // "Waiting for last pieces..." forever, only recovering on app restart
    // (when session state is reloaded from disk). If the target file exists
    // with the expected size, trust the disk over the stats.
    if let Some(ref mut p) = progress {
        if !p.finished && p.total_bytes > 0 && p.progress >= 0.99 {
            if let Some(zip_path) = manager.file_output_path(game_idx) {
                if let Ok(meta) = std::fs::metadata(&zip_path) {
                    if meta.len() >= p.total_bytes {
                        log::info!(
                            "Disk-based completion: {} fully assembled ({} bytes) but librqbit stats lagged at {}. Treating as finished.",
                            title, meta.len(), p.downloaded_bytes
                        );
                        p.downloaded_bytes = p.total_bytes;
                        p.progress = 1.0;
                        p.finished = true;
                    }
                }
            }
        }
    }

    // Extract !DOSmetadata.zip if it just finished downloading (check main eXoDOS manager)
    if let Some(ref main_mgr) = main_mgr_opt {
        if let Some(dosbox_meta) = main_mgr.index().find_dosbox_metadata_zip() {
            if main_mgr.is_file_complete(dosbox_meta.index).await {
                if let Some(zip_path) = main_mgr.file_output_path(dosbox_meta.index) {
                    let lock = zip_path.with_extension("extracted");
                    if zip_path.exists() && !lock.exists() {
                        let torrent_root = main_mgr.torrent_root();
                        tauri::async_runtime::spawn_blocking(move || {
                            let result = (|| -> Result<(), String> {
                                let file = std::fs::File::open(&zip_path).map_err(|e| e.to_string())?;
                                let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
                                archive.extract(&torrent_root).map_err(|e| e.to_string())?;
                                std::fs::write(&lock, "").map_err(|e| e.to_string())?;
                                Ok(())
                            })();
                            match result {
                                Ok(()) => log::info!("Extracted DOSBox configs to {}", torrent_root.display()),
                                Err(e) => log::error!("Failed to extract DOSBox configs: {}", e),
                            }
                        });
                    }
                }
            }
        }
    }

    // If download is complete and not yet installed, extract the ZIP and mark installed.
    if let Some(ref p) = progress {
        if p.finished && !already_installed {
            let zip_out = manager.file_output_path(game_idx);
            log::debug!(
                "Extraction check for {}: zip_path={:?} exists={}",
                title, zip_out, zip_out.as_ref().map(|p| p.exists()).unwrap_or(false)
            );
            if let Some(zip_path) = zip_out {
                if zip_path.exists() {
                    // ZIP materialised - clear any stuck-download retry counter so a
                    // future stuck cycle on the same game id starts fresh from 0.
                    if let Ok(mut map) = retry_state().lock() {
                        map.remove(&id);
                    }
                    let lock_path = zip_path.with_extension("extracting");

                    // Clean up stale lock files (e.g., from crashed/interrupted extraction)
                    if lock_path.exists() {
                        if let Ok(age) = std::fs::metadata(&lock_path)
                            .and_then(|m| m.modified())
                            .and_then(|t| t.elapsed().map_err(std::io::Error::other))
                        {
                            if age.as_secs() > 300 {
                                log::warn!("Removing stale extraction lock: {}", lock_path.display());
                                let _ = std::fs::remove_file(&lock_path);
                            }
                        }
                    }

                    if std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&lock_path)
                        .is_ok()
                    {
                        let extract_dir = zip_path.parent().unwrap().to_path_buf();
                        let game_id = id;
                        let db_path = {
                            let conn = db_state.0.lock().map_err(|e| e.to_string())?;
                            conn.path().map(PathBuf::from)
                                .ok_or_else(|| "Cannot determine database path".to_string())?
                        };

                        tauri::async_runtime::spawn_blocking(move || {
                            log::info!("Extracting {} from {}", title, zip_path.display());
                            if let Err(e) = extract_game_zip(&zip_path, &extract_dir) {
                                log::error!("Failed to extract {}: {}", title, e);
                                let _ = std::fs::remove_file(&lock_path);
                                return;
                            }
                            match db::open(&db_path) {
                                Ok(conn) => {
                                    if let Err(e) = queries::set_game_installed(&conn, game_id, true) {
                                        log::error!("Failed to mark {} installed: {}", title, e);
                                    } else {
                                        log::info!("Installed: {}", title);
                                    }
                                }
                                Err(e) => log::error!("Failed to open DB for install update: {}", e),
                            }
                            let _ = std::fs::remove_file(&lock_path);
                        });
                    }
                } else {
                    // ZIP not on disk despite torrent reporting 100%.
                    // Common cause: pieces covering this file were received as a side effect of
                    // downloading a neighboring file, but librqbit never assembled the pieces
                    // into the output file. A plain re-select is a no-op when librqbit's view
                    // of the file is "already complete", so we toggle the selection - deselect,
                    // briefly yield, then re-add - to nudge librqbit into re-evaluating the file.
                    // Throttled to one attempt every 5 seconds to avoid spamming the session.
                    log::warn!(
                        "Download reports 100% but ZIP missing: {}. Re-requesting file assembly.",
                        zip_path.display()
                    );
                    let retry_key = id;
                    let now = std::time::Instant::now();
                    // After this many failed retries (~5 min at 5 s intervals), give up
                    // and surface an error so the UI stops polling forever and the user
                    // can take action (cancel + re-download).
                    const MAX_ATTEMPTS: u32 = 60;
                    // Returns (attempts_so_far, did_increment_this_poll). The counter only
                    // ticks every 5 s; in-between polls observe the same value with
                    // did_increment=false, so error/recovery decisions stay stable across
                    // every poll instead of flickering with the throttle window.
                    let (attempts, ticked) = retry_state().lock()
                        .map(|mut map| {
                            // Prune stale entries (>2 minutes idle) to bound memory.
                            map.retain(|_, (t, _)| now.duration_since(*t).as_secs() < 120);
                            // checked_sub: `Instant - Duration` panics when the process
                            // hasn't been alive that long (observed shortly after boot).
                            let seed = now
                                .checked_sub(std::time::Duration::from_secs(60))
                                .unwrap_or(now);
                            let entry = map.entry(retry_key).or_insert((seed, 0));
                            if now.duration_since(entry.0).as_secs() >= 5 {
                                entry.0 = now;
                                entry.1 = entry.1.saturating_add(1);
                                (entry.1, true)
                            } else {
                                (entry.1, false)
                            }
                        })
                        .unwrap_or((0, false));
                    if ticked {
                        if attempts <= MAX_ATTEMPTS {
                            let mgr = manager.clone();
                            tauri::async_runtime::spawn(async move {
                                // Toggle: deselect → tiny pause → re-add. The pause lets librqbit
                                // settle the deselect bookkeeping before the next selection update.
                                mgr.deselect_file(game_idx).await;
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                let _ = mgr.download_files(vec![game_idx]).await;
                            });
                        } else if attempts == MAX_ATTEMPTS + 1 {
                            log::error!(
                                "Giving up on stuck download for game {} ({}) after {} retries; \
                                 surfacing error to UI",
                                id, title, MAX_ATTEMPTS
                            );
                        }
                    }
                    // Show as still in-progress so the frontend keeps polling until the ZIP
                    // appears and extraction can proceed normally - unless we've exhausted retries,
                    // in which case surface an error so the UI can prompt the user to cancel.
                    if let Some(ref mut p) = progress {
                        p.finished = false;
                        if attempts > MAX_ATTEMPTS {
                            p.error = Some(
                                "Download stuck - librqbit reports 100% but the file isn't on disk. \
                                 Cancel and re-download to recover.".to_string()
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(progress)
}

/// Cancel an in-progress download: deselects the file from the torrent, then clears in_library.
/// Deselect happens first so the DB and torrent state stay consistent even if one step fails.
#[tauri::command]
pub async fn cancel_download(
    db_state: State<'_, DbState>,
    torrent_state: State<'_, TorrentState>,
    id: i64,
) -> Result<(), String> {
    let (game_idx, gamedata_idx, source) = {
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        let game = queries::fetch_game_by_id(&conn, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Game {} not found", id))?;

        // The GameData ZIP is shared with this game's other language
        // variants (LP installs auto-download the EN GameData). Only
        // deselect it when no other in-flight download still needs it.
        let gamedata_idx = match game.gamedata_torrent_index {
            Some(gd) => {
                // in_library alone (not installed=0): a variant whose game
                // ZIP already extracted may still be fetching this GameData.
                // Over-retention for long-installed variants is harmless -
                // their GameData is complete anyway.
                let still_needed: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM games \
                         WHERE gamedata_torrent_index = ?1 AND id != ?2 \
                           AND in_library = 1",
                        rusqlite::params![gd, id],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                if still_needed > 0 {
                    log::info!(
                        "cancel_download: keeping shared GameData index {} ({} other download(s) need it)",
                        gd, still_needed
                    );
                    None
                } else {
                    Some(gd as usize)
                }
            }
            None => None,
        };

        (
            game.game_torrent_index.map(|i| i as usize),
            gamedata_idx,
            game.torrent_source.unwrap_or_else(|| "eXoDOS".to_string()),
        )
    };

    // Deselect from torrent first - if this fails silently, we still want to clear the DB flag.
    // Clone Arc before dropping the guard so we don't hold the read lock across awaits.
    {
        let manager_arc = {
            let guard = torrent_state.0.read().await;
            guard.get(&source).cloned()
        };
        if let Some(manager) = manager_arc {
            if let Some(idx) = game_idx {
                manager.deselect_file(idx).await;
            }
            if let Some(idx) = gamedata_idx {
                manager.deselect_file(idx).await;
            }
        }
    }

    // Clear DB flag after torrent deselection.
    {
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        queries::clear_in_library(&conn, id).map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Uninstall a game: back up saves, delete game files, free disk space.
#[tauri::command]
pub async fn uninstall_game(
    db_state: State<'_, DbState>,
    torrent_state: State<'_, TorrentState>,
    id: i64,
) -> Result<String, String> {
    let (game, data_dir) = {
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        let game = queries::fetch_game_by_id(&conn, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Game {} not found", id))?;
        let data_dir = queries::get_config(&conn, "data_dir")
            .map_err(|e| e.to_string())?
            .ok_or("Data directory not configured")?;
        (game, data_dir)
    };

    if !game.installed && !game.in_library {
        return Err(format!("{} is not installed", game.title));
    }

    let shortcode = game.shortcode.as_deref()
        .ok_or("Game has no shortcode")?
        .to_string();

    let source = game.torrent_source.as_deref().unwrap_or("eXoDOS");
    let inner_folder = collection_inner_folder(source);
    let game_prefix = collection_game_prefix(source);
    let torrent_root = collection_data_dir(&data_dir, source).join(inner_folder);

    // Get game name from bat filename for ZIP deletion
    let game_name = game.application_path.as_deref()
        .and_then(crate::commands::setup::game_name_from_app_path)
        .unwrap_or_else(|| game.title.clone());

    // Determine game directory
    // For EN:  <game_prefix>/<shortcode>/
    // For LP:  <game_prefix>/<lang_dir>/<shortcode>/
    let mut game_dir_candidates = vec![torrent_root.join(format!("{}/{}", game_prefix, shortcode))];
    for ld in LANG_DIRS {
        game_dir_candidates.push(torrent_root.join(format!("{}/{}/{}", game_prefix, ld, shortcode)));
    }

    let game_dir: Option<PathBuf> = game_dir_candidates.into_iter().find(|d| d.exists());

    let db_path = {
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        conn.path().map(PathBuf::from)
            .ok_or_else(|| "Cannot determine database path".to_string())?
    };

    let deleted_rels: Vec<String> = tauri::async_runtime::spawn_blocking(move || {
        if let Some(ref dir) = game_dir {
            if dir.exists() {
                // Back up the entire game directory (preserves saves, configs, etc.)
                let save_dir = torrent_root.join(format!("{}/!save/{}", game_prefix, shortcode));
                if save_dir.exists() {
                    let _ = std::fs::remove_dir_all(&save_dir);
                }
                // Rename is the fastest way to "back up" - atomic move
                if let Err(e) = std::fs::rename(dir, &save_dir) {
                    // Rename failed (cross-device?), fall back to copy + delete
                    log::warn!("Rename to save dir failed ({}), falling back to copy", e);
                    if let Err(e) = copy_dir_recursive(dir, &save_dir) {
                        log::error!("Failed to back up game directory '{}': {}", dir.display(), e);
                        // Don't delete the source if backup failed
                    } else {
                        let _ = std::fs::remove_dir_all(dir);
                    }
                }
                log::info!("Backed up saves to {}", save_dir.display());
            }
        }

        // Track which ZIPs actually got deleted (torrent-relative paths) so
        // the caller can reset piece bookkeeping in exactly the torrents
        // that tracked them.
        let mut zip_rels = vec![format!("{}/{}.zip", game_prefix, game_name)];
        for ld in LANG_DIRS {
            zip_rels.push(format!("{}/{}/{}.zip", game_prefix, ld, game_name));
        }
        let mut deleted_rels: Vec<String> = Vec::new();
        for rel in &zip_rels {
            let zip = torrent_root.join(rel);
            if zip.exists() && std::fs::remove_file(&zip).is_ok() {
                deleted_rels.push(rel.clone());
            }
        }

        if let Ok(conn) = db::open(&db_path) {
            if let Err(e) = queries::set_game_installed(&conn, id, false) {
                log::error!("Failed to update uninstall status: {}", e);
            }
            // Also clear in_library
            let _ = conn.execute(
                "UPDATE games SET in_library = 0 WHERE id = ?1",
                rusqlite::params![id],
            );
        } else {
            log::error!("Failed to open DB for uninstall update");
        }

        deleted_rels
    })
    .await
    .map_err(|e| e.to_string())?;

    // Deleted ZIPs' pieces are still marked "had" in librqbit's fastresume
    // state; a later re-download would report 100% instantly with no file on
    // disk (stuck-download loop). All collections overlay one root, so a
    // deleted ZIP may be tracked by a torrent OTHER than this game's source
    // (e.g. a GLP uninstall also removes the EN ZIP). Reset exactly the
    // torrents that tracked a deleted path.
    let managers: Vec<(String, std::sync::Arc<crate::torrent::manager::DownloadManager>)> = {
        let guard = torrent_state.0.read().await;
        guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };

    // Only deselect this game's shared GameData when no other variant still
    // wants it (mirrors cancel_download).
    let gamedata_drop: Option<usize> = match game.gamedata_torrent_index {
        Some(gd) => {
            let conn = db_state.0.lock().map_err(|e| e.to_string())?;
            let still_needed: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM games \
                     WHERE gamedata_torrent_index = ?1 AND id != ?2 AND in_library = 1",
                    rusqlite::params![gd, id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            if still_needed > 0 { None } else { Some(gd as usize) }
        }
        None => None,
    };

    for (col_id, mgr) in managers {
        let mut drop_indices: Vec<usize> = deleted_rels
            .iter()
            .filter_map(|rel| mgr.index().find_by_path(rel).map(|f| f.index))
            .collect();
        let is_source = col_id == source;
        if is_source {
            if let Some(gi) = game.game_torrent_index {
                let gi = gi as usize;
                if !drop_indices.contains(&gi) {
                    drop_indices.push(gi);
                }
            }
            if let Some(gd) = gamedata_drop {
                if !drop_indices.contains(&gd) {
                    drop_indices.push(gd);
                }
            }
        }

        let tracked_deleted = deleted_rels
            .iter()
            .any(|rel| mgr.index().find_by_path(rel).is_some());
        if tracked_deleted {
            // Disk state changed under this torrent - full invalidation.
            if let Err(e) = mgr.invalidate_after_file_delete(&drop_indices).await {
                log::warn!("Failed to reset {} torrent state after uninstall: {}", col_id, e);
            }
        } else if is_source {
            // Nothing deleted from this torrent's files; just drop the
            // selection so the re-add doesn't fetch the uninstalled game.
            for idx in drop_indices {
                mgr.deselect_file(idx).await;
            }
        }
    }

    Ok(format!("Uninstalled: {}", game.title))
}

/// Does this game's DOSBox config ask for MIDI music (MT-32 or General
/// MIDI)? Reads the bundled per-game conf; permissive on read failure so a
/// missing conf never blocks a download decision.
fn game_requests_midi(torrent_root: &std::path::Path, dosbox_conf: Option<&str>) -> bool {
    let Some(rel) = dosbox_conf else { return false };
    // DB paths mix separators ("eXo\eXoDOS\!dos\SQ1VGA/dosbox.conf").
    let rel = rel.replace('\\', "/");
    let Ok(text) = std::fs::read_to_string(torrent_root.join(rel)) else {
        return false;
    };
    let lower = text.to_ascii_lowercase();
    lower.contains("mididevice")
        || lower.contains("mt32.")
        || lower.contains("fluid.")
        || lower.contains("[mt32]")
        || lower.contains("[fluidsynth]")
}

/// Extract the mt32 subtree (MT-32/CM32L ROMs incl. rev0, SoundCanvas +
/// AWE64 soundfonts, ~54 MB) from util.zip into `<torrent_root>/eXo/mt32/`.
///
/// util.zip is a matryoshka: the payload sits in a nested EXTDOS.zip whose
/// top-level `mt32/` dir is what the game configs reference as `.\mt32\`.
/// The inner zip (467 MB uncompressed) is staged to a temp file rather than
/// RAM; the rest of it (Windows emulator builds) is never extracted.
fn extract_mt32_from_util_zip(
    util_zip: &std::path::Path,
    torrent_root: &std::path::Path,
) -> Result<usize, String> {
    let file = std::fs::File::open(util_zip).map_err(|e| e.to_string())?;
    let mut outer = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    let tmp_path = util_zip.with_extension("extdos_tmp");
    {
        let mut inner_entry = outer
            .by_name("EXTDOS.zip")
            .map_err(|e| format!("EXTDOS.zip not found inside util.zip: {}", e))?;
        let mut tmp = std::fs::File::create(&tmp_path).map_err(|e| e.to_string())?;
        std::io::copy(&mut inner_entry, &mut tmp).map_err(|e| e.to_string())?;
    }

    let result = (|| {
        let tmp = std::fs::File::open(&tmp_path).map_err(|e| e.to_string())?;
        let mut inner = zip::ZipArchive::new(tmp).map_err(|e| e.to_string())?;
        let dest_root = torrent_root.join("eXo");
        // mt32/ everywhere; on Windows also eXo's DOSBox ECE builds so
        // ECE-variant games run their intended emulator.
        let mut prefixes: Vec<&str> = vec!["mt32/"];
        if cfg!(windows) {
            prefixes.push("emulators/dosbox/ece4230/");
            prefixes.push("emulators/dosbox/ece4460/");
        }
        let mut extracted = 0usize;
        for i in 0..inner.len() {
            let mut entry = inner.by_index(i).map_err(|e| e.to_string())?;
            let name = entry.name().replace('\\', "/");
            let lower = name.to_ascii_lowercase();
            if !prefixes.iter().any(|p| lower.starts_with(p))
                || name.contains("..")
                || entry.is_dir()
            {
                continue;
            }
            let out_path = dest_root.join(&name);
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            extracted += 1;
        }
        if extracted == 0 {
            return Err("no mt32/ entries found in EXTDOS.zip".to_string());
        }
        Ok(extracted)
    })();

    let _ = std::fs::remove_file(&tmp_path);
    result
}

/// Watch util.zip until it finishes downloading, then extract the mt32
/// payload. Runs as its own task because the frontend only polls progress
/// while a GAME download is active - util.zip (~630 MB) routinely finishes
/// long after the 8 MB game that triggered it, with nobody left polling.
fn spawn_mt32_extraction_watcher(mgr: std::sync::Arc<crate::torrent::manager::DownloadManager>, util_index: usize) {
    tauri::async_runtime::spawn(async move {
        let torrent_root = mgr.torrent_root();
        let mt32_dir = torrent_root.join("eXo/mt32");
        let ece_dir = torrent_root.join("eXo/emulators/dosbox/ece4230");
        // Generous ceiling: 6 h at 10 s per check for slow swarms.
        for _ in 0..2160 {
            if mt32_dir.exists() && (!cfg!(windows) || ece_dir.exists()) {
                return; // someone else finished the job
            }
            if mgr.is_file_complete(util_index).await {
                let Some(zip_path) = mgr.file_output_path(util_index) else {
                    return;
                };
                let lock = zip_path.with_extension("mt32_extracted");
                if !zip_path.exists() || lock.exists() {
                    return;
                }
                let _ = std::fs::write(&lock, "");
                let root = torrent_root.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    match extract_mt32_from_util_zip(&zip_path, &root) {
                        Ok(n) => log::info!("Extracted {} MT-32/soundfont files from util.zip", n),
                        Err(e) => {
                            let _ = std::fs::remove_file(&lock);
                            log::error!("Failed to extract mt32 from util.zip: {}", e);
                        }
                    }
                })
                .await;
                return;
            }
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        }
        log::warn!("mt32 extraction watcher timed out waiting for util.zip");
    });
}

/// Create a directory link (symlink on Unix, junction on Windows - junctions
/// need no admin rights or developer mode).
#[cfg(unix)]
fn link_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}
#[cfg(windows)]
fn link_dir(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    junction::create(src, dst)
}

/// Build the per-launch overlay root for an LP game: a staging dir whose
/// `<shortcode>` entry links to the LP game dir, so the EN config's autoexec
/// ("mount c .\eXoDOS\", "cd <shortcode>", launch) runs unmodified against
/// the LP files. Any other eXoDOS-root entries the autoexec references
/// (shared CD-image folders etc.) get pass-through links to the real tree.
/// Rebuilt from scratch on every launch; contains only links, no data.
fn build_lp_overlay(
    working_dir: &std::path::Path,
    game_folder: &str,
    shortcode: &str,
    lang_dir: &str,
    lp_game_dir: &std::path::Path,
    en_conf: &str,
) -> Result<PathBuf, String> {
    let staging = working_dir
        .join(".exodium_lp")
        .join(format!("{}_{}", lang_dir.trim_start_matches('!'), shortcode));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .map_err(|e| format!("clearing {}: {}", staging.display(), e))?;
    }
    std::fs::create_dir_all(&staging).map_err(|e| format!("creating {}: {}", staging.display(), e))?;
    link_dir(lp_game_dir, &staging.join(shortcode))
        .map_err(|e| format!("linking {}: {}", shortcode, e))?;

    // Pass-through links for other referenced root entries.
    let real_root = working_dir.join(game_folder);
    let needle = format!("{}\\", game_folder);
    let autoexec = en_conf.split("[autoexec]").nth(1).unwrap_or("");
    for (idx, _) in autoexec.match_indices(&needle) {
        let rest = &autoexec[idx + needle.len()..];
        let entry: String = rest
            .chars()
            .take_while(|c| !"\\/\" \t\r\n".contains(*c))
            .collect();
        if entry.is_empty() || entry.eq_ignore_ascii_case(shortcode) {
            continue;
        }
        let dst = staging.join(&entry);
        let src = real_root.join(&entry);
        if !dst.exists() && src.exists() {
            if let Err(e) = link_dir(&src, &dst) {
                log::warn!("LP overlay: pass-through link {} failed: {}", entry, e);
            }
        }
    }
    Ok(staging)
}

/// Can the EN autoexec run against the LP dir via the overlay? Simulates the
/// cd chain (a root-level `cd <shortcode>` lands in the LP game dir) and
/// requires the launch command's program to exist at the resulting location.
/// LP variants occasionally restructure the game (renamed executable,
/// different subdirs) - those fall back to the generated-autoexec strategy.
fn lp_autoexec_compatible(
    en_conf: &str,
    shortcode: &str,
    lp_game_dir: &std::path::Path,
    real_root: &std::path::Path,
) -> bool {
    let Some(autoexec) = en_conf.split("[autoexec]").nth(1) else {
        return false;
    };
    // cwd: None = mount root (the overlay staging dir).
    let mut cwd: Option<PathBuf> = None;
    for line in autoexec.lines() {
        let t = line.trim();
        let t = t.strip_prefix('@').unwrap_or(t).trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let lower = t.to_ascii_lowercase();

        if lower == "cd" || lower == "cd." || lower == "cd.." {
            continue;
        }
        let cd_target = if let Some(r) = lower.strip_prefix("cd ") {
            Some(r)
        } else if let Some(r) = lower.strip_prefix("cd\\") {
            // "cd\FOO" is an absolute path from the mount root.
            cwd = None;
            Some(r)
        } else {
            None
        };
        if let Some(target) = cd_target {
            let target = target.trim().trim_matches('"');
            if target.is_empty() || target == "\\" || target == "/" || target == ".." {
                cwd = None;
                continue;
            }
            let next = match &cwd {
                None => {
                    if target.eq_ignore_ascii_case(shortcode) {
                        lp_game_dir.to_path_buf()
                    } else {
                        // Root-level cd into a non-game entry resolves
                        // through a pass-through link to the real tree.
                        real_root.join(target)
                    }
                }
                Some(dir) => dir.join(target),
            };
            if !next.exists() {
                log::info!(
                    "LP launch: EN autoexec cd target '{}' missing under LP layout",
                    target
                );
                return false;
            }
            cwd = Some(next);
            continue;
        }

        // Housekeeping lines that never launch anything.
        let is_drive_switch = lower.len() == 2
            && lower.as_bytes()[1] == b':'
            && lower.as_bytes()[0].is_ascii_alphabetic();
        if is_drive_switch
            || ["mount ", "imgmount ", "echo ", "rem ", "set ", "config "]
                .iter()
                .any(|p| lower.starts_with(p))
            || ["cls", "exit", "pause", "echo", "echo."].contains(&lower.as_str())
        {
            continue;
        }

        // First real command = the launch line. Verify its program exists at
        // the simulated cwd; unrecognizable forms (boot images, drive-letter
        // paths) are trusted - the EN config knows better than any heuristic.
        let base = t
            .strip_prefix("call ")
            .or_else(|| t.strip_prefix("CALL "))
            .or_else(|| t.strip_prefix("loadfix "))
            .unwrap_or(t);
        let base = base.split_whitespace().next().unwrap_or(base);
        if base.contains(':') || base.contains('\\') || base.contains('/') {
            return true;
        }
        let dir = match &cwd {
            Some(d) => d.clone(),
            None => return true, // command at mount root - rare, trust it
        };
        let base_lower = base.to_ascii_lowercase();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                let stem = name.rsplitn(2, '.').last().unwrap_or(&name);
                if stem == base_lower || name == base_lower {
                    return true;
                }
            }
        }
        log::info!(
            "LP launch: EN launch command '{}' not present in {} - falling back",
            base,
            dir.display()
        );
        return false;
    }
    // No launch command at all (fully commented autoexec): the overlay still
    // works - the caller appends a find_lp_launch command.
    true
}

/// Patch a DOSBox config file: convert Windows-style relative paths to absolute Linux paths.
/// The eXoDOS configs use `.\eXoDOS\game\` which doesn't work on Linux.
///
/// For LP games, `lp_info` provides the shortcode, language dir, game_folder (the second
/// component of game_prefix, e.g. "eXoDOS"), and the resolved LP game directory path.
/// The EN config runs VERBATIM against an overlay mount whose `<shortcode>` entry links
/// to the LP game dir - preserving eXo's authored launch commands, imgmounts, and
/// utilities. Only when the LP variant's layout is incompatible with the EN autoexec
/// does it fall back to a generated autoexec.
fn patch_dosbox_conf(
    conf_path: &std::path::Path,
    working_dir: &std::path::Path,
    lp_info: Option<(&str, &str, &str, &std::path::Path)>, // (shortcode, lang_dir, game_folder, lp_game_dir)
    // false when launching under DOSBox ECE, which understands the original
    // ECE [midi] keys natively - translating them would break its MIDI.
    translate_for_staging: bool,
) -> Result<PathBuf, String> {
    let content = std::fs::read_to_string(conf_path)
        .map_err(|e| format!("Failed to read {}: {}", conf_path.display(), e))?;

    let abs_prefix = format!("{}/", working_dir.to_string_lossy());

    let patched = if let Some((shortcode, lang_dir, game_folder, game_dir)) = lp_info {
        // Strategy 1: overlay mount. The EN autoexec is ground truth authored
        // by eXo; the only real difference for an LP install is WHERE the
        // game files live. Point every eXoDOS-root reference at a staging dir
        // whose <shortcode> entry links to the LP game dir and run the config
        // as written. This also shadows an installed EN variant of the same
        // game - the link always wins.
        let real_root = working_dir.join(game_folder);
        let overlay = if game_dir.exists()
            && lp_autoexec_compatible(&content, shortcode, game_dir, &real_root)
        {
            build_lp_overlay(working_dir, game_folder, shortcode, lang_dir, game_dir, &content)
                .map_err(|e| log::warn!("LP overlay build failed for {}: {}", shortcode, e))
                .ok()
        } else {
            None
        };

        if let Some(staging) = overlay {
            log::info!(
                "LP launch: overlay mount for {} ({} -> {})",
                shortcode,
                staging.display(),
                game_dir.display()
            );
            let staging_fwd = staging.to_string_lossy().replace('\\', "/");
            let mut result = content
                // Route eXoDOS-root references through the overlay first...
                .replace(&format!(".\\{}\\", game_folder), &format!("{}/", staging_fwd))
                .replace(&format!(".\\{}", game_folder), &staging_fwd)
                // ...then the usual absolute-path + slash rewriting for the rest.
                .replace(".\\", &abs_prefix)
                .replace('\\', "/");

            // If autoexec has no actual launch command (e.g., all commented out with #),
            // append one found by inspecting the LP game directory.
            if !autoexec_has_launch_cmd(&result) {
                log::info!("LP launch: autoexec has no launch cmd, appending find_lp_launch for {}", shortcode);
                if let Some((subdir, cmd)) = find_lp_launch(game_dir, Some(&content)) {
                    // Strip any trailing `exit` so our appended commands aren't skipped.
                    let trimmed = result.trim_end();
                    if trimmed.to_ascii_lowercase().ends_with("exit") {
                        result.truncate(trimmed.len() - "exit".len());
                        result.push('\n');
                    }
                    // The generated command runs from the mount root; enter the
                    // game dir (via the overlay link) first.
                    result.push_str(&format!("cd {}\n", shortcode));
                    if !subdir.is_empty() {
                        result.push_str(&format!("cd {}\n", subdir));
                    }
                    result.push_str("cls\n");
                    result.push_str(&format!("{}\n", cmd));
                    result.push_str("exit\n");
                }
            }
            result
        } else {
            // Strategy 2: Different directory structure - generate custom autoexec
            log::info!("LP launch: generating custom autoexec for {} (redirected path not found)", shortcode);
            let settings = content
                .split("[autoexec]")
                .next()
                .unwrap_or(&content);

            let mut patched = settings
                .replace(".\\", &abs_prefix)
                .replace('\\', "/");

            let game_dir_abs = game_dir.to_string_lossy();
            patched.push_str("[autoexec]\n");
            patched.push_str(&format!("@mount c \"{}\"\n", game_dir_abs));
            patched.push_str("c:\n");

            // Find the game subdirectory and launch command
            if let Some((subdir, cmd)) = find_lp_launch(game_dir, Some(&content)) {
                if !subdir.is_empty() {
                    patched.push_str(&format!("cd {}\n", subdir));
                }
                patched.push_str("cls\n");
                patched.push_str(&format!("{}\n", cmd));
            }
            patched.push_str("exit\n");
            patched
        }
    } else {
        // EN game: simple path replacement
        content
            .replace(".\\", &abs_prefix)
            .replace('\\', "/")
    };

    let patched = if translate_for_staging {
        translate_midi_for_staging(&patched)
    } else {
        patched
    };

    let patched_path = working_dir.join(".exodium_launch.conf");
    std::fs::write(&patched_path, &patched)
        .map_err(|e| format!("Failed to write patched config: {}", e))?;

    log::debug!("Patched config written to {}", patched_path.display());

    Ok(patched_path)
}

/// Translate DOSBox-ECE MIDI settings to DOSBox Staging equivalents.
///
/// ~1,500 eXoDOS configs carry ECE-style dotted keys in [midi]
/// (`mt32.romdir`, `fluid.soundfont`, `fluid.*`) that Staging silently
/// ignores - MT-32 and General-MIDI games then play with wrong or no music.
/// Staging expects the same settings in dedicated [mt32] / [fluidsynth]
/// sections, so: capture the ECE values, drop the dotted keys, and append
/// the Staging sections (unless the config already has them - the ~750
/// Staging-authored eXoDOS configs pass through unchanged). Also maps
/// `mididevice = default` (ECE) to Staging's `auto`.
///
/// Runs after the path rewriting in patch_dosbox_conf, so captured values
/// like `.\mt32` are already absolute forward-slash paths.
fn translate_midi_for_staging(conf: &str) -> String {
    let lower = conf.to_ascii_lowercase();
    let has_ece_keys = lower.contains("mt32.") || lower.contains("fluid.");
    let has_default_device = lower.contains("mididevice");
    if !has_ece_keys && !has_default_device {
        return conf.to_string();
    }
    let has_mt32_section = lower.lines().any(|l| l.trim() == "[mt32]");
    let has_fluid_section = lower.lines().any(|l| l.trim() == "[fluidsynth]");

    let mut romdir: Option<String> = None;
    let mut soundfont: Option<String> = None;
    let mut out: Vec<String> = Vec::with_capacity(conf.lines().count() + 6);
    let mut section = String::new();

    for line in conf.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed.to_ascii_lowercase();
            out.push(line.to_string());
            continue;
        }
        if section == "[midi]" && !trimmed.starts_with('#') {
            if let Some((key, value)) = trimmed.split_once('=') {
                let key = key.trim().to_ascii_lowercase();
                let value = value.trim();
                if key == "mt32.romdir" {
                    romdir = Some(value.to_string());
                    continue; // drop the ECE key
                }
                if key == "fluid.soundfont" {
                    soundfont = Some(value.to_string());
                    continue;
                }
                if key.starts_with("mt32.") || key.starts_with("fluid.") {
                    continue; // ECE tuning keys with no Staging equivalent
                }
                if key == "mididevice" && value.eq_ignore_ascii_case("default") {
                    out.push("mididevice = auto".to_string());
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }

    if !has_mt32_section {
        if let Some(dir) = romdir {
            out.push(String::new());
            out.push("[mt32]".to_string());
            out.push(format!("romdir = {}", dir));
            if !std::path::Path::new(&dir).exists() {
                log::warn!(
                    "MT-32 ROM dir {} not on disk yet - music will be missing until \
                     the DOSBox support files finish downloading",
                    dir
                );
            }
        }
    }
    if !has_fluid_section {
        if let Some(sf) = soundfont {
            out.push(String::new());
            out.push("[fluidsynth]".to_string());
            out.push(format!("soundfont = {}", sf));
            if !std::path::Path::new(&sf).exists() {
                log::warn!(
                    "Soundfont {} not on disk yet - General MIDI music will be missing \
                     until the DOSBox support files finish downloading",
                    sf
                );
            }
        }
    }

    let mut result = out.join("\n");
    result.push('\n');
    result
}

/// Find the launch command for an LP game by inspecting its directory.
/// Prefers the launch command named by the EN config's autoexec (when given),
/// then parses run.bat to extract the actual game executable, since run.bat
/// itself is a LaunchBox-specific menu script not suitable for DOSBox autoexec.
/// Returns (subdir, command) if found.
fn find_lp_launch(game_dir: &std::path::Path, en_conf: Option<&str>) -> Option<(String, String)> {
    // Strategy 0: the EN autoexec names the real launcher ("cd cobmiss" then
    // "@cm") - by far the strongest signal, and the only one that works for
    // games with a bare root-level EXE and no .bat (e.g. Cobra Mission ES:
    // CM.EXE + INSTALL.EXE, nothing else runnable). Use the first
    // non-housekeeping command if the referenced program exists in the LP dir.
    if let Some(autoexec) = en_conf.and_then(|c| c.split("[autoexec]").nth(1)) {
        for line in autoexec.lines() {
            let t = line.trim();
            let t = t.strip_prefix('@').unwrap_or(t).trim();
            let t = t
                .strip_prefix("call ")
                .or_else(|| t.strip_prefix("CALL "))
                .unwrap_or(t)
                .trim();
            if t.is_empty() {
                continue;
            }
            let lower = t.to_ascii_lowercase();
            let is_drive_switch = lower.len() == 2
                && lower.as_bytes()[1] == b':'
                && lower.as_bytes()[0].is_ascii_alphabetic();
            let is_housekeeping = is_drive_switch
                || lower.starts_with('#')
                || ["mount ", "imgmount ", "echo ", "rem ", "cd ", "cd\\", "set "]
                    .iter()
                    .any(|p| lower.starts_with(p))
                || ["cls", "cd", "exit", "pause", "echo", "echo."]
                    .contains(&lower.as_str());
            if is_housekeeping {
                continue;
            }
            let base = t.split_whitespace().next().unwrap_or(t);
            let base_lower = base.to_ascii_lowercase();
            if let Ok(entries) = std::fs::read_dir(game_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let name_lower = entry.file_name().to_string_lossy().to_ascii_lowercase();
                    let runnable = name_lower.ends_with(".exe")
                        || name_lower.ends_with(".com")
                        || name_lower.ends_with(".bat");
                    let stem = name_lower.rsplitn(2, '.').last().unwrap_or(&name_lower);
                    if runnable && (stem == base_lower || name_lower == base_lower) {
                        log::info!(
                            "LP launch: using EN autoexec command '{}' (found {})",
                            t,
                            entry.file_name().to_string_lossy()
                        );
                        return Some((String::new(), t.to_string()));
                    }
                }
            }
            // Only the FIRST real command is the launch line; later lines
            // (cleanup, exit chains) must not be mistaken for it.
            break;
        }
    }

    let mut search_dirs: Vec<(String, std::path::PathBuf)> =
        vec![("".to_string(), game_dir.to_path_buf())];

    if let Ok(entries) = std::fs::read_dir(game_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.path().is_dir() {
                search_dirs.push((
                    entry.file_name().to_string_lossy().to_string(),
                    entry.path(),
                ));
            }
        }
    }

    // Strategy 1: Parse run.bat to find the real executable
    for (subdir, dir) in &search_dirs {
        let run_bat = dir.join("run.bat");
        if let Ok(content) = std::fs::read_to_string(&run_bat) {
            // Look for "@call <program>" or just "<program>" lines that reference
            // a .com/.exe/.bat that exists in the directory
            for line in content.lines() {
                let trimmed = line.trim();
                let cmd = trimmed
                    .strip_prefix("@call ")
                    .or_else(|| trimmed.strip_prefix("@CALL "))
                    .or_else(|| trimmed.strip_prefix("@"))
                    .unwrap_or(trimmed);
                let cmd = cmd.trim();
                let cmd_lower = cmd.to_ascii_lowercase();

                // Skip control flow, echo, copy, config, choice, labels, etc.
                let skip_prefixes = [
                    ":", "echo", "cls", "copy", "config", "choice",
                    "if ", "goto", "exit", "rem ", "set ", "pause",
                ];
                if cmd.is_empty() || skip_prefixes.iter().any(|p| cmd_lower.starts_with(p)) {
                    continue;
                }

                // Check if this command corresponds to an actual file in the game dir
                let base = cmd.split_whitespace().next().unwrap_or(cmd);
                // Search directory for a case-insensitive match
                if let Ok(entries) = std::fs::read_dir(dir) {
                    let base_lower = base.to_ascii_lowercase();
                    for entry in entries.filter_map(|e| e.ok()) {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let name_lower = name.to_ascii_lowercase();
                        let stem = name_lower.rsplitn(2, '.').last().unwrap_or(&name_lower);
                        if stem == base_lower || name_lower == base_lower {
                            log::info!("LP launch: found '{}' via run.bat in {}", base, subdir);
                            return Some((subdir.clone(), base.to_string()));
                        }
                    }
                }
            }
        }
    }

    // Strategy 2: Look for any .bat file that calls an exe/com (skip known utility names).
    // Returns the .bat itself as the command so all its steps run in sequence.
    const SKIP_BAT_STEMS: &[&str] = &[
        "anleit", "readme", "install", "setup", "help", "manual",
        "problem", "config", "uninstal", "uninst",
    ];
    for (subdir, dir) in &search_dirs {
        let dir_stem = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let mut candidates: Vec<String> = if let Ok(entries) = std::fs::read_dir(dir) {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    name.ends_with(".bat")
                        && name != "run.bat"
                        && !SKIP_BAT_STEMS.iter().any(|s| name.starts_with(s))
                })
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        } else {
            vec![]
        };

        // Prefer .bat whose stem matches the directory name
        candidates.sort_by_key(|b| {
            let stem = b.rsplitn(2, '.').last().unwrap_or(b).to_lowercase();
            usize::from(stem != dir_stem)
        });

        for bat in &candidates {
            let bat_path = dir.join(bat);
            if let Ok(content) = std::fs::read_to_string(&bat_path) {
                let has_exe_call = content.lines().any(|line| {
                    let l = line.trim().to_ascii_lowercase();
                    !l.is_empty()
                        && !l.starts_with(':')
                        && !l.starts_with("rem ")
                        && (l.contains(".exe") || l.contains(".com"))
                });
                if has_exe_call {
                    log::info!("LP launch: found .bat launcher '{}' in '{}'", bat, subdir);
                    return Some((subdir.clone(), bat.clone()));
                }
            }
        }
    }

    // Strategy 3: Look for a .com file (more likely to be a DOS game than .exe)
    for (subdir, dir) in &search_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.ends_with(".com") && !name.contains("mouse") {
                    return Some((
                        subdir.clone(),
                        entry.file_name().to_string_lossy().to_string(),
                    ));
                }
            }
        }
    }

    // Strategy 4: Look for a .exe in subdirectories, then the game dir root
    // (skip utilities and installers). Subdirs first to keep the historical
    // preference; the root pass catches games like Cobra Mission ES whose
    // only executable sits at the top level.
    const SKIP_EXE_STEMS: &[&str] = &[
        "install", "setup", "uninst", "config", "cdtest", "showtext",
        // DOS/4GW and protected-mode extenders - not the game itself
        "rtm", "dos4gw", "dpmi", "cwsdpmi",
    ];
    let subdirs_then_root = search_dirs
        .iter()
        .filter(|(s, _)| !s.is_empty())
        .chain(search_dirs.iter().filter(|(s, _)| s.is_empty()));
    for (subdir, dir) in subdirs_then_root {
        let dir_stem = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let mut exes: Vec<String> = if let Ok(entries) = std::fs::read_dir(dir) {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let name = e.file_name().to_string_lossy().to_lowercase();
                    name.ends_with(".exe")
                        && !SKIP_EXE_STEMS.iter().any(|s| name.starts_with(s))
                })
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        } else {
            vec![]
        };

        // Prefer exe whose stem matches the directory name
        exes.sort_by_key(|e| {
            let stem = e.rsplitn(2, '.').last().unwrap_or(e).to_lowercase();
            usize::from(stem != dir_stem)
        });

        if let Some(exe) = exes.first() {
            log::info!("LP launch: found .exe '{}' in '{}'", exe, subdir);
            return Some((subdir.clone(), exe.clone()));
        }
    }

    None
}

/// Returns true if the [autoexec] section of a dosbox conf contains at least one
/// line that looks like an actual game launch command (not just mounts, drive switches,
/// comments, or housekeeping).
fn autoexec_has_launch_cmd(conf: &str) -> bool {
    let autoexec = match conf.split("[autoexec]").nth(1) {
        Some(s) => s,
        None => return false,
    };
    autoexec.lines().any(|line| {
        let l = line.trim().to_ascii_lowercase();
        if l.is_empty() || l.starts_with('#') || l.starts_with("rem ") {
            return false;
        }
        // Drive-switch: single letter followed by colon (a: through z:)
        let is_drive_switch = l.len() >= 2
            && l.as_bytes()[1] == b':'
            && l.as_bytes()[0].is_ascii_alphabetic();
        if is_drive_switch {
            return false;
        }
        const NON_LAUNCH: &[&str] = &[
            "@echo", "@exit", "echo ", "mount ", "imgmount", "exit", "cls",
        ];
        !NON_LAUNCH.iter().any(|p| l.starts_with(p))
    })
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("Failed to create {}: {}", dst.display(), e))?;
    for entry in walkdir::WalkDir::new(src).into_iter().filter_map(|e| e.ok()) {
        let rel = entry.path().strip_prefix(src).unwrap();
        let target = dst.join(rel);
        if entry.path().is_dir() {
            if let Err(e) = std::fs::create_dir_all(&target) {
                log::warn!("Failed to create dir {}: {}", target.display(), e);
            }
        } else if let Err(e) = std::fs::copy(entry.path(), &target) {
            log::warn!("Failed to copy {} -> {}: {}", entry.path().display(), target.display(), e);
        }
    }
    Ok(())
}

/// Extract a game ZIP in place, then restore saves from !save/ if available.
fn extract_game_zip(zip_path: &std::path::Path, dest: &std::path::Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    // Get the top-level directory name from the ZIP (the shortcode)
    let shortcode = archive.by_index(0).ok()
        .and_then(|e| e.name().split('/').next().map(|s| s.to_string()));

    archive.extract(dest).map_err(|e| e.to_string())?;
    log::info!("Extracted: {} -> {}", zip_path.display(), dest.display());

    // Restore saves if available
    // Saves are at !save/<shortcode>/ which could be:
    // - In dest itself (e.g., dest = .../eXo/eXoDOS/, saves at .../eXo/eXoDOS/!save/SQ5/)
    // - Or relative to the game dir's grandparent for LP games
    if let Some(sc) = shortcode {
        let game_dir = dest.join(&sc);
        // Search for !save in dest and parent directories
        let save_candidates = [
            dest.join(format!("!save/{}", sc)),
            dest.parent().map(|p| p.join(format!("!save/{}", sc))).unwrap_or_default(),
        ];
        for save_dir in &save_candidates {
            if save_dir.exists() && game_dir.exists() {
                log::info!("Restoring saves from {}", save_dir.display());
                let _ = copy_dir_recursive(save_dir, &game_dir);
                break;
            }
        }
    }

    Ok(())
}

/// Resolve the DOSBox Staging binary path.
/// Tauri's `externalBin` places sidecars at different locations per platform:
///  - macOS: Exodium.app/Contents/MacOS/dosbox-staging (next to the main binary)
///  - Windows: <install_dir>/dosbox-staging.exe (next to the main .exe)
///  - Linux (AppImage/deb): resources/dosbox-staging (inside the resource dir)
///
/// So we check `current_exe().parent()` AND `resource_dir()`, then fall back to PATH.
fn resolve_dosbox(app: &AppHandle) -> PathBuf {
    use tauri::Manager;
    let bin = if cfg!(windows) { "dosbox-staging.exe" } else { "dosbox-staging" };

    // 1. resource_dir/dosbox-bin/ - the canonical location since v0.6.6 on
    //    Windows, where the .exe MUST live alongside its bundled DLLs
    //    (SDL2.dll, vcruntime140.dll, …) plus DOSBox's `resources/` codepage
    //    folder for Windows DLL search to find them. On macOS/Linux this
    //    directory only contains a `.placeholder`, so the lookup falls
    //    through to the externalBin location below.
    //
    //    In dev mode resource_dir is src-tauri/, and the staged bundle lives
    //    one level deeper at src-tauri/resources/dosbox-bin/ (only flattened
    //    to <resource_dir>/dosbox-bin/ at bundle time). Check both so dev on
    //    Windows finds the DLL-adjacent .exe instead of falling through to
    //    the bare externalBin in binaries/, which would fail with missing-DLL
    //    errors when DOSBox tries to start.
    // Subdirs to search under resource_dir, in priority order. Bundled layout
    // (production) flattens to `dosbox-bin/`; dev layout keeps the staged
    // `resources/dosbox-bin/` source path. Update both if the bundle config
    // moves the binary directory.
    const DOSBOX_RES_DIRS: &[&str] = &["dosbox-bin", "resources/dosbox-bin"];
    if let Ok(res_dir) = app.path().resource_dir() {
        for sub in DOSBOX_RES_DIRS {
            let dbs_in_res = res_dir.join(sub).join(bin);
            if dbs_in_res.exists() {
                log::info!("Using bundled DOSBox (resource bin dir): {}", dbs_in_res.display());
                return dbs_in_res;
            }
        }
    }

    // 2. Next to the main executable (macOS Contents/MacOS/, Linux install dir).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(bin);
            if candidate.exists() {
                log::info!("Using bundled DOSBox (exe dir): {}", candidate.display());
                return candidate;
            }
        }
    }

    // 3. Inside resource_dir directly (legacy packaging layouts).
    if let Ok(res_dir) = app.path().resource_dir() {
        let prod = res_dir.join(bin);
        if prod.exists() {
            log::info!("Using bundled DOSBox (resource dir): {}", prod.display());
            return prod;
        }

        // 4. Dev mode (pnpm tauri dev): resource_dir is src-tauri/; binary is in binaries/
        //    named with the Rust target triple, e.g. dosbox-staging-aarch64-apple-darwin.
        let binaries_dir = res_dir.join("binaries");
        if let Ok(entries) = std::fs::read_dir(&binaries_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("dosbox-staging") {
                    log::info!("Using bundled DOSBox (dev): {}", entry.path().display());
                    return entry.path();
                }
            }
        }
    }

    log::warn!("Bundled DOSBox not found, falling back to system PATH");
    PathBuf::from(bin)
}

/// Install DOSBox Staging glshaders into the user config dir if missing.
///
/// DOSBox aborts at startup with "Fallback shader 'interpolation/bilinear'
/// not found" unless it finds glshaders in the user config dir. The shader
/// pack is bundled as a Tauri resource (`bundle.resources` maps
/// `resources/dosbox-glshaders` → `dosbox-glshaders` inside resource_dir).
/// Here we copy it to the user config dir on first launch; subsequent
/// launches see the dir already exists and no-op.
///
/// On macOS this is only needed when the user opts into CRT shaders -
/// otherwise launch_game writes `output = texture` (SDL renderer, no
/// shader pipeline) which sidesteps the startup check entirely.
fn ensure_dosbox_shaders(app: &AppHandle) {
    use tauri::Manager;

    let user_shader_dir: Option<PathBuf> = if cfg!(target_os = "linux") {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config")))
            .map(|b| b.join("dosbox").join("glshaders"))
    } else if cfg!(target_os = "windows") {
        std::env::var("LOCALAPPDATA")
            .ok()
            .map(|p| PathBuf::from(p).join("DOSBox").join("glshaders"))
    } else if cfg!(target_os = "macos") {
        std::env::var("HOME")
            .ok()
            .map(|h| PathBuf::from(h).join("Library").join("Preferences").join("DOSBox").join("glshaders"))
    } else {
        None
    };

    let Some(user_shader_dir) = user_shader_dir else {
        log::warn!("Could not determine DOSBox user config dir; shaders not installed");
        return;
    };

    if user_shader_dir.is_dir() {
        return;
    }

    let res_dir = match app.path().resource_dir() {
        Ok(d) => d,
        Err(e) => {
            log::warn!("resource_dir() failed while installing DOSBox shaders: {}", e);
            return;
        }
    };
    let bundled = res_dir.join("dosbox-glshaders");
    if !bundled.is_dir() {
        log::debug!("No bundled DOSBox shaders at {}", bundled.display());
        return;
    }

    if let Some(parent) = user_shader_dir.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::warn!("Failed to create DOSBox config parent dir: {}", e);
            return;
        }
    }

    if let Err(e) = copy_dir_recursive(&bundled, &user_shader_dir) {
        log::warn!("Failed to install DOSBox shaders: {}", e);
    } else {
        log::info!("Installed DOSBox shaders to {}", user_shader_dir.display());
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GameSettings {
    pub glshader: Option<String>,
    pub fullscreen: Option<String>,
    pub cycles: Option<String>,
    pub custom_conf: Option<String>,
}

#[tauri::command]
pub fn get_game_settings(state: State<DbState>, id: i64) -> Result<GameSettings, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let cfg = queries::get_all_game_config(&conn, id).map_err(|e| e.to_string())?;
    Ok(GameSettings {
        glshader: cfg.get("glshader").cloned(),
        fullscreen: cfg.get("fullscreen").cloned(),
        cycles: cfg.get("cycles").cloned(),
        custom_conf: cfg.get("custom_conf").cloned(),
    })
}

#[tauri::command]
pub fn set_game_settings(
    state: State<DbState>,
    id: i64,
    glshader: Option<String>,
    fullscreen: Option<String>,
    cycles: Option<String>,
    custom_conf: Option<String>,
) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    // For each key: Some(value) = set, None = delete (inherit global)
    let pairs: &[(&str, &Option<String>)] = &[
        ("glshader", &glshader),
        ("fullscreen", &fullscreen),
        ("cycles", &cycles),
        ("custom_conf", &custom_conf),
    ];
    for (key, val) in pairs {
        match val {
            Some(v) if !v.is_empty() => {
                queries::set_game_config(&conn, id, key, v).map_err(|e| e.to_string())?;
            }
            _ => {
                queries::delete_game_config(&conn, id, key).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_recently_played(state: State<DbState>, limit: Option<usize>) -> Result<Vec<Game>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    queries::fetch_recently_played(&conn, limit.unwrap_or(12)).map_err(|e| e.to_string())
}

/// Launch a downloaded game via DOSBox Staging.
#[tauri::command]
pub fn launch_game(app: AppHandle, db_state: State<DbState>, id: i64) -> Result<String, String> {
    // Read everything we need from the DB and drop the lock before the heavy
    // DOSBox path resolution + process spawning below.
    let (game, data_dir, crt_auto_enabled, fullscreen_enabled, per_game_config) = {
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        let game = queries::fetch_game_by_id(&conn, id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Game with id {} not found", id))?;
        let data_dir = queries::get_config(&conn, "data_dir")
            .map_err(|e| e.to_string())?
            .ok_or("Data directory not configured. Run setup first.")?;
        let global_glshader = queries::get_config(&conn, "global_glshader")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "crt-auto".to_string());
        let default_fullscreen = queries::get_config(&conn, "default_fullscreen")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| "window".to_string());
        let per_game_config = queries::get_all_game_config(&conn, id).map_err(|e| e.to_string())?;
        // Record the launch timestamp for "Recently Played" shelf.
        if let Err(e) = queries::set_last_played(&conn, id) {
            log::warn!("Failed to update last_played for {}: {}", game.title, e);
        }
        (game, data_dir, global_glshader == "crt-auto", default_fullscreen == "fullscreen", per_game_config)
    }; // lock dropped here - before path resolution + DOSBox spawning

    if !game.installed {
        return Err(format!("{} is not installed. Download it first.", game.title));
    }

    let dosbox_conf = game
        .dosbox_conf
        .as_deref()
        .ok_or_else(|| {
            let msg = format!("Game '{}' (id={}, lang={}, shortcode={:?}) has no DOSBox config path",
                game.title, id, game.language, game.shortcode);
            log::error!("launch_game: {}", msg);
            msg
        })?;

    // Normalize Windows backslashes
    let dosbox_conf = dosbox_conf.replace('\\', "/");

    // Each collection has its own subdirectory (except eXoDOS which is at the root).
    // Layout:  <data_dir>/<inner_folder>/           - for eXoDOS
    //          <data_dir>/<col_id>/<inner_folder>/  - for sub-collections
    let source = game.torrent_source.as_deref().unwrap_or("eXoDOS");
    let main_inner = collection_inner_folder("eXoDOS");
    let src_inner = collection_inner_folder(source);
    let src_game_prefix = collection_game_prefix(source);
    let main_torrent_root = collection_data_dir(&data_dir, "eXoDOS").join(main_inner);
    let torrent_root = collection_data_dir(&data_dir, source).join(src_inner);
    // working_dir is the first path component of game_prefix (e.g. "eXo")
    let working_dir_name = src_game_prefix.split('/').next().unwrap_or("eXo");
    let mut working_dir = torrent_root.join(working_dir_name);
    let mut game_conf = torrent_root.join(&dosbox_conf);
    let options_conf = main_torrent_root.join("eXo/emulators/dosbox/options.conf");

    // For LP games, the dosbox_conf was inherited from the EN game.
    // The config lives in the main eXoDOS data dir, but game files are in the LP dir.
    // We use the EN config but redirect mount paths to the LP location via lp_redirect.
    if !game_conf.exists() && source != "eXoDOS" {
        let main_conf = main_torrent_root.join(&dosbox_conf);
        if main_conf.exists() {
            game_conf = main_conf;
            // Keep working_dir as LP torrent root - lp_redirect will fix mount paths
        }
    }

    // The config might be under a language-specific subdirectory
    if !game_conf.exists() {
        let main_game_prefix = collection_game_prefix("eXoDOS");
        let main_segment = crate::commands::setup::collection_def("eXoDOS")
            .map(|c| c.shortcode_segment)
            .unwrap_or("!dos");
        if let Some(shortcode) = dosbox_conf
            .strip_suffix("/dosbox.conf")
            .and_then(|p| p.rsplit('/').next())
            .filter(|s| !s.is_empty())
        {
            let roots = if source != "eXoDOS" {
                vec![&torrent_root, &main_torrent_root]
            } else {
                vec![&torrent_root]
            };
            'outer: for root in &roots {
                for lang_dir in LANG_DIRS {
                    let alt = root.join(format!(
                        "{}/{}/{}/{}/dosbox.conf",
                        main_game_prefix, main_segment, lang_dir, shortcode
                    ));
                    if alt.exists() {
                        game_conf = alt;
                        working_dir = root.join(working_dir_name);
                        break 'outer;
                    }
                }
            }
        }
    }

    if !game_conf.exists() {
        let msg = format!(
            "Game config not found: {}\nMake sure the game is fully downloaded and extracted.",
            game_conf.display()
        );
        log::error!("launch_game({}): {}", game.title, msg);
        return Err(msg);
    }

    if !working_dir.exists() {
        return Err(format!("Working directory not found: {}", working_dir.display()));
    }

    // For LP games, determine the language dir and game path for config patching.
    // The game_folder is the second component of game_prefix (e.g. "eXoDOS" from "eXo/eXoDOS").
    let shortcode = game.shortcode.as_deref().unwrap_or("");
    let game_folder = src_game_prefix.split('/').nth(1).unwrap_or("eXoDOS");

    // Auto-extract ZIP on first launch if the game directory doesn't exist yet.
    // This mirrors LaunchBox's on-demand extraction behavior and handles games that were
    // imported from an existing installation where ZIPs haven't been extracted.
    if !shortcode.is_empty() {
        let game_dir = if let Some(ld) = collection_lang_dir(source) {
            torrent_root.join(format!("{}/{}/{}", src_game_prefix, ld, shortcode))
        } else {
            torrent_root.join(format!("{}/{}", src_game_prefix, shortcode))
        };
        if !game_dir.exists() {
            let game_name = game.application_path.as_deref()
                .and_then(crate::commands::setup::game_name_from_app_path)
                .unwrap_or_else(|| game.title.clone());
            // LP ZIPs live under the collection's language dir
            // ("eXo/eXoDOS/<lang>/<name>.zip"); EN under the prefix root.
            let mut zip_candidates: Vec<PathBuf> = Vec::new();
            if let Some(ld) = collection_lang_dir(source) {
                zip_candidates
                    .push(torrent_root.join(format!("{}/{}/{}.zip", src_game_prefix, ld, game_name)));
            }
            zip_candidates.push(torrent_root.join(format!("{}/{}.zip", src_game_prefix, game_name)));

            if let Some(zip_path) = zip_candidates.iter().find(|z| z.exists()) {
                log::info!("Auto-extracting {} before launch", zip_path.display());
                // Extract next to the ZIP so the game dir lands where the
                // game_dir probe above expects it (lang dir for LP, prefix
                // root for EN).
                let dest = zip_path
                    .parent()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| torrent_root.join(src_game_prefix));
                if let Err(e) = extract_game_zip(zip_path, &dest) {
                    let msg = e.to_string();
                    if msg.contains("EOCD") || msg.contains("invalid Zip") || msg.contains("Invalid archive") {
                        // ZIP is a torrent stub or corrupted file - reset installed flag so the
                        // user can re-download rather than hitting this error on every launch.
                        if let Ok(conn) = db_state.0.lock() {
                            let _ = queries::set_game_installed(&conn, id, false);
                        }
                        return Err(format!(
                            "Game ZIP for '{}' is incomplete or corrupted (torrent placeholder). \
                             Please re-download the game.",
                            game.title
                        ));
                    }
                    return Err(format!("Failed to extract game before launch: {}", msg));
                }
            } else {
                return Err(format!(
                    "Game files not found for '{}'. The game may need to be re-downloaded.",
                    game.title
                ));
            }
        }
    }
    let lp_info = collection_lang_dir(source).map(|ld| {
        let dir = torrent_root.join(format!("{}/{}/{}", src_game_prefix, ld, shortcode));
        (shortcode, ld, game_folder, dir)
    });

    // Engine selection: on Windows, ECE-variant games run eXo's actual
    // DOSBox ECE build (extracted from util.zip's EXTDOS.zip into
    // eXo/emulators/dosbox/<variant>/). Everywhere else - and until the
    // build is on disk - DOSBox Staging is the best-effort fallback.
    let mut ece_bin: Option<PathBuf> = None;
    if let Some(ref variant) = game.dosbox_variant {
        if variant.starts_with("ece") {
            if cfg!(windows) {
                let base = main_torrent_root.join("eXo/emulators/dosbox");
                ece_bin = [
                    base.join(variant).join("DOSBox.exe"),
                    base.join("ece4230").join("DOSBox.exe"),
                ]
                .into_iter()
                .find(|p| p.exists());
                if ece_bin.is_none() {
                    log::info!(
                        "ECE build not on disk yet for '{}' - using Staging (fetched with util.zip on next MIDI/ECE download)",
                        game.title
                    );
                }
            } else {
                log::info!(
                    "Game '{}' is tuned for DOSBox ECE '{}' (Windows-only build). \
                     Running under DOSBox Staging - experience may vary.",
                    game.title, variant
                );
            }
        }
    }
    let use_ece = ece_bin.is_some();

    let patched_conf = patch_dosbox_conf(
        &game_conf,
        &working_dir,
        lp_info.as_ref().map(|(sc, ld, gf, dir)| (*sc, *ld, *gf, dir.as_path())),
        // ECE understands its native [midi] keys - only translate for Staging.
        !use_ece,
    )?;

    log::info!(
        "Launching: {} with config {} (patched: {}, engine: {})",
        game.title,
        game_conf.display(),
        patched_conf.display(),
        if use_ece { "DOSBox ECE" } else { "DOSBox Staging" }
    );

    // Linux/Windows need shaders for DOSBox to start at all. macOS only needs
    // them if CRT auto is enabled (otherwise `output = texture` sidesteps the
    // shader pipeline). Avoid writing files to the user's macOS prefs dir
    // unless they've actually opted into shader-based rendering.
    #[cfg(not(target_os = "macos"))]
    ensure_dosbox_shaders(&app);
    #[cfg(target_os = "macos")]
    if crt_auto_enabled {
        ensure_dosbox_shaders(&app);
    }

    let dosbox_bin = ece_bin.unwrap_or_else(|| resolve_dosbox(&app));
    let mut cmd = Command::new(&dosbox_bin);
    cmd.current_dir(&working_dir)
        .arg("-conf")
        .arg(&patched_conf);

    if options_conf.exists() {
        cmd.arg("-conf").arg(&options_conf);
    }

    // macOS: the standalone binary extracted from the .app DMG lacks the bundle's
    // Contents/Resources/glshaders/, so DOSBox aborts when it can't find the
    // mandatory 'interpolation/bilinear' fallback shader. Default path is to
    // force `output = texture` (SDL hardware renderer, no shader pipeline) via
    // a last-wins conf fragment - sidesteps the shader requirement entirely.
    // If the user enabled CRT shaders globally we skip this override and rely
    // on ensure_dosbox_shaders having installed the pack into
    // ~/Library/Preferences/DOSBox/glshaders (see that function).
    #[cfg(target_os = "macos")]
    {
        if !crt_auto_enabled {
            let conf_path = std::path::Path::new(&data_dir).join("exodium_macos_dosbox.conf");
            std::fs::write(&conf_path, "[sdl]\noutput = texture\n")
                .map_err(|e| format!("Failed to write macOS override conf: {e}"))?;
            cmd.arg("-conf").arg(&conf_path);
        }
    }

    // Global user-preference overrides (all platforms, applied LAST so they win
    // against per-game and options.conf settings). Always written and always
    // authoritative - for BOTH the on and off states. Reason: in DOSBox Staging
    // 0.82+ the default `glshader` is `crt-auto`, and ~90% of eXoDOS per-game
    // configs don't explicitly set glshader, so without an active "off" override
    // the user's unchecked CRT toggle would still get crt-auto from Staging's
    // default. Same logic applies to fullscreen - write the explicit value so
    // the user's UI state always wins, regardless of what eXoDOS configs or
    // DOSBox defaults say.
    {
        let glshader_val = if crt_auto_enabled { "crt-auto" } else { "sharp" };
        let fullscreen_val = if fullscreen_enabled { "true" } else { "false" };
        // glshader is Staging-specific; under ECE only fullscreen applies.
        let frag = if use_ece {
            format!("[sdl]\nfullscreen = {fullscreen_val}\n")
        } else {
            format!(
                "[sdl]\nfullscreen = {fullscreen_val}\n[render]\nglshader = {glshader_val}\n"
            )
        };
        let conf_path = std::path::Path::new(&data_dir).join("exodium_global_overrides.conf");
        std::fs::write(&conf_path, &frag)
            .map_err(|e| format!("Failed to write global override conf: {e}"))?;
        cmd.arg("-conf").arg(&conf_path);
    }

    // Per-game overrides (last-wins over global). Only written if the user has
    // configured game-specific settings via the Game Settings dialog.
    {
        let game_conf_path = std::path::Path::new(&data_dir)
            .join(format!("exodium_game_{}.conf", id));
        if per_game_config.is_empty() {
            // Clean up stale conf file from a previous configuration.
            let _ = std::fs::remove_file(&game_conf_path);
        } else {
            let mut frag = String::new();
            if let Some(fs) = per_game_config.get("fullscreen") {
                frag.push_str(&format!("[sdl]\nfullscreen = {}\n", fs));
            }
            if let Some(gs) = per_game_config.get("glshader") {
                if gs != "default" {
                    frag.push_str(&format!("[render]\nglshader = {}\n", gs));
                }
            }
            if let Some(cy) = per_game_config.get("cycles") {
                frag.push_str(&format!("[cpu]\ncycles = {}\n", cy));
            }
            if let Some(custom) = per_game_config.get("custom_conf") {
                let trimmed = custom.trim();
                if !trimmed.is_empty() {
                    frag.push('\n');
                    frag.push_str(trimmed);
                    frag.push('\n');
                }
            }
            if !frag.is_empty() {
                std::fs::write(&game_conf_path, &frag)
                    .map_err(|e| format!("Failed to write per-game conf: {e}"))?;
                cmd.arg("-conf").arg(&game_conf_path);
            }
        }
    }

    // macOS dev builds: the binary extracted from the .app DMG has a bundle-anchored
    // code signature that becomes invalid without the surrounding bundle. Re-sign
    // ad-hoc if the signature is broken so macOS doesn't SIGKILL the process.
    #[cfg(all(target_os = "macos", debug_assertions))]
    {
        let _ = std::process::Command::new("xattr")
            .args(["-d", "com.apple.quarantine"])
            .arg(&dosbox_bin)
            .output();
        let sig_ok = std::process::Command::new("codesign")
            .arg("-v")
            .arg(&dosbox_bin)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !sig_ok {
            log::warn!("DOSBox binary has invalid signature, re-signing ad-hoc: {}", dosbox_bin.display());
            let _ = std::process::Command::new("codesign")
                .args(["--force", "--sign", "-"])
                .arg(&dosbox_bin)
                .output();
        }
    }

    // Stdio handling differs by platform:
    //
    // macOS: Tauri 2 GUI builds were observed returning EBADF from posix_spawn
    // when stdout/stderr used Stdio::from(File) (dup2-based file_actions). We
    // null all three streams there. DOSBox Staging on macOS writes its own
    // logs into ~/Library/Preferences/DOSBox/, so the diagnostic surface is
    // preserved.
    //
    // Linux/Windows: keep the per-game log file capture introduced for Issue
    // #4 ("started then closed" crashes). On Windows in particular, DOSBox
    // doesn't write a user-accessible log otherwise, so dropping this would
    // be a diagnostic regression.
    #[cfg(target_os = "macos")]
    {
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
    }
    #[cfg(not(target_os = "macos"))]
    {
        cmd.stdin(std::process::Stdio::null());
        let mut stdio_set = false;
        if let Some(log_dir) = crate::commands::setup::LOG_DIR.get() {
            let _ = std::fs::create_dir_all(log_dir);
            let dosbox_log_path = log_dir.join(format!("dosbox-{}.log", id));
            match std::fs::File::create(&dosbox_log_path) {
                Ok(stdout_file) => match stdout_file.try_clone() {
                    Ok(stderr_file) => {
                        cmd.stdout(std::process::Stdio::from(stdout_file));
                        cmd.stderr(std::process::Stdio::from(stderr_file));
                        log::info!("DOSBox output → {}", dosbox_log_path.display());
                        stdio_set = true;
                    }
                    Err(e) => log::warn!("DOSBox log handle clone failed: {e}"),
                },
                Err(e) => log::warn!(
                    "Failed to open DOSBox log file {}: {e}",
                    dosbox_log_path.display()
                ),
            }
        }
        if !stdio_set {
            cmd.stdout(std::process::Stdio::null());
            cmd.stderr(std::process::Stdio::null());
        }
    }

    // macOS-only: force fork+exec instead of posix_spawn via a no-op pre_exec.
    // posix_spawn was the EBADF source on Tauri 2 GUI builds; fork+exec is more
    // permissive about parent fd state. Linux doesn't have the bug and would
    // pay a perf cost from skipping posix_spawn, so we don't apply it there.
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt;
        unsafe { cmd.pre_exec(|| Ok(())); }
    }

    log::info!("Spawning DOSBox: {}", dosbox_bin.display());
    cmd.spawn().map_err(|e| {
        log::error!("DOSBox spawn failed for {}: {} (raw_os_error={:?})",
            dosbox_bin.display(), e, e.raw_os_error());
        format!(
            "Failed to launch DOSBox Staging ({}): {}",
            dosbox_bin.display(), e
        )
    })?;

    Ok(format!("Launched: {}", game.title))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── translate_midi_for_staging ───────────────────────────────────────────

    #[test]
    fn midi_translate_converts_ece_keys_to_staging_sections() {
        // Shape of ~1,500 real eXoDOS configs after path rewriting.
        let conf = "[sdl]\nfullscreen = true\n\
                    [midi]\nmididevice = mt32\nmpu401 = intelligent\n\
                    mt32.romdir = /data/eXo/mt32\n\
                    fluid.soundfont = /data/eXo/mt32/SoundCanvas.sf2\n\
                    fluid.gain = 0.4\n\
                    [autoexec]\nmount c /data/eXo/eXoDOS/SQ5\n";
        let out = translate_midi_for_staging(conf);

        // ECE dotted keys removed from [midi], Staging keys kept.
        assert!(!out.contains("mt32.romdir"));
        assert!(!out.contains("fluid.soundfont"));
        assert!(!out.contains("fluid.gain"));
        assert!(out.contains("mididevice = mt32"));
        assert!(out.contains("mpu401 = intelligent"));

        // Staging sections appended with the captured values.
        assert!(out.contains("[mt32]\nromdir = /data/eXo/mt32"));
        assert!(out.contains("[fluidsynth]\nsoundfont = /data/eXo/mt32/SoundCanvas.sf2"));

        // Autoexec untouched.
        assert!(out.contains("mount c /data/eXo/eXoDOS/SQ5"));
    }

    #[test]
    fn midi_translate_maps_default_device_to_auto() {
        let conf = "[midi]\nmididevice = default\nmt32.romdir = /x/mt32\n";
        let out = translate_midi_for_staging(conf);
        assert!(out.contains("mididevice = auto"));
        assert!(!out.contains("default"));
    }

    #[test]
    fn midi_translate_leaves_staging_native_configs_alone() {
        // Shape of the ~750 Staging-authored eXoDOS configs.
        let conf = "[midi]\nmididevice = auto\n\
                    [mt32]\nromdir = /data/eXo/mt32\n\
                    [fluidsynth]\nsoundfont = /data/eXo/mt32/SoundCanvas.sf2\n";
        let out = translate_midi_for_staging(conf);
        assert_eq!(out.matches("[mt32]").count(), 1);
        assert_eq!(out.matches("[fluidsynth]").count(), 1);
        assert!(out.contains("romdir = /data/eXo/mt32"));
    }

    #[test]
    fn midi_translate_no_midi_config_is_passthrough() {
        let conf = "[sdl]\nfullscreen = true\n[autoexec]\nrunme.exe\n";
        assert_eq!(translate_midi_for_staging(conf), conf);
    }

    // ── collection_data_dir ──────────────────────────────────────────────────

    #[test]
    fn collection_data_dir_exodos_is_root() {
        let dir = collection_data_dir("/data", "eXoDOS");
        assert_eq!(dir, std::path::PathBuf::from("/data"));
    }

    #[test]
    fn collection_data_dir_glp_is_root() {
        let dir = collection_data_dir("/data", "eXoDOS_GLP");
        assert_eq!(dir, std::path::PathBuf::from("/data"));
    }

    #[test]
    fn collection_data_dir_slp_is_root() {
        let dir = collection_data_dir("/data", "eXoDOS_SLP");
        assert_eq!(dir, std::path::PathBuf::from("/data"));
    }

    // ── patch_dosbox_conf ────────────────────────────────────────────────────

    fn write_conf(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn patch_dosbox_conf_converts_windows_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();

        let conf_content = "[sdl]\nfullscreen=false\n[autoexec]\n@mount c .\\eXoDOS\\SQ5\nc:\nSQ5.bat\nexit\n";
        let conf_path = write_conf(working_dir, "dosbox.conf", conf_content);

        let patched_path = patch_dosbox_conf(&conf_path, working_dir, None, true).unwrap();
        let patched = fs::read_to_string(&patched_path).unwrap();

        // Backslash replaced with forward slash
        assert!(!patched.contains('\\'), "no backslashes should remain: {}", patched);
        // Relative .\ prefix replaced with absolute working dir. On Windows
        // the working dir itself contains backslashes, which the patcher
        // normalizes to forward slashes - normalize the expectation too.
        let abs_prefix = format!("{}/", working_dir.to_string_lossy()).replace('\\', "/");
        assert!(patched.contains(&abs_prefix), "absolute path prefix expected: {}", patched);
    }

    #[test]
    fn patch_dosbox_conf_lp_overlay_direct_mount() {
        // EN conf mounts the game dir directly: mount target must be routed
        // through the overlay staging dir whose link points at the LP dir.
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        let lp_dir = working_dir.join("eXoDOS/!german/SQ5");
        fs::create_dir_all(&lp_dir).unwrap();
        fs::write(lp_dir.join("SQ5.BAT"), b"").unwrap();

        let conf_content = "[autoexec]\n@mount c .\\eXoDOS\\SQ5\nc:\nSQ5.bat\nexit\n";
        let conf_path = write_conf(working_dir, "dosbox.conf", conf_content);

        let patched_path = patch_dosbox_conf(
            &conf_path,
            working_dir,
            Some(("SQ5", "!german", "eXoDOS", &lp_dir)),
            true,
        )
        .unwrap();
        let patched = fs::read_to_string(&patched_path).unwrap();

        assert!(
            patched.contains(".exodium_lp/german_SQ5"),
            "mount should be routed through the overlay: {}",
            patched
        );
        assert!(patched.contains("SQ5.bat"), "launch command must survive: {}", patched);
        // The overlay link resolves to the LP dir.
        let linked = working_dir.join(".exodium_lp/german_SQ5/SQ5");
        assert!(linked.join("SQ5.BAT").exists(), "overlay link should reach LP files");
    }

    #[test]
    fn patch_dosbox_conf_lp_overlay_root_mount_cd() {
        // Cobra Mission (ES) shape: EN conf mounts the eXoDOS root and cd's
        // into the game dir; the LP dir holds a bare root-level EXE.
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        fs::create_dir_all(working_dir.join("eXoDOS")).unwrap();
        let lp_dir = working_dir.join("eXoDOS/!spanish/cobmiss");
        fs::create_dir_all(&lp_dir).unwrap();
        fs::write(lp_dir.join("CM.EXE"), b"").unwrap();

        let conf_content =
            "[autoexec]\n@mount c .\\eXoDOS\\\nc:\ncls\ncd cobmiss\n@cm\nexit\n";
        let conf_path = write_conf(working_dir, "dosbox.conf", conf_content);

        let patched_path = patch_dosbox_conf(
            &conf_path,
            working_dir,
            Some(("cobmiss", "!spanish", "eXoDOS", &lp_dir)),
            true,
        )
        .unwrap();
        let patched = fs::read_to_string(&patched_path).unwrap();

        assert!(
            patched.contains(".exodium_lp/spanish_cobmiss"),
            "root mount should be routed through the overlay: {}",
            patched
        );
        // The authored launch sequence survives verbatim.
        assert!(patched.contains("cd cobmiss"), "{}", patched);
        assert!(patched.contains("@cm"), "{}", patched);
        // And the overlay resolves cd cobmiss -> LP files.
        let linked = working_dir.join(".exodium_lp/spanish_cobmiss/cobmiss");
        assert!(linked.join("CM.EXE").exists(), "overlay link should reach LP files");
    }

    #[test]
    fn patch_dosbox_conf_lp_falls_back_when_exe_renamed() {
        // LP variant renamed the executable: the EN launch command can't be
        // validated, so the generated-autoexec fallback must kick in and
        // find the actual root-level EXE.
        let tmp = tempfile::tempdir().unwrap();
        let working_dir = tmp.path();
        fs::create_dir_all(working_dir.join("eXoDOS")).unwrap();
        let lp_dir = working_dir.join("eXoDOS/!spanish/cobmiss");
        fs::create_dir_all(&lp_dir).unwrap();
        fs::write(lp_dir.join("JUEGO.EXE"), b"").unwrap();

        let conf_content =
            "[autoexec]\n@mount c .\\eXoDOS\\\nc:\ncd cobmiss\n@cm\nexit\n";
        let conf_path = write_conf(working_dir, "dosbox.conf", conf_content);

        let patched_path = patch_dosbox_conf(
            &conf_path,
            working_dir,
            Some(("cobmiss", "!spanish", "eXoDOS", &lp_dir)),
            true,
        )
        .unwrap();
        let patched = fs::read_to_string(&patched_path).unwrap();

        assert!(
            patched.to_ascii_lowercase().contains("juego.exe"),
            "fallback should launch the real executable: {}",
            patched
        );
        assert!(
            patched.contains("!spanish/cobmiss"),
            "fallback mounts the LP dir directly: {}",
            patched
        );
    }

    // ── find_lp_launch ───────────────────────────────────────────────────────

    #[test]
    fn find_lp_launch_parses_run_bat() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();

        // Create the target executable so the directory scan finds it
        fs::write(game_dir.join("sq5.exe"), b"").unwrap();

        let run_bat = "@call sq5.exe\n";
        fs::write(game_dir.join("run.bat"), run_bat).unwrap();

        let result = find_lp_launch(game_dir, None);
        assert!(result.is_some(), "run.bat parsing should find a launch command");
        let (subdir, cmd) = result.unwrap();
        assert_eq!(subdir, "", "game is in root of game_dir");
        assert_eq!(cmd, "sq5.exe");
    }

    #[test]
    fn find_lp_launch_finds_com_file_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();

        // No run.bat, but a .com file exists
        fs::write(game_dir.join("game.com"), b"").unwrap();

        let result = find_lp_launch(game_dir, None);
        assert!(result.is_some(), ".com file should be found as fallback");
        let (_, cmd) = result.unwrap();
        assert!(cmd.to_lowercase().ends_with(".com"));
    }

    #[test]
    fn find_lp_launch_returns_none_for_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(find_lp_launch(tmp.path(), None).is_none());
    }

    #[test]
    fn find_lp_launch_uses_en_autoexec_command() {
        // Regression: Cobra Mission (ES) - bare root-level CM.EXE plus
        // INSTALL.EXE, no .bat. The EN autoexec names the launcher.
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();
        fs::write(game_dir.join("CM.EXE"), b"").unwrap();
        fs::write(game_dir.join("INSTALL.EXE"), b"").unwrap();
        fs::write(game_dir.join("DAT.VOL"), b"").unwrap();

        let en_conf = "[sdl]\nfullscreen=false\n[autoexec]\n\
                       @mount c .\\eXoDOS\\\nc:\ncls\ncd cobmiss\n@cm\nexit\n";
        let (subdir, cmd) = find_lp_launch(game_dir, Some(en_conf)).unwrap();
        assert_eq!(subdir, "");
        assert_eq!(cmd, "cm");
    }

    #[test]
    fn find_lp_launch_falls_back_to_root_exe() {
        // No EN hint, no .bat/.com: the root-level EXE must still be found
        // (installers are skipped).
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();
        fs::write(game_dir.join("CM.EXE"), b"").unwrap();
        fs::write(game_dir.join("INSTALL.EXE"), b"").unwrap();

        let (subdir, cmd) = find_lp_launch(game_dir, None).unwrap();
        assert_eq!(subdir, "");
        assert_eq!(cmd.to_ascii_lowercase(), "cm.exe");
    }

    #[test]
    fn find_lp_launch_en_hint_ignores_missing_program() {
        // EN autoexec references a program the LP dir doesn't have -
        // must fall through to the heuristics, not return a broken command.
        let tmp = tempfile::tempdir().unwrap();
        let game_dir = tmp.path();
        fs::write(game_dir.join("game.com"), b"").unwrap();

        let en_conf = "[autoexec]\nmount c .\\eXoDOS\\\nc:\ncd foo\n@other\nexit\n";
        let (_, cmd) = find_lp_launch(game_dir, Some(en_conf)).unwrap();
        assert_eq!(cmd.to_ascii_lowercase(), "game.com");
    }
}
