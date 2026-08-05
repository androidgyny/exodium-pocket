mod commands;
pub mod db;
pub mod import;
pub mod models;
pub mod torrent;
pub mod vhd;

// Re-export utilities used by the generate_db binary and integration tests
pub use commands::game_name_from_app_path;
pub use commands::{collection_base_id, collection_data_dir, CollectionDef, COLLECTION_MAP};

use std::path::Path;
use std::sync::Mutex;

use tauri::Manager;
use tokio::sync::RwLock;

use commands::{
    bundled_metadata_dir, cancel_content_pack_install, cancel_download,
    download_game, factory_reset, get_available_collections, get_config,
    get_content_pack_progress, get_default_data_dir, get_download_progress, get_game,
    get_game_metadata, get_game_settings, get_log_dir, get_poster_dir, get_preview_dir,
    get_game_variants, get_games, get_genres, get_installed_games, get_recently_played,
    get_section_keys, set_game_settings,
    create_playlist, delete_playlist, get_game_playlists, get_playlists, rename_playlist,
    set_playlist_membership,
    get_setup_status, get_thumbnail_dir, get_torrent_info, init_download_manager,
    game_printing_unavailable, init_log_dir, init_resource_dir, install_content_pack, launch_game,
    list_content_packs,
    get_transfer_stats, open_log_folder, open_manual, scan_installed_games, set_config, set_rate_limits,
    set_seeding_enabled, setup_from_local, setup_import,
    update_check_supported,
    setup_start, toggle_favorite, uninstall_content_pack, uninstall_game, validate_exodos_dir,
    ContentPackState, DbState, TorrentState,
};

/// Raise the file-descriptor soft limit as high as the platform allows.
/// librqbit's filesystem storage opens EVERY file of a torrent read/write
/// and keeps the handles (14,011 for the eXoDOS torrent) - Linux's default
/// soft limit of 1024 makes the first torrent add fail with "error opening
/// ... in read/write mode" at roughly file #950 (verified in the field:
/// Brickwar's GameData at torrent index 947). Standard practice for torrent
/// clients; no-op on Windows (no comparable limit).
#[cfg(unix)]
fn raise_fd_limit() {
    unsafe {
        let mut lim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) != 0 {
            return;
        }
        // Try the hard limit first; macOS reports RLIM_INFINITY but caps
        // setrlimit at kern.maxfilesperproc, so fall back through sane values.
        let mut candidates: Vec<libc::rlim_t> = vec![lim.rlim_max.min(1 << 20), 65536, 10240];

        // macOS: kern.maxfilesperproc scales with installed RAM - 245760 on
        // large machines but 61440 (or less) on small ones. 61440 < 65536,
        // so without querying it those machines fell through to 10240: fewer
        // fds than the eXoDOS torrent has files (14011), and librqbit opens
        // EVERY file on torrent add. Field failure: EMFILE at torrent index
        // ~10220 ("Laserwars (1994).zip") on an M2 Mac mini.
        #[cfg(target_os = "macos")]
        {
            let mut maxfiles: libc::c_int = 0;
            let mut len = std::mem::size_of::<libc::c_int>();
            let name = b"kern.maxfilesperproc\0";
            if libc::sysctlbyname(
                name.as_ptr() as *const libc::c_char,
                &mut maxfiles as *mut _ as *mut libc::c_void,
                &mut len,
                std::ptr::null_mut(),
                0,
            ) == 0
                && maxfiles > 0
            {
                candidates.push(maxfiles as libc::rlim_t);
            }
        }
        // Descending order so the early-exit below stays correct.
        candidates.sort_unstable_by(|a, b| b.cmp(a));
        candidates.dedup();

        // The eXoDOS torrent has 14 011 files and librqbit holds an fd for
        // each; anything below this leaves the main collection un-addable.
        const COMFORTABLE: libc::rlim_t = 15_000;

        for target in candidates {
            if target <= lim.rlim_cur {
                break; // already high enough
            }
            let new = libc::rlimit {
                rlim_cur: target,
                rlim_max: lim.rlim_max,
            };
            if libc::setrlimit(libc::RLIMIT_NOFILE, &new) == 0 {
                log::info!("Raised open-file limit: {} -> {}", lim.rlim_cur, target);
                if target < COMFORTABLE {
                    log::warn!(
                        "Open-file limit {} is below the ~{} needed for the main eXoDOS \
                         torrent - downloads from it will fail with 'error opening ... in \
                         read/write mode'. Raise kern.maxfilesperproc / ulimit -n.",
                        target, COMFORTABLE
                    );
                }
                return;
            }
        }
        if lim.rlim_cur < COMFORTABLE {
            log::warn!(
                "Could not raise open-file limit above {} - large torrents may fail to add",
                lim.rlim_cur
            );
        }
    }
}

#[cfg(not(unix))]
fn raise_fd_limit() {}

/// Copy the bundled pre-built DB to the target path.
pub fn install_bundled_db(target: &Path) -> Result<(), String> {
    let metadata_dir = bundled_metadata_dir()?;

    let bundled_db = metadata_dir.join("exodium.db");
    let bundled_db_gz = metadata_dir.join("exodium.db.gz");

    // Clean up any stale WAL/SHM files
    let _ = std::fs::remove_file(target.with_extension("db-wal"));
    let _ = std::fs::remove_file(target.with_extension("db-shm"));

    if bundled_db.exists() {
        std::fs::copy(&bundled_db, target)
            .map_err(|e| format!("Failed to copy bundled DB: {}", e))?;
        log::info!("Installed bundled DB from {}", bundled_db.display());
    } else if bundled_db_gz.exists() {
        use flate2::read::GzDecoder;
        let file = std::fs::File::open(&bundled_db_gz)
            .map_err(|e| e.to_string())?;
        let mut decoder = GzDecoder::new(file);
        let mut out = std::fs::File::create(target)
            .map_err(|e| e.to_string())?;
        std::io::copy(&mut decoder, &mut out)
            .map_err(|e| e.to_string())?;
        log::info!("Installed bundled DB from {}", bundled_db_gz.display());
    } else {
        return Err(format!(
            "No bundled database found in {}",
            metadata_dir.display()
        ));
    }
    Ok(())
}

/// Grant the asset protocol access to the game-media subtree of the
/// user-chosen data dir at runtime. The static scope in tauri.conf.json is
/// limited to $RESOURCE/$APPDATA; game media (thumbnails, screenshots,
/// manuals) all live under <data_dir>/eXoDOS - granting the data dir itself
/// (often $HOME) would expose far more than the app serves.
pub fn allow_asset_dir(app: &tauri::AppHandle, data_dir: &Path) {
    use tauri::Manager;
    // Served subtrees: per-collection game media (<data>/<inner_folder>, one
    // per distinct inner folder - eXoWin3x/eXoWin9x have their own trees) and
    // installed content packs (posters, metadata screenshots) in <data>/content.
    // Regression note: v0.7.x granted only eXoDOS/, silently blocking every
    // content-pack image ("asset protocol not configured to allow" spam).
    let mut subs: Vec<&str> = commands::setup::COLLECTION_MAP
        .iter()
        .map(|c| c.inner_folder)
        .collect();
    subs.push("content");
    subs.dedup();
    for sub in subs {
        let dir = data_dir.join(sub);
        if let Err(e) = app.asset_protocol_scope().allow_directory(&dir, true) {
            log::warn!("Failed to extend asset scope to {}: {}", dir.display(), e);
        }
    }

    // Bundled preview thumbnails. The static scope grants $RESOURCE/**, which
    // covers the packaged app - but in `tauri dev` the previews are read from
    // the source tree (src-tauri/resources/previews), which is outside it. The
    // result was ~200 denials per session and no fallback covers for any game
    // without a downloaded poster pack.
    let preview_roots = [
        commands::setup::RESOURCE_DIR.get().map(|d| d.join("previews")),
        // `tauri dev` resolves previews from the source tree - see
        // setup::get_preview_dir, which probes exactly this path first.
        Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("resources").join("previews")),
    ];
    for previews in preview_roots.into_iter().flatten() {
        if !previews.is_dir() {
            continue;
        }
        if let Err(e) = app.asset_protocol_scope().allow_directory(&previews, true) {
            log::warn!("Failed to extend asset scope to {}: {}", previews.display(), e);
        }
    }
}

/// Empty in-memory DB used when the real one can't be opened, so the app
/// can still reach the event loop and show the startup-error dialog without
/// commands panicking on missing state.
fn fallback_in_memory_db() -> rusqlite::Connection {
    let conn = rusqlite::Connection::open_in_memory()
        .expect("in-memory SQLite cannot fail to open");
    let _ = db::init(&conn);
    conn
}

/// Open the installed DB, (re)installing the bundled one when the file is
/// missing, unreadable, or empty (post factory-reset). Every failure comes
/// back as a message for the startup error dialog instead of a panic.
fn open_or_reinstall_db(db_path: &Path) -> Result<rusqlite::Connection, String> {
    if !db_path.exists() {
        install_bundled_db(db_path)?;
    }

    match db::open(db_path).and_then(|c| {
        db::init(&c)?;
        Ok(c)
    }) {
        Ok(c) => {
            // Check if DB has games; if empty (factory reset), reinstall
            let count: i64 = c
                .query_row("SELECT COUNT(*) FROM games", [], |r| r.get(0))
                .unwrap_or(0);
            if count == 0 {
                drop(c);
                install_bundled_db(db_path)?;
                let c = db::open(db_path)
                    .map_err(|e| format!("failed to open freshly installed DB: {}", e))?;
                db::init(&c).map_err(|e| format!("failed to run migrations: {}", e))?;
                Ok(c)
            } else {
                Ok(c)
            }
        }
        Err(e) => {
            log::warn!("Database unreadable ({}), reinstalling", e);
            let _ = std::fs::remove_file(db_path);
            install_bundled_db(db_path)?;
            let c = db::open(db_path)
                .map_err(|e| format!("failed to open freshly installed DB: {}", e))?;
            db::init(&c).map_err(|e| format!("failed to initialize schema: {}", e))?;
            Ok(c)
        }
    }
}

/// Append collections this release added to the user's `collections` config.
///
/// A catalog refresh brings a new pack's games into an existing install, but
/// `init_download_manager` only starts managers for the ids listed in that
/// config - written once at setup. Without this the new games render with a
/// download button that can only fail ("No torrent manager for collection").
/// Existing entries are preserved and their order kept; nothing is removed.
fn enable_new_collections(conn: &rusqlite::Connection) {
    let Ok(Some(current)) = db::queries::get_config(conn, "collections") else {
        return;
    };
    let mut ids: Vec<&str> = current.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    let added: Vec<&str> = COLLECTION_MAP
        .iter()
        .map(|c| c.id)
        .filter(|id| !ids.contains(id))
        .collect();
    if added.is_empty() {
        return;
    }
    ids.extend(added.iter().copied());
    match db::queries::set_config(conn, "collections", &ids.join(",")) {
        Ok(_) => log::info!("Enabled newly shipped collections: {}", added.join(", ")),
        Err(e) => log::error!("Could not enable new collections {:?}: {}", added, e),
    }
}

/// Stage the bundled catalog DB as a temp file so it can be ATTACHed for a
/// catalog refresh. Always a copy in data_dir - ATTACHing the bundled file
/// in place would create WAL sidecars inside the (possibly read-only/signed)
/// resources dir. The caller deletes the temp file when done.
fn stage_bundled_catalog(data_dir: &Path) -> Result<std::path::PathBuf, String> {
    let metadata_dir = bundled_metadata_dir()?;
    let tmp = data_dir.join("catalog-refresh.db");

    let bundled_db = metadata_dir.join("exodium.db");
    if bundled_db.exists() {
        std::fs::copy(&bundled_db, &tmp).map_err(|e| e.to_string())?;
        return Ok(tmp);
    }
    let bundled_db_gz = metadata_dir.join("exodium.db.gz");
    if bundled_db_gz.exists() {
        let file = std::fs::File::open(&bundled_db_gz).map_err(|e| e.to_string())?;
        let mut decoder = flate2::read::GzDecoder::new(file);
        let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        std::io::copy(&mut decoder, &mut out).map_err(|e| e.to_string())?;
        return Ok(tmp);
    }
    Err(format!(
        "No bundled database found in {}",
        metadata_dir.display()
    ))
}

/// Make-writer that locks a shared file handle on every write. Cloning the
/// `Arc` is cheap; locking is per-event and brief, so contention is not a
/// concern for our log volume.
#[derive(Clone)]
struct SharedFileMakeWriter(std::sync::Arc<std::sync::Mutex<std::fs::File>>);

impl std::io::Write for SharedFileMakeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self.0.lock() {
            // `write_all` rather than `write`: short writes on a regular file
            // are vanishingly rare but possible, and a partial log line that
            // tracing-subscriber doesn't retry would corrupt the log file.
            Ok(mut f) => f.write_all(buf).map(|_| buf.len()),
            // If the mutex is poisoned, drop the bytes rather than panic in
            // the logger. We still return Ok so the subscriber doesn't loop.
            Err(_) => Ok(buf.len()),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self.0.lock() {
            Ok(mut f) => f.flush(),
            Err(_) => Ok(()),
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedFileMakeWriter {
    type Writer = SharedFileMakeWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// Initialize the global tracing subscriber. Output is fanned out to both
/// stderr (visible in `pnpm tauri dev`) and a persistent log file at
/// `<log_dir>/exodium.log` (the only sink visible in a packaged Windows GUI
/// build, where stderr is detached). `tracing-log` bridges `log!` calls from
/// any crate into the same subscriber, so logs from `log` and `tracing` users
/// (e.g. librqbit) end up in one stream.
///
/// Returns the log file path so the UI can show it to users for diagnosis.
fn init_logger(log_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::io::Write;
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};

    let _ = std::fs::create_dir_all(log_dir);
    let log_path = log_dir.join("exodium.log");

    // Rotate once the log grows past ~10 MB: keep exactly one predecessor
    // (exodium.log.1) so a crash/wedge can't destroy its own evidence and
    // the log can't grow unbounded (55 MB observed in the field).
    if let Ok(meta) = std::fs::metadata(&log_path) {
        if meta.len() > 10 * 1024 * 1024 {
            let _ = std::fs::rename(&log_path, log_dir.join("exodium.log.1"));
        }
    }

    let file_result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);

    // Default to info everywhere - debug chatter (librqbit's per-piece /
    // requeue messages) drowned the log window in the field. Override with
    // `RUST_LOG` when diagnosing, e.g. `RUST_LOG=librqbit=debug,exodium_lib=debug`
    // (or `librqbit_dht=debug` for DHT issues).
    let default_filter = "info";
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_filter));

    // Write a session separator before the subscriber takes over so multi-run
    // log files remain readable.
    let file_writer: Option<SharedFileMakeWriter> = match file_result {
        Ok(mut file) => {
            let epoch_secs = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(
                file,
                "\n=== exodium session start (epoch {}, log_dir {}) ===",
                epoch_secs,
                log_dir.display()
            );
            Some(SharedFileMakeWriter(std::sync::Arc::new(std::sync::Mutex::new(
                file,
            ))))
        }
        Err(_) => None,
    };

    // Build the subscriber: stderr layer + (optional) file layer + filter.
    // `with_target(true)` keeps "librqbit::session" prefixes so we can tell
    // librqbit events apart from our own.
    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_ansi(false);
    let registry = tracing_subscriber::registry().with(env_filter).with(stderr_layer);

    let result = if let Some(writer) = file_writer.clone() {
        let file_layer = fmt::layer()
            .with_writer(writer)
            .with_target(true)
            .with_ansi(false);
        registry.with(file_layer).try_init()
    } else {
        registry.try_init()
    };

    if result.is_err() {
        // A subscriber was already installed (e.g. tests) - not fatal.
        return None;
    }

    // Bridge `log!` → tracing so log-only crates land in the same sink.
    // Ignore failure: it just means log was already initialized elsewhere.
    let _ = tracing_log::LogTracer::init();

    file_writer.map(|_| log_path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    raise_fd_limit();

    // WebKitGTK's DMA-BUF renderer aborts with "Could not create default EGL
    // display: EGL_BAD_PARAMETER" on some Wayland/driver combos (NVIDIA
    // especially) before our window even exists. Disable it unless the user
    // explicitly set the variable themselves - the fallback render path works
    // everywhere at a minor compositing cost.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        // A second instance would contend on the SQLite DB and corrupt the
        // torrent session; focus the existing window instead.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            use tauri::Manager;
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // Initialize the logger as early as possible so later setup steps' log
            // output is captured. `app_log_dir()` resolves to platform conventions:
            //   Windows:  %APPDATA%\com.redfox.exodium\logs
            //   macOS:    ~/Library/Logs/com.redfox.exodium
            //   Linux:    ~/.local/share/com.redfox.exodium/logs
            let log_dir = app.path().app_log_dir().ok();
            // Cache the log directory so the `get_log_dir` Tauri command can
            // serve it without going through `app.path()` again - the
            // round-trip was observed failing in shipped Windows builds.
            if let Some(ref dir) = log_dir {
                init_log_dir(dir.clone());
            }
            let log_path = log_dir.as_deref().and_then(init_logger);
            if let Some(ref p) = log_path {
                log::info!("Log file: {}", p.display());
            }

            // Cache the resource_dir BEFORE any code tries to read bundled metadata,
            // torrents, or shaders - the sync helpers in setup.rs rely on this.
            if let Ok(res_dir) = app.path().resource_dir() {
                init_resource_dir(res_dir);
            } else {
                log::warn!("resource_dir() unavailable; bundled assets may not be found");
            }

            // Any failure from here on is fatal but must be VISIBLE: a panic
            // in setup() kills the process with nothing on screen. A BLOCKING
            // dialog can't be used here either - setup() runs on the main
            // thread before the event loop, and tauri-plugin-dialog's
            // blocking_show deadlocks there on macOS (the alert needs the
            // main run loop, which would be parked). Instead: show a
            // non-blocking dialog whose dismissal exits the app, and continue
            // setup on an empty in-memory DB so the event loop starts and
            // the dialog can actually render.
            let mut startup_error: Option<String> = None;

            let data_dir = match app.path().app_data_dir() {
                Ok(d) => d,
                Err(e) => {
                    startup_error =
                        Some(format!("Could not resolve the application data directory: {}", e));
                    std::env::temp_dir().join("exodium-fallback")
                }
            };
            if startup_error.is_none() {
                if let Err(e) = std::fs::create_dir_all(&data_dir) {
                    startup_error = Some(format!(
                        "Could not create the application data directory {}: {}",
                        data_dir.display(),
                        e
                    ));
                }
            }
            let db_path = data_dir.join("exodium.db");
            log::info!("Database path: {}", db_path.display());

            let mut conn = if startup_error.is_none() {
                match open_or_reinstall_db(&db_path) {
                    Ok(c) => c,
                    Err(msg) => {
                        startup_error = Some(format!("Could not open the game database: {}", msg));
                        fallback_in_memory_db()
                    }
                }
            } else {
                fallback_in_memory_db()
            };

            if let Some(msg) = &startup_error {
                log::error!("Fatal startup error: {}", msg);
                use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                let exit_handle = app.handle().clone();
                app.dialog()
                    .message(format!("{}\n\nSee the log folder for details.", msg))
                    .title("Exodium failed to start")
                    .kind(MessageDialogKind::Error)
                    .show(move |_| exit_handle.exit(1));
            }

            // Catalog refresh: existing installs never re-read the bundled DB,
            // so corrected/new catalog data (torrent indices, sizes, new games)
            // is applied here whenever the shipped CATALOG_VERSION moves ahead
            // of the installed one. User state (installed/favorites/...) and
            // games.id survive - see db::refresh_catalog.
            let installed_ver = db::catalog_version(&conn);
            if startup_error.is_none() && installed_ver < db::CATALOG_VERSION {
                match stage_bundled_catalog(&data_dir) {
                    Ok(cat_path) => {
                        match db::refresh_catalog(&mut conn, &cat_path) {
                            Ok((updated, inserted)) => {
                                log::info!(
                                    "Catalog refreshed v{} -> v{}: {} rows updated, {} inserted",
                                    installed_ver, db::CATALOG_VERSION, updated, inserted
                                );
                                enable_new_collections(&conn);
                            }
                            Err(e) => log::error!("Catalog refresh failed: {}", e),
                        }
                        let _ = std::fs::remove_file(&cat_path);
                    }
                    Err(e) => log::error!("Catalog refresh: bundled DB unavailable: {}", e),
                }
            }

            // Clean up stale content-pack download artifacts from interrupted installs.
            if let Ok(Some(user_data_dir)) = db::queries::get_config(&conn, "data_dir") {
                let user_data_path = std::path::Path::new(&user_data_dir);
                // Asset protocol must reach game media in the user-chosen dir.
                allow_asset_dir(app.handle(), user_data_path);
                commands::content_packs::cleanup_stale_downloads(user_data_path);
                // Remove content packs whose installed version is lower than the
                // current manifest (e.g. v0.2.x shortcode-keyed posters after the
                // v0.3.x hash-keyed rebuild). Without this the 404s for every
                // game card flood the tauri::protocol::asset error log.
                commands::content_packs::cleanup_stale_content_packs(&conn, user_data_path);
            }

            app.manage(DbState(Mutex::new(conn)));
            app.manage(TorrentState(RwLock::new(std::collections::HashMap::new())));
            app.manage(ContentPackState::new());
            app.manage(commands::media::VideoState::new());

            // macOS uses native traffic-light controls (no custom titlebar).
            // Linux/Windows keep the framed shell from tauri.conf.json
            // (decorations: false). Done at runtime so we don't depend on
            // platform-specific config files which weren't picking up reliably.
            #[cfg(target_os = "macos")]
            {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_decorations(true);
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_games,
            get_game,
            get_installed_games,
            get_game_variants,
            get_genres,
            launch_game,
            game_printing_unavailable,
            get_config,
            set_config,
            set_seeding_enabled,
            set_rate_limits,
            get_transfer_stats,
            update_check_supported,
            get_torrent_info,
            setup_start,
            get_setup_status,
            setup_import,
            setup_from_local,
            get_default_data_dir,
            get_thumbnail_dir,
            get_available_collections,
            init_download_manager,
            commands::media::start_game_video,
            commands::media::get_video_status,
            commands::media::cancel_game_video,
            factory_reset,
            download_game,
            cancel_download,
            uninstall_game,
            get_download_progress,
            toggle_favorite,
            get_section_keys,
            validate_exodos_dir,
            scan_installed_games,
            commands::win9x::get_win9x_support_status,
            commands::win9x::win9x_engine_available,
            commands::win9x::win9x_network_status,
            commands::win9x::enable_win9x_network,
            commands::win9x::win9x_needs_network_prompt,
            commands::win9x::dismiss_win9x_network_prompt,
            list_content_packs,
            install_content_pack,
            uninstall_content_pack,
            get_content_pack_progress,
            cancel_content_pack_install,
            get_preview_dir,
            get_poster_dir,
            get_game_metadata,
            get_game_settings,
            set_game_settings,
            get_recently_played,
            get_log_dir,
            open_log_folder,
            open_manual,
            get_playlists,
            create_playlist,
            rename_playlist,
            delete_playlist,
            set_playlist_membership,
            get_game_playlists,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod collection_migration_tests {
    use super::*;

    fn config_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        db::init(&conn).unwrap();
        conn
    }

    /// The upgrade path for a pack shipped after the user's install: its games
    /// arrive with the catalog refresh, so its manager has to start too.
    #[test]
    fn appends_collections_the_install_predates() {
        let conn = config_db();
        db::queries::set_config(&conn, "collections", "eXoDOS,eXoDOS_GLP").unwrap();
        enable_new_collections(&conn);
        let after = db::queries::get_config(&conn, "collections").unwrap().unwrap();
        let ids: Vec<&str> = after.split(',').collect();
        for c in COLLECTION_MAP {
            assert!(ids.contains(&c.id), "{} missing from {:?}", c.id, ids);
        }
        // The user's existing entries keep their place.
        assert_eq!(&ids[..2], &["eXoDOS", "eXoDOS_GLP"]);
    }

    /// Nothing to do must mean no write - and no key must stay no key, so a
    /// pre-setup install is not handed a collection list it never chose.
    #[test]
    fn leaves_complete_and_unset_configs_alone() {
        let conn = config_db();
        let all = COLLECTION_MAP.iter().map(|c| c.id).collect::<Vec<_>>().join(",");
        db::queries::set_config(&conn, "collections", &all).unwrap();
        enable_new_collections(&conn);
        assert_eq!(db::queries::get_config(&conn, "collections").unwrap().unwrap(), all);

        let fresh = config_db();
        enable_new_collections(&fresh);
        assert_eq!(db::queries::get_config(&fresh, "collections").unwrap(), None);
    }
}

#[cfg(all(test, unix))]
mod fd_limit_tests {
    #[test]
    fn raise_fd_limit_reaches_torrent_scale() {
        super::raise_fd_limit();
        unsafe {
            let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
            assert_eq!(libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim), 0);
            println!("soft limit after raise: {} (hard {})", lim.rlim_cur, lim.rlim_max);
            // Floor candidate is 10240; anything less means the raise loop broke.
            assert!(lim.rlim_cur >= 10240.min(lim.rlim_max));
        }
    }
}
