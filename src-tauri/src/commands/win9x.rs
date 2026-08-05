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
        add_parent_case_aliases(&dest_root);
        Ok(extracted)
    })();

    let _ = std::fs::remove_file(&tmp_path);
    let _ = std::fs::remove_dir_all(&staging_root);
    result
}

/// The pack's play.confs reference the x98 parent VHDs in inconsistent case
/// (`win98jap` / `Win98Jap`, `Win95dx8` / `win95Dx8` / `Win95DX8`), which is
/// invisible on Windows/macOS but breaks ~24 games on Linux's case-sensitive
/// filesystems. Symlink every observed conf spelling to the real file. No-op
/// for aliases that already exist and on non-Unix platforms.
fn add_parent_case_aliases(dest_root: &Path) {
    #[cfg(unix)]
    {
        let parent_dir = dest_root.join("emulators/dosbox/x98/parent");
        const ALIASES: [(&str, &str); 5] = [
            ("win98jap.vhd", "win98Jap.vhd"),
            ("Win98Jap.vhd", "win98Jap.vhd"),
            ("Win95dx8.vhd", "Win95DX8.vhd"),
            ("win95Dx8.vhd", "Win95DX8.vhd"),
            ("win98chinese.vhd", "Win98Chinese.vhd"),
        ];
        for (alias, target) in ALIASES {
            let link = parent_dir.join(alias);
            if parent_dir.join(target).exists() && !link.exists() {
                if let Err(e) = std::os::unix::fs::symlink(target, &link) {
                    log::warn!("Failed to create parent-VHD case alias {}: {}", alias, e);
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dest_root;
    }
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

/// The zip a `MOUNT <letter> "<...>.zip"` line points at, when that zip wraps
/// everything in exactly ONE top-level directory.
///
/// Background: DOSBox-X converts mounted host drives into emulated FAT disks
/// when a guest OS boots (`convertdrivefat`, on by default), so the mount IS
/// visible in Windows - but at whatever depth the zip has. eXo's convention
/// is files at the zip root (the game's desktop shortcut points straight at
/// `E:\<GAME>.EXE`); a zip that wraps them in a folder puts the executable one
/// level too deep and the shortcut dies with "drive or network connection is
/// unavailable". Seen with Chinese Checkers (CC32.zip -> `CC32/CCHECK11.EXE`,
/// shortcut `E:\CCHECK11.EXE`) after eXo repackaged a newer game build.
fn zip_wrapper_dir(zip_path: &Path) -> Option<String> {
    let file = std::fs::File::open(zip_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut wrapper: Option<String> = None;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).ok()?;
        let name = entry.name().replace('\\', "/");
        let top = name.split('/').next()?.to_string();
        // A file at the root means the zip is already laid out as eXo's
        // convention expects - leave it alone.
        if !name[top.len()..].starts_with('/') {
            return None;
        }
        match &wrapper {
            Some(w) if *w != top => return None,
            Some(_) => {}
            None => wrapper = Some(top),
        }
    }
    wrapper
}

/// Rewrite `MOUNT <letter> "<...>.zip"` to mount the zip's inner directory
/// instead, extracting it next to the zip once. Only for zips whose entries
/// all sit under a single top-level directory (see `zip_wrapper_dir`); every
/// other mount line is left untouched.
fn unwrap_single_dir_zip_mounts(conf: &str, exo_dir: &Path) -> String {
    let mount_re = |line: &str| -> Option<(String, String)> {
        let trimmed = line.trim_start();
        let rest = trimmed.strip_prefix("MOUNT ").or_else(|| trimmed.strip_prefix("mount "))?;
        let (letter, target) = rest.trim_start().split_once(char::is_whitespace)?;
        let target = target.trim().trim_matches('"');
        target
            .to_ascii_lowercase()
            .ends_with(".zip")
            .then(|| (letter.trim().to_string(), target.to_string()))
    };

    conf.lines()
        .map(|line| {
            let Some((letter, target)) = mount_re(line) else {
                return line.to_string();
            };
            let zip_path = if Path::new(&target).is_absolute() {
                PathBuf::from(&target)
            } else {
                exo_dir.join(target.trim_start_matches("./"))
            };
            let Some(wrapper) = zip_wrapper_dir(&zip_path) else {
                return line.to_string();
            };
            let dest = zip_path.with_extension("exodium_mount");
            let inner = dest.join(&wrapper);
            if !inner.is_dir() {
                let Ok(file) = std::fs::File::open(&zip_path) else {
                    return line.to_string();
                };
                let extracted = zip::ZipArchive::new(file)
                    .and_then(|mut a| a.extract(&dest))
                    .is_ok();
                if !extracted || !inner.is_dir() {
                    let _ = std::fs::remove_dir_all(&dest);
                    return line.to_string();
                }
                log::info!(
                    "Unwrapped {} - mounting its '{}' directory so the game's files sit at the drive root",
                    zip_path.display(),
                    wrapper
                );
            }
            format!("MOUNT {} \"{}\"", letter, inner.display())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Can this process capture raw packets? DOSBox-X's `pcap` backend bridges
/// the guest NIC onto a real interface, which is what eXo's remote-multiplayer
/// titles need: they dial a PPTP tunnel to eXo's IPX server, and PPTP rides on
/// GRE, a protocol user-mode NAT cannot carry.
///
/// Windows gets this for free (the pack's setup installs npcap). On macOS the
/// `/dev/bpf*` nodes are root-only unless Wireshark's ChmodBPF helper is
/// installed; on Linux it takes CAP_NET_RAW. Both are one-time, user-side
/// decisions we must not make for them - so we detect and adapt instead.
#[cfg(unix)]
fn can_capture_packets() -> bool {
    #[cfg(target_os = "macos")]
    {
        (0..4).any(|i| {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/dev/bpf{i}"))
                .is_ok()
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        // AF_PACKET/SOCK_RAW is exactly the privilege libpcap needs.
        // SAFETY: plain syscall; the fd is closed immediately.
        unsafe {
            let fd = libc::socket(libc::AF_PACKET, libc::SOCK_RAW, 0);
            if fd < 0 {
                return false;
            }
            libc::close(fd);
            true
        }
    }
}

/// The host interface to bridge onto, i.e. the one carrying the default route.
/// eXo's confs name a Windows adapter (`realnic = Rea…`), which means nothing
/// here, so pcap needs an explicit local answer.
#[cfg(unix)]
fn default_interface() -> Option<String> {
    let (prog, args): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("route", &["-n", "get", "default"])
    } else {
        ("ip", &["-o", "route", "get", "1.1.1.1"])
    };
    let out = Command::new(prog).args(args).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    if cfg!(target_os = "macos") {
        text.lines()
            .find_map(|l| l.trim().strip_prefix("interface:"))
            .map(|s| s.trim().to_string())
    } else {
        let mut parts = text.split_whitespace();
        while let Some(word) = parts.next() {
            if word == "dev" {
                return parts.next().map(|s| s.to_string());
            }
        }
        None
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Win9xNetworkStatus {
    /// True once the host lets us bridge the guest onto a real interface.
    pub enabled: bool,
    /// False where nothing can be done from inside the app (Flatpak DOSBox-X,
    /// missing PolicyKit) - the UI then shows `manual_hint` instead of a button.
    pub can_enable: bool,
    /// Platform-specific one-liner for what enabling actually grants.
    pub detail: String,
    /// Command to run by hand when `can_enable` is false.
    pub manual_hint: Option<String>,
}

/// Whether eXo's remote-multiplayer titles can reach their IPX server, and
/// whether Exodium can obtain that permission for the user.
#[tauri::command]
pub async fn win9x_network_status() -> Result<Win9xNetworkStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let enabled = can_capture_packets();
        Ok(Win9xNetworkStatus {
            enabled,
            can_enable: !enabled,
            detail: if enabled {
                "Multiplayer games can reach eXo's IPX server.".into()
            } else {
                "Multiplayer games cannot connect: bridging the emulated network card \
                 needs packet-capture access, which macOS keeps closed by default."
                    .into()
            },
            manual_hint: None,
        })
    }
    #[cfg(target_os = "linux")]
    {
        let enabled = can_capture_packets();
        let pkexec = binary_exists_on_path("pkexec");
        Ok(Win9xNetworkStatus {
            enabled,
            can_enable: !enabled && pkexec,
            detail: if enabled {
                "Multiplayer games can reach eXo's IPX server.".into()
            } else {
                "Multiplayer games cannot connect: bridging the emulated network card \
                 needs the CAP_NET_RAW capability on DOSBox-X."
                    .into()
            },
            manual_hint: (!enabled && !pkexec)
                .then(|| "sudo setcap cap_net_raw+ep $(which dosbox-x)".to_string()),
        })
    }
    #[cfg(windows)]
    {
        Ok(Win9xNetworkStatus {
            enabled: true,
            can_enable: false,
            detail: "Multiplayer uses npcap, which ships with the eXoWin9x support files."
                .into(),
            manual_hint: None,
        })
    }
}

/// Should launching this game offer to turn multiplayer on first?
///
/// True only when all four hold: it is a Win9x game, its conf boots one of
/// eXo's network parent images (`W98-C-Net`/`-Net2`, 67 titles - the others
/// have no online mode to enable), the host cannot capture packets yet, and
/// the user has not already answered for good. The prompt belongs on Play
/// rather than in Settings because that is where the missing feature is about
/// to be noticed.
#[tauri::command]
pub async fn win9x_needs_network_prompt(
    db_state: State<'_, super::DbState>,
    id: i64,
) -> Result<bool, String> {
    #[cfg(windows)]
    {
        let _ = (&db_state, id);
        return Ok(false);
    }
    #[cfg(not(windows))]
    {
        if can_capture_packets() {
            return Ok(false);
        }
        let (game, data_dir, asked) = {
            let conn = db_state.0.lock().map_err(|e| e.to_string())?;
            let game = crate::db::queries::fetch_game_by_id(&conn, id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Game {id} not found"))?;
            let data_dir = crate::db::queries::get_config(&conn, "data_dir")
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            let asked = crate::db::queries::get_config(&conn, "win9x_network_prompt")
                .map_err(|e| e.to_string())?;
            (game, data_dir, asked)
        };
        if asked.as_deref() == Some("off") {
            return Ok(false);
        }
        let source = game.torrent_source.as_deref().unwrap_or("eXoDOS");
        if !crate::commands::setup::collection_def(source).is_some_and(|c| c.year_subdirs) {
            return Ok(false);
        }
        let Some(app_path) = game.application_path.as_deref() else {
            return Ok(false);
        };
        let inner = crate::commands::setup::collection_def(source)
            .map(|c| c.inner_folder)
            .unwrap_or("eXoWin9x");
        let torrent_root = super::games::collection_data_dir(&data_dir, source).join(inner);
        let Some(conf_dir) = app_path
            .replace('\\', "/")
            .rsplit_once('/')
            .map(|(dir, _)| torrent_root.join(dir))
        else {
            return Ok(false);
        };
        let Some(play_conf) = find_file_ci(&conf_dir, "play.conf") else {
            return Ok(false);
        };
        let conf = std::fs::read_to_string(&play_conf).unwrap_or_default();
        Ok(conf.to_ascii_lowercase().contains("w98-c-net"))
    }
}

/// Remember that the user does not want to be asked about multiplayer again.
#[tauri::command]
pub async fn dismiss_win9x_network_prompt(
    db_state: State<'_, super::DbState>,
) -> Result<(), String> {
    let conn = db_state.0.lock().map_err(|e| e.to_string())?;
    crate::db::queries::set_config(&conn, "win9x_network_prompt", "off").map_err(|e| e.to_string())
}

/// Ask the operating system - not the user's shell - for the permission that
/// bridging needs. macOS shows its own authentication sheet; Linux shows
/// PolicyKit's. Nothing here runs without that dialog being accepted.
#[tauri::command]
pub async fn enable_win9x_network(app: AppHandle) -> Result<Win9xNetworkStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = &app;
        install_bpf_daemon_macos().await?;
    }
    #[cfg(target_os = "linux")]
    {
        grant_cap_net_raw_linux(&app).await?;
    }
    #[cfg(windows)]
    {
        let _ = &app;
    }
    win9x_network_status().await
}

/// Install a boot-time helper that hands this user the BPF devices.
///
/// Same shape as Wireshark's ChmodBPF, with one deliberate difference: the
/// nodes are chowned to the current user rather than opened up to a shared
/// `access_bpf` group. It is the narrower grant, and it takes effect
/// immediately - a new group membership would only apply after a re-login,
/// which reads as "the button did nothing".
#[cfg(target_os = "macos")]
async fn install_bpf_daemon_macos() -> Result<(), String> {
    let user = std::env::var("USER").map_err(|_| "cannot determine the current user")?;
    if !user.chars().all(|c| c.is_alphanumeric() || "._-".contains(c)) {
        return Err(format!("unexpected user name: {user}"));
    }
    let label = "com.redfox.exodium.bpf";
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>RunAtLoad</key><true/>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/sh</string><string>-c</string>
    <string>chown {user} /dev/bpf* &amp;&amp; chmod 600 /dev/bpf*</string>
  </array>
</dict>
</plist>
"#
    );
    let tmp_dir = std::env::temp_dir().join(format!("exodium_bpf_{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
    let tmp_plist = tmp_dir.join("daemon.plist");
    std::fs::write(&tmp_plist, plist).map_err(|e| e.to_string())?;

    let dest = format!("/Library/LaunchDaemons/{label}.plist");
    let script = tmp_dir.join("install.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nset -e\n\
             cp '{}' '{dest}'\n\
             chown root:wheel '{dest}'\n\
             chmod 644 '{dest}'\n\
             launchctl unload '{dest}' 2>/dev/null || true\n\
             launchctl load -w '{dest}'\n\
             chown {user} /dev/bpf* && chmod 600 /dev/bpf*\n",
            tmp_plist.display()
        ),
    )
    .map_err(|e| e.to_string())?;

    let out = Command::new("osascript")
        .arg("-e")
        .arg(format!(
            "do shell script \"/bin/sh {}\" with administrator privileges",
            script.display()
        ))
        .output()
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&tmp_dir);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        // -128 is the user dismissing the authentication sheet.
        if err.contains("-128") {
            return Err("cancelled".into());
        }
        return Err(format!("could not install the helper: {}", err.trim()));
    }
    Ok(())
}

/// Put CAP_NET_RAW on the DOSBox-X binary the launcher actually resolves.
#[cfg(target_os = "linux")]
async fn grant_cap_net_raw_linux(app: &AppHandle) -> Result<(), String> {
    let torrent_root = PathBuf::new();
    let bin = match resolve_dosbox_x(app, &torrent_root) {
        Some(EngineCmd::Direct(path)) => path,
        Some(EngineCmd::Flatpak(_)) => {
            return Err(
                "The Flatpak build of DOSBox-X cannot be granted packet access. Install \
                 DOSBox-X from your distribution's packages to use multiplayer."
                    .into(),
            )
        }
        None => return Err("DOSBox-X was not found on this system.".into()),
    };
    let bin = if bin.is_absolute() {
        bin
    } else {
        let out = Command::new("which").arg(&bin).output().map_err(|e| e.to_string())?;
        PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
    };
    let out = Command::new("pkexec")
        .arg("setcap")
        .arg("cap_net_raw+ep")
        .arg(&bin)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        // 126 is PolicyKit's "the dialog was dismissed".
        if out.status.code() == Some(126) {
            return Err("cancelled".into());
        }
        return Err(format!(
            "setcap failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Network-backend fragment appended to every DOSBox-X launch.
///
/// Windows keeps eXo's authored `pcap` setup verbatim. Elsewhere we bridge
/// with pcap when the host allows raw capture (then remote multiplayer works
/// as eXo intended) and otherwise fall back to slirp: user-mode NAT that
/// carries plain TCP/UDP, loads without a permission prompt, and above all
/// does not greet the player with an in-guest network error at boot.
fn ne2000_override() -> String {
    #[cfg(unix)]
    {
        if can_capture_packets() {
            if let Some(nic) = default_interface() {
                log::info!("Win9x networking: bridging the guest NIC onto {nic} (pcap)");
                return format!("[ne2000]\nbackend = pcap\n[ethernet, pcap]\nrealnic = {nic}\n");
            }
        }
        "[ne2000]\nbackend = slirp\n".to_string()
    }
    #[cfg(not(unix))]
    {
        String::new()
    }
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

    // One narrow exception to "the conf runs verbatim": `.\`-relative HOST
    // path tokens are rewritten to `./` form. DOSBox-X on POSIX opens
    // existing files through backslash paths fine, but CANNOT CREATE them -
    // `vhdmake` silently wrote nothing, every boot reused the shipped, dirty
    // child VHD, and games whose child isn't shipped at all (W95-C.vhd)
    // booted "Invalid system disk". Guest text is untouched - that is
    // rewrite_host_paths' contract (see the Win3x PATH lesson). A token is
    // only rewritten when its target (or, for files vhdmake will create, its
    // parent directory) exists under eXo/.
    let play_conf = {
        let content = std::fs::read_to_string(&play_conf)
            .map_err(|e| format!("Failed to read {}: {}", play_conf.display(), e))?;
        let patched = super::games::rewrite_host_paths(&content, &|body| {
            let fwd = body.replace('\\', "/");
            let target = exo_dir.join(&fwd);
            let creatable = target.parent().is_some_and(|p| p.is_dir());
            if target.exists() || creatable {
                format!("./{}", fwd)
            } else {
                format!(".\\{}", body)
            }
        });
        let patched = unwrap_single_dir_zip_mounts(&patched, exo_dir);
        let patched_path =
            super::games::launch_conf_dir(app)?.join(format!("win9x_play_{}.conf", id));
        std::fs::write(&patched_path, &patched)
            .map_err(|e| format!("Failed to write patched play.conf: {e}"))?;
        patched_path
    };

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
    // - windowresolution: eXo's options9x.conf default (1280x960) is in
    //   logical points and overflows a 1117-point MacBook screen with the
    //   image partly cut off; 1024x768 fits every common display.
    // - output opengl: the base conf's ttf/outputswitch combo is not
    //   user-resizable; opengl windows scale by dragging.
    // - ne2000 backend: see `ne2000_override`.
    let mut frag = format!(
        "[sdl]\nfullscreen = {}\nwindowresolution = 1024x768\noutput = opengl\n{}",
        fullscreen,
        ne2000_override()
    );
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

    // eXo's bats pass -nomenu; we deliberately keep the menu. DOSBox-X
    // 2025.02.01 renders the guest at a FIXED size and crops when the window
    // is dragged smaller (measured on macOS with opengl, surface and
    // openglpp alike - upstream issue #3661), so the Video menu is the only
    // runtime escape hatch: fullscreen scales correctly, and the output mode
    // can be switched to `surface`, which centres the whole guest screen
    // instead of cropping it. On macOS the menu lives in the global menu bar
    // and costs no window space.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_zip(path: &Path, entries: &[&str]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        for name in entries {
            if name.ends_with('/') {
                zip.add_directory(name.trim_end_matches('/'), opts).unwrap();
            } else {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(b"x").unwrap();
            }
        }
        zip.finish().unwrap();
    }

    #[test]
    fn wrapped_zip_mounts_its_inner_directory() {
        let dir = std::env::temp_dir().join(format!("exodium_zipmount_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // One top-level dir: the game's files sit one level too deep for the
        // desktop shortcut, so the mount is redirected at the inner dir.
        let wrapped = dir.join("CC32.zip");
        write_zip(&wrapped, &["CC32/", "CC32/CCHECK11.EXE"]);
        let out = unwrap_single_dir_zip_mounts(&format!("MOUNT e \"{}\"", wrapped.display()), &dir);
        assert!(out.ends_with("CC32.exodium_mount/CC32\""), "{out}");
        assert!(dir.join("CC32.exodium_mount/CC32/CCHECK11.EXE").is_file());

        // Files at the zip root are eXo's convention - left verbatim.
        let flat = dir.join("MpgDec20.zip");
        write_zip(&flat, &["license.txt", "MPGDEC.DLL"]);
        let line = format!("MOUNT e \"{}\"", flat.display());
        assert_eq!(unwrap_single_dir_zip_mounts(&line, &dir), line);

        // Non-zip mounts and other lines are never touched.
        let conf = "IMGMOUNT c ./x.vhd\nMOUNT e \"./games\"\nBOOT -l c";
        assert_eq!(unwrap_single_dir_zip_mounts(conf, &dir), conf);

        let _ = std::fs::remove_dir_all(&dir);
    }
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
