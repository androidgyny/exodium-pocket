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
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Manager, State};

use super::TorrentState;
use crate::models::Game;

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

// ── Launch ───────────────────────────────────────────────────────────────────

/// How a resolved engine is invoked: a binary on disk, or DOSBox-X's Flatpak
/// on Linux (no official Linux binaries exist for DOSBox-X).
enum EngineCmd {
    Direct(PathBuf),
    Flatpak(&'static str),
}

impl EngineCmd {
    /// Build a Command; `grant` is a directory the Flatpak sandbox must see.
    fn command(&self, grant: &Path) -> (Command, PathBuf) {
        match self {
            EngineCmd::Direct(bin) => (Command::new(bin), bin.clone()),
            EngineCmd::Flatpak(id) => {
                let mut cmd = Command::new("flatpak");
                cmd.arg("run")
                    .arg(format!("--filesystem={}", grant.display()))
                    .arg(id);
                (cmd, PathBuf::from("flatpak"))
            }
        }
    }
}

fn binary_exists_on_path(name: &str) -> bool {
    let checker = if cfg!(windows) { "where" } else { "which" };
    Command::new(checker)
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Bundled-resource probe: <resource_dir>/<sub>, falling back to the dev
/// tree's src-tauri/resources/<sub> (same convention as resolve_dosbox).
fn resource_candidate(app: &AppHandle, sub: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(res) = app.path().resource_dir() {
        candidates.push(res.join(sub));
    }
    if let Some(res) = crate::commands::setup::RESOURCE_DIR.get() {
        candidates.push(res.join(sub));
    }
    candidates.into_iter().find(|p| p.exists())
}

/// DOSBox-X for x98 games. On Windows eXo's own "x98" build (extracted from
/// EXTWin9x.zip) is the intended emulator, exactly like the ECE precedent;
/// bundled builds serve macOS/Windows-without-support-files; Linux falls
/// back to a system DOSBox-X or its Flatpak.
fn resolve_dosbox_x(app: &AppHandle, torrent_root: &Path) -> Option<EngineCmd> {
    if cfg!(windows) {
        let exo_build = torrent_root.join("eXo/emulators/dosbox/x98/dosbox-x.exe");
        if exo_build.exists() {
            return Some(EngineCmd::Direct(exo_build));
        }
    }
    let bundled = if cfg!(windows) {
        resource_candidate(app, "dosbox-x-bin/dosbox-x.exe")
    } else if cfg!(target_os = "macos") {
        resource_candidate(app, "dosbox-x/dosbox-x.app/Contents/MacOS/dosbox-x")
    } else {
        None
    };
    if let Some(bin) = bundled {
        return Some(EngineCmd::Direct(bin));
    }
    if binary_exists_on_path("dosbox-x") {
        return Some(EngineCmd::Direct(PathBuf::from("dosbox-x")));
    }
    if cfg!(target_os = "linux")
        && Command::new("flatpak")
            .args(["info", "com.dosbox_x.DOSBox-X"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    {
        return Some(EngineCmd::Flatpak("com.dosbox_x.DOSBox-X"));
    }
    None
}

/// 86Box for 86box* games. Bundled on all three platforms; PATH fallback.
fn resolve_86box(app: &AppHandle) -> Option<EngineCmd> {
    let bundled = if cfg!(windows) {
        resource_candidate(app, "86box-bin/86Box.exe")
    } else if cfg!(target_os = "macos") {
        resource_candidate(app, "86box/86Box.app/Contents/MacOS/86Box")
    } else {
        resource_candidate(app, "86box/86Box.AppImage")
    };
    if let Some(bin) = bundled {
        return Some(EngineCmd::Direct(bin));
    }
    if binary_exists_on_path("86Box") {
        return Some(EngineCmd::Direct(PathBuf::from("86Box")));
    }
    None
}

/// Per-variant 86Box wiring, from eXo's 9xlaunch86Box*.bat files: which
/// disposable child VHD to recreate, off which parent, and which per-game
/// cfg to copy into the emulator dir.
fn e86box_variant_files(variant: &str) -> (&'static str, &'static str, &'static str) {
    match variant {
        "86boxME" => ("ME-C.vhd", "ME-P.vhd", "play.cfg"),
        "86boxNetHost" => ("W98-Host.vhd", "W98-NetHost.vhd", "Host.cfg"),
        "86boxNetJoin" => ("W98-Join.vhd", "W98-NetJoin.vhd", "Join.cfg"),
        _ => ("W98-C.vhd", "W98-P.vhd", "play.cfg"),
    }
}

/// Case-insensitive lookup of a file name inside a directory (the conf dirs
/// mix "play.cfg"/"Play.cfg" and the pack was authored case-insensitively).
fn find_file_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.exists() {
        return Some(direct);
    }
    let lower = name.to_ascii_lowercase();
    std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok()).find_map(|e| {
        (e.file_name().to_string_lossy().to_ascii_lowercase() == lower).then(|| e.path())
    })
}

pub(crate) async fn launch_win9x_game(
    app: &AppHandle,
    game: Game,
    id: i64,
    data_dir: &str,
    fullscreen: bool,
    per_game_config: &std::collections::HashMap<String, String>,
) -> Result<String, String> {
    let source = game.torrent_source.as_deref().unwrap_or("eXoWin9x");
    let inner = crate::commands::setup::collection_def(source)
        .map(|c| c.inner_folder)
        .unwrap_or("eXoWin9x");
    let torrent_root = super::games::collection_data_dir(data_dir, source).join(inner);
    let variant = game.dosbox_variant.clone().unwrap_or_else(|| "x98".to_string());
    let variant = variant.as_str();

    if variant == "pcbox" {
        return Err(format!(
            "'{}' needs PCBox, a Windows-only emulator Exodium does not ship yet.",
            game.title
        ));
    }

    if !win9x_support_ready(&torrent_root, Some(variant)) {
        return Err(
            "Windows 9x support files (OS images + emulators) are not installed yet. \
             They download automatically with the first Win9x game - check the \
             download progress, or re-download any Win9x game to restart it."
                .to_string(),
        );
    }

    let app_path = game
        .application_path
        .as_deref()
        .ok_or_else(|| format!("'{}' has no launcher path in the catalogue", game.title))?;
    // Conf dir: eXo/eXoWin9x/!win9x/<year>/<TitleDir>/ - the parent of the
    // per-game launcher bat.
    let rel_conf_dir = app_path
        .replace('\\', "/")
        .rsplit_once('/')
        .map(|(dir, _)| dir.to_string())
        .ok_or_else(|| format!("Unexpected launcher path: {}", app_path))?;
    let conf_dir = torrent_root.join(&rel_conf_dir);
    if !conf_dir.exists() {
        return Err(format!(
            "Game config folder not found: {}\nRe-install the game to restore it.",
            conf_dir.display()
        ));
    }

    // Auto-extract the game ZIP on first launch (imported installs may still
    // be zipped - mirrors the Staging path's behavior).
    let shortcode = game.shortcode.as_deref().unwrap_or("");
    if !shortcode.is_empty() {
        let game_dir = torrent_root.join(super::games::collection_rel_game_dir(
            source,
            shortcode,
            Some(app_path),
        ));
        if !game_dir.exists() {
            let game_name = Some(app_path)
                .and_then(crate::commands::setup::game_name_from_app_path)
                .unwrap_or_else(|| game.title.clone());
            let zip = torrent_root.join(super::games::collection_rel_zip(
                source,
                &game_name,
                Some(app_path),
            ));
            if zip.exists() {
                log::info!("Auto-extracting {} before launch", zip.display());
                let dest = zip.parent().map(PathBuf::from).unwrap_or_else(|| torrent_root.clone());
                let extract = tauri::async_runtime::spawn_blocking(move || {
                    super::games::extract_game_zip(&zip, &dest)
                })
                .await
                .map_err(|e| format!("extraction task failed: {e}"))?;
                extract.map_err(|e| format!("Failed to extract game before launch: {e}"))?;
            } else {
                return Err(format!(
                    "Game files not found for '{}'. The game may need to be re-downloaded.",
                    game.title
                ));
            }
        }
    }

    // Working dir is <torrent_root>/eXo - every relative path in the confs
    // and eXo's own launch bats resolves from there.
    let exo_dir = torrent_root.join("eXo");

    if variant.starts_with("86box") {
        launch_86box(app, game, id, &exo_dir, &conf_dir, variant, fullscreen)
    } else {
        launch_dosbox_x(
            app,
            game,
            id,
            &torrent_root,
            &exo_dir,
            &conf_dir,
            fullscreen,
            per_game_config,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn launch_dosbox_x(
    app: &AppHandle,
    game: Game,
    id: i64,
    torrent_root: &Path,
    exo_dir: &Path,
    conf_dir: &Path,
    fullscreen: bool,
    per_game_config: &std::collections::HashMap<String, String>,
) -> Result<String, String> {
    let Some(engine) = resolve_dosbox_x(app, torrent_root) else {
        return Err(if cfg!(target_os = "linux") {
            "DOSBox-X is required for Windows 9x games but was not found. \
             Install it via your package manager or Flatpak \
             (com.dosbox_x.DOSBox-X) and try again."
                .to_string()
        } else {
            "DOSBox-X is required for Windows 9x games but was not found. \
             Re-run the app installer or place dosbox-x on your PATH."
                .to_string()
        });
    };

    let play_conf = find_file_ci(conf_dir, "play.conf")
        .ok_or_else(|| format!("play.conf not found in {}", conf_dir.display()))?;
    let options_conf = exo_dir.join("emulators/dosbox/options9x.conf");
    let base_conf = exo_dir.join("emulators/dosbox/x98/dosbox-x.conf");

    let (mut cmd, bin) = engine.command(torrent_root);
    cmd.current_dir(exo_dir);
    // eXo's own x98 exe runs in portable mode and auto-loads the base conf
    // sitting next to it; any other build needs it passed explicitly, FIRST,
    // so play.conf layers on top exactly as authored.
    let is_exo_build = matches!(
        &engine,
        EngineCmd::Direct(b) if b.starts_with(exo_dir)
    );
    if !is_exo_build && base_conf.exists() {
        cmd.arg("-conf").arg(&base_conf);
    }
    cmd.arg("-conf").arg(&play_conf);
    if options_conf.exists() {
        cmd.arg("-conf").arg(&options_conf);
    }

    // User preference overrides, applied last so they win. DOSBox-X shares
    // the [sdl] fullscreen key with vanilla DOSBox; glshader does not apply.
    let mut frag = format!("[sdl]\nfullscreen = {}\n", fullscreen);
    if let Some(custom) = per_game_config.get("custom_conf") {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            frag.push('\n');
            frag.push_str(trimmed);
            frag.push('\n');
        }
    }
    let frag_path = super::games::launch_conf_dir(app)?.join(format!("win9x_overrides_{}.conf", id));
    std::fs::write(&frag_path, &frag).map_err(|e| format!("Failed to write override conf: {e}"))?;
    cmd.arg("-conf").arg(&frag_path);

    cmd.arg("-nomenu");
    if cfg!(windows) {
        cmd.arg("-noconsole");
    }

    log::info!(
        "Launching Win9x game {} via DOSBox-X ({})",
        game.title,
        bin.display()
    );
    super::games::spawn_emulator_and_track(cmd, &bin, &game, id)
}

fn launch_86box(
    app: &AppHandle,
    game: Game,
    id: i64,
    exo_dir: &Path,
    conf_dir: &Path,
    variant: &str,
    fullscreen: bool,
) -> Result<String, String> {
    let Some(engine) = resolve_86box(app) else {
        return Err(
            "86Box is required for this game but was not found. Re-run the app \
             installer or place 86Box on your PATH."
                .to_string(),
        );
    };

    let emul_dir = exo_dir.join("emulators/86Box98");
    let (child_name, parent_name, cfg_name) = e86box_variant_files(variant);

    // Recreate the disposable C: drive: a fresh differencing child of the
    // shared parent OS image, exactly what eXo's makevhd.exe does per launch.
    // Saves are unaffected - they live on the game's own VHD (drive D:).
    let child = emul_dir.join(child_name);
    if child.exists() {
        std::fs::remove_file(&child).map_err(|e| e.to_string())?;
    }
    let parent = emul_dir.join("parent").join(parent_name);
    crate::vhd::create_differencing(&child, &parent, &format!(r".\parent\{}", parent_name))?;

    // The per-game cfg is copied over the emulator's play.cfg (it references
    // the child VHD and the game's own VHD by relative path).
    let game_cfg = find_file_ci(conf_dir, cfg_name)
        .ok_or_else(|| format!("{} not found in {}", cfg_name, conf_dir.display()))?;
    let active_cfg = emul_dir.join("play.cfg");
    std::fs::copy(&game_cfg, &active_cfg).map_err(|e| e.to_string())?;

    let (mut cmd, bin) = engine.command(exo_dir);
    cmd.current_dir(exo_dir)
        .arg("-c")
        .arg(&active_cfg)
        // vmpath: where 86Box keeps/finds nvr state and (since v4) also
        // looks for roms/ - the extracted 86Box98 tree ships both.
        .arg("-P")
        .arg(&emul_dir);
    if fullscreen {
        cmd.arg("-F");
    }
    // AppImages need FUSE; the env var makes them self-extract when the
    // host has none (common in containers/minimal distros).
    if cfg!(target_os = "linux") {
        cmd.env("APPIMAGE_EXTRACT_AND_RUN", "1");
    }

    log::info!(
        "Launching Win9x game {} via 86Box ({}, variant {})",
        game.title,
        bin.display(),
        variant
    );
    super::games::spawn_emulator_and_track(cmd, &bin, &game, id)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Win9xSupportStatus {
    /// "ready" | "downloading" | "missing"
    pub phase: String,
    /// Download progress 0..1 while phase == "downloading".
    pub progress: f32,
}

/// Whether the emulator a Win9x game needs is resolvable on this machine.
/// The launcher's own resolver answers, so the note in the detail panel can
/// never disagree with what launch would actually do. Mainly a Linux
/// concern: DOSBox-X has no official Linux binaries, so PATH/Flatpak may
/// genuinely be empty there.
#[tauri::command]
pub async fn win9x_engine_available(
    app: AppHandle,
    db_state: State<'_, super::DbState>,
    variant: Option<String>,
) -> Result<bool, String> {
    let variant = variant.unwrap_or_else(|| "x98".to_string());
    if variant == "pcbox" {
        return Ok(false);
    }
    if variant.starts_with("86box") {
        return Ok(resolve_86box(&app).is_some());
    }
    let data_dir = {
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        crate::db::queries::get_config(&conn, "data_dir")
            .map_err(|e| e.to_string())?
            .unwrap_or_default()
    };
    let inner = crate::commands::setup::collection_def("eXoWin9x")
        .map(|c| c.inner_folder)
        .unwrap_or("eXoWin9x");
    let torrent_root = super::games::collection_data_dir(&data_dir, "eXoWin9x").join(inner);
    Ok(resolve_dosbox_x(&app, &torrent_root).is_some())
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
