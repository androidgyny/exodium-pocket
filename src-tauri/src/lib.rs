mod commands;
pub mod db;
pub mod import;
pub mod models;
pub mod torrent;

// Re-export utilities used by the generate_db binary and integration tests
pub use commands::game_name_from_app_path;
pub use commands::{collection_data_dir, CollectionDef, COLLECTION_MAP};

use std::path::Path;
use std::sync::Mutex;

use tauri::Manager;
use tokio::sync::RwLock;

use commands::{
    bundled_metadata_dir, cancel_content_pack_install, cancel_download, check_for_updates,
    download_game, factory_reset, get_available_collections, get_config,
    get_content_pack_progress, get_default_data_dir, get_download_progress, get_game,
    get_game_metadata, get_game_settings, get_log_dir, get_poster_dir, get_preview_dir,
    get_game_variants, get_games, get_genres, get_installed_games, get_recently_played,
    get_section_keys, set_game_settings,
    get_setup_status, get_thumbnail_dir, get_torrent_info, init_download_manager,
    init_log_dir, init_resource_dir, install_content_pack, launch_game, list_content_packs,
    open_log_folder, scan_installed_games, set_config, setup_from_local, setup_import,
    setup_start, toggle_favorite, uninstall_content_pack, uninstall_game, validate_exodos_dir,
    ContentPackState, DbState, TorrentState,
};

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

    let file_result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);

    // Diagnostic default. Captures the events we need to triage Windows
    // stuck-at-0% (file open errors, peer / tracker activity, sparse-file
    // allocation, our own info) without drowning the file in DHT bootstrap
    // chatter. Set `RUST_LOG` to override — e.g. `RUST_LOG=librqbit_dht=debug`
    // if DHT diagnosis is needed too. ~30s of normal startup ≈ tens of KB.
    let default_filter = "info,librqbit=debug,librqbit_dht=info,exodium_lib=debug,rqbit=info";
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
        // A subscriber was already installed (e.g. tests) — not fatal.
        return None;
    }

    // Bridge `log!` → tracing so log-only crates land in the same sink.
    // Ignore failure: it just means log was already initialized elsewhere.
    let _ = tracing_log::LogTracer::init();

    file_writer.map(|_| log_path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Initialize the logger as early as possible so later setup steps' log
            // output is captured. `app_log_dir()` resolves to platform conventions:
            //   Windows:  %APPDATA%\com.redfox.exodium\logs
            //   macOS:    ~/Library/Logs/com.redfox.exodium
            //   Linux:    ~/.local/share/com.redfox.exodium/logs
            let log_dir = app.path().app_log_dir().ok();
            // Cache the log directory so the `get_log_dir` Tauri command can
            // serve it without going through `app.path()` again — the
            // round-trip was observed failing in shipped Windows builds.
            if let Some(ref dir) = log_dir {
                init_log_dir(dir.clone());
            }
            let log_path = log_dir.as_deref().and_then(init_logger);
            if let Some(ref p) = log_path {
                log::info!("Log file: {}", p.display());
            }

            // Cache the resource_dir BEFORE any code tries to read bundled metadata,
            // torrents, or shaders — the sync helpers in setup.rs rely on this.
            if let Ok(res_dir) = app.path().resource_dir() {
                init_resource_dir(res_dir);
            } else {
                log::warn!("resource_dir() unavailable; bundled assets may not be found");
            }

            let data_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data dir");
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("exodium.db");

            log::info!("Database path: {}", db_path.display());

            // If no DB exists, install the bundled one
            if !db_path.exists() {
                if let Err(e) = install_bundled_db(&db_path) {
                    log::error!("Failed to install bundled DB: {}", e);
                }
            }

            // Open DB, reinstall if corrupt
            let conn = match db::open(&db_path).and_then(|c| { db::init(&c)?; Ok(c) }) {
                Ok(c) => {
                    // Check if DB has games; if empty (factory reset), reinstall
                    let count: i64 = c
                        .query_row("SELECT COUNT(*) FROM games", [], |r| r.get(0))
                        .unwrap_or(0);
                    if count == 0 {
                        drop(c);
                        if let Err(e) = install_bundled_db(&db_path) {
                            log::error!("Failed to install bundled DB: {}", e);
                        }
                        let c = db::open(&db_path).expect("failed to open installed DB");
                        db::init(&c).expect("failed to run migrations on bundled DB");
                        c
                    } else {
                        c
                    }
                }
                Err(e) => {
                    log::warn!("Database unreadable ({}), reinstalling", e);
                    let _ = std::fs::remove_file(&db_path);
                    if let Err(e) = install_bundled_db(&db_path) {
                        log::error!("Failed to install bundled DB: {}", e);
                    }
                    let c = db::open(&db_path).expect("failed to create database");
                    db::init(&c).expect("failed to initialize schema");
                    c
                }
            };

            // Clean up stale content-pack download artifacts from interrupted installs.
            if let Ok(Some(user_data_dir)) = db::queries::get_config(&conn, "data_dir") {
                let user_data_path = std::path::Path::new(&user_data_dir);
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_games,
            get_game,
            get_installed_games,
            get_game_variants,
            get_genres,
            launch_game,
            get_config,
            set_config,
            get_torrent_info,
            setup_start,
            get_setup_status,
            setup_import,
            setup_from_local,
            get_default_data_dir,
            get_thumbnail_dir,
            get_available_collections,
            init_download_manager,
            factory_reset,
            download_game,
            cancel_download,
            uninstall_game,
            get_download_progress,
            check_for_updates,
            toggle_favorite,
            get_section_keys,
            validate_exodos_dir,
            scan_installed_games,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
