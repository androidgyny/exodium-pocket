import { createSignal, onMount, onCleanup, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Dialog } from "@ark-ui/solid/dialog";
import { Tooltip } from "@ark-ui/solid/tooltip";
import { Toggle } from "./components/Toggle";
import { Library } from "./pages/Library";
import { Setup } from "./pages/Setup";
import { SearchBar } from "./components/SearchBar";
import { WelcomeModal } from "./components/WelcomeModal";
import { SeedingConsentDialog } from "./components/SeedingConsentDialog";
import { needsSeedingConsent } from "./stores/seeding";
import { ContentPackSettings } from "./components/ContentPackSettings";
import { DownloadIndicator } from "./components/DownloadIndicator";
import { WindowFrame } from "./components/WindowFrame";
import { ToastContainer } from "./components/ToastContainer";
import {
  getSetupStatus,
  initDownloadManager,
  factoryReset,
  getConfig,
  setConfig,
  setSeedingEnabled,
  scanInstalledGames,
  openLogFolder,
  checkForUpdates,
} from "./api/tauri";
import { updateState, checkForAppUpdate, startUpdate, restartToUpdate } from "./stores/updater";
import { fetchGames } from "./stores/games";
import { applyNetworkMode, isOffline, loadNetworkMode } from "./stores/network";
import { loadThumbnailDir } from "./stores/thumbnails";
import { refreshInstalledPacks } from "./stores/contentPacks";
import { showToast } from "./stores/toasts";
import "./styles/main.css";
import { Button } from "./components/Button";

type AppPhase = "loading" | "setup" | "ready";

/** Friendly labels for collection IDs used in update toasts. */
const COLLECTION_LABEL: Record<string, string> = {
  eXoDOS: "eXoDOS",
  eXoDOS_GLP: "German Language Pack",
  eXoDOS_PLP: "Polish Language Pack",
  eXoDOS_SLP: "Spanish Language Pack",
};

/** Compare bundled-manifest infohashes against the per-collection hashes
 *  stored at last `init_download_manager`. A mismatch means the shipped
 *  catalogue moved ahead of the user's DB (typically after an Exodium
 *  upgrade). Phase 1 only notifies; applying the update is deferred until
 *  a non-destructive merge-import path exists. Suppress per-session via
 *  `sessionStorage` so users aren't re-toasted on every focus event. */
async function notifyCatalogUpdates() {
  try {
    if (sessionStorage.getItem("catalog_update_notified") === "1") { return; }
    const info = await checkForUpdates();
    if (!info.updates || info.updates.length === 0) { return; }
    const totalNew = info.updates.reduce((sum, u) => sum + (u.new_game_count ?? 0), 0);
    const cols = info.updates
      .map((u) => COLLECTION_LABEL[u.collection] ?? u.collection)
      .join(", ");
    showToast(
      totalNew > 0
        ? `Catalogue update available: ${totalNew.toLocaleString()} games in ${cols}`
        : `Catalogue update available for ${cols}`,
      "info",
      {
        detail: "A reimport flow will be added in a future release. For now, run Factory Reset to refresh.",
        durationMs: 12000,
      },
    );
    sessionStorage.setItem("catalog_update_notified", "1");
  } catch (e) {
    console.warn("[updates] check_for_updates failed:", e);
  }
}

function App() {
  const [phase, setPhase] = createSignal<AppPhase>("loading");
  const [showSettings, setShowSettings] = createSignal(false);
  const [settingsTab, setSettingsTab] = createSignal<"general" | "packs">("general");
  const [showWelcomeModal, setShowWelcomeModal] = createSignal(false);
  const [showSeedingConsent, setShowSeedingConsent] = createSignal(false);
  const [dataDir, setDataDir] = createSignal("");
  const [resetError, setResetError] = createSignal("");
  const [logOpenError, setLogOpenError] = createSignal("");
  const [resetting, setResetting] = createSignal(false);

  // Derived: the actual game storage folder shown to the user.
  const gameFolderPath = () => {
    const dir = dataDir();
    if (!dir) return "";
    const sep = dir.includes("\\") ? "\\" : "/";
    return dir.replace(/[/\\]$/, "") + sep + "eXoDOS";
  };

  onMount(() => {
    // Suppress the webview's native right-click menu app-wide (Inspect,
    // Reload, ... don't belong in a launcher). Component-level custom menus
    // (GameCard) hook the same event and render their own UI, unaffected.
    // Kept enabled in dev so Inspect Element stays reachable.
    if (!import.meta.env.DEV) {
      const suppress = (e: MouseEvent) => {
        // Editable fields keep the native menu - it carries cut/copy/paste.
        const t = e.target as HTMLElement | null;
        if (t?.closest('input, textarea, [contenteditable="true"]')) { return; }
        e.preventDefault();
      };
      document.addEventListener("contextmenu", suppress);
      onCleanup(() => document.removeEventListener("contextmenu", suppress));
    }
  });

  onMount(async () => {
    try {
      const status = await getSetupStatus();
      if (status.ready) {
        setPhase("ready");
        await loadNetworkMode();
        setShowSeedingConsent(await needsSeedingConsent());
        try {
          await initDownloadManager();
        } catch (e) {
          console.error("Failed to init download manager:", e);
        }
        const dir = await getConfig("data_dir");
        if (dir) { setDataDir(dir); }
        loadThumbnailDir();
        refreshInstalledPacks();
        // Update checks are network calls; offline mode means none are made.
        if (!isOffline()) {
          notifyCatalogUpdates();
          checkForAppUpdate();
          // Setup skips the content-pack offer while offline without marking it
          // seen, so pick it up on the first online start instead of dropping
          // it silently.
          getConfig("welcome_seen").then((seen) => {
            if (seen !== "1") { setShowWelcomeModal(true); }
          }).catch(() => {});
        }
      } else {
        setPhase("setup");
      }
    } catch {
      setPhase("setup");
    }
  });

  const handleSetupComplete = async () => {
    setPhase("ready");
    await loadNetworkMode();
    const dir = await getConfig("data_dir");
    if (dir) { setDataDir(dir); }
    loadThumbnailDir();
    refreshInstalledPacks();
    fetchGames();
    if (!isOffline()) { checkForAppUpdate(); }

    // Show the welcome modal if the user hasn't seen it yet - but never in
    // offline mode: it exists to offer downloads, which the user just declined.
    // `welcome_seen` stays unwritten, and the startup path above re-offers it on
    // the first online session; the packs are in Settings either way.
    const welcomeSeen = await getConfig("welcome_seen");
    if (welcomeSeen !== "1" && !isOffline()) {
      setShowWelcomeModal(true);
    }
  };

  const handleChangeDataDir = async () => {
    const selected = await open({ title: "Select new data directory", directory: true });
    if (!selected) return;
    await setConfig("data_dir", selected);
    setDataDir(selected);
    await initDownloadManager();
  };

  const [scanning, setScanning] = createSignal(false);
  const [scanResult, setScanResult] = createSignal("");

  const handleRescan = async () => {
    setScanning(true);
    setScanResult("");
    try {
      const count = await scanInstalledGames();
      setScanResult(`${count} game${count !== 1 ? "s" : ""} marked as installed`);
      fetchGames();
    } catch (e) {
      setScanResult(`Error: ${e}`);
    } finally {
      setScanning(false);
    }
  };

  const [showResetDialog, setShowResetDialog] = createSignal(false);
  const [deleteGameData, setDeleteGameData] = createSignal(false);

  // Global launch-time overrides (persisted via DB config table, read by the
  // Rust launch_game command, layered as a last-wins -conf fragment).
  // Initial values MUST mirror the backend defaults in launch_game (unset
  // global_glshader means crt-auto there), so the UI is truthful even
  // before loadGameDefaults resolves.
  const [crtAuto, setCrtAuto] = createSignal(true);
  const [defaultFullscreen, setDefaultFullscreen] = createSignal(false);

  const [seeding, setSeeding] = createSignal(false);
  const loadGameDefaults = async () => {
    try {
      const [shader, fs, seed] = await Promise.all([
        getConfig("global_glshader"),
        getConfig("default_fullscreen"),
        getConfig("seeding_enabled"),
      ]);
      setCrtAuto(shader == null || shader === "crt-auto");
      setDefaultFullscreen(fs === "fullscreen");
      // Opt-in: only an explicit "1" means sharing (mirrors the Rust side).
      setSeeding(seed === "1");
    } catch (e) {
      console.warn("[settings] failed to load game defaults:", e);
    }
  };

  // Opening goes through this helper because Ark's onOpenChange only fires
  // for component-initiated changes (Escape, backdrop, close button) - not
  // when we flip the controlled `open` prop, so init logic there never ran.
  const openSettings = () => {
    loadGameDefaults();
    loadNetworkMode();
    setLogOpenError("");
    setModeError("");
    setSettingsTab("general");
    setShowSettings(true);
  };

  const [switchingMode, setSwitchingMode] = createSignal(false);
  const [modeError, setModeError] = createSignal("");

  /** Flipping this rebuilds the torrent state: going offline drops every
   *  manager (which shuts the librqbit session down), going online creates a
   *  fresh session and re-adopts any interrupted downloads. */
  const handleToggleOnline = async (online: boolean) => {
    setModeError("");
    setSwitchingMode(true);
    try {
      const stopped = await applyNetworkMode(online ? "live" : "offline");
      // Two different fates, so they get two different sentences: torrent
      // downloads keep their file selection and pick up again, pack installs
      // are plain HTTP transfers that have to be restarted by hand.
      const notes: string[] = [];
      if (stopped.downloads > 0) {
        notes.push(`${stopped.downloads} game download${stopped.downloads === 1 ? "" : "s"} paused - resumes when you go back online`);
      }
      if (stopped.packs > 0) {
        notes.push(`${stopped.packs} content pack download${stopped.packs === 1 ? "" : "s"} cancelled`);
      }
      showToast(
        online ? "Online mode - downloads enabled" : "Offline mode - torrent client stopped",
        "info",
        notes.length > 0 ? { detail: `${notes.join("; ")}.` } : {},
      );
      // Offline installs are never asked about seeding, so going online is
      // where an old install finally owes the answer.
      if (online) { setShowSeedingConsent(await needsSeedingConsent()); }
    } catch (e) {
      setModeError(`Could not switch mode: ${e}`);
    } finally {
      setSwitchingMode(false);
    }
  };

  /** The answer from the one-time consent dialog. Errors propagate so the
   *  dialog can stay open and say so - a failed write here would otherwise
   *  leave the key unset and ask again on the next start. */
  const handleSeedingConsent = async (enabled: boolean) => {
    await setSeedingEnabled(enabled);
    setSeeding(enabled);
    setShowSeedingConsent(false);
    showToast(
      enabled ? "Sharing with other players is on" : "Sharing with other players is off",
      "info",
      { detail: "Change it any time in Settings → Network." },
    );
  };

  const handleToggleSeeding = async (next: boolean) => {
    setSeeding(next);
    try {
      await setSeedingEnabled(next);
    } catch (e) {
      console.error("[settings] failed to save seeding preference:", e);
      setSeeding(!next);
    }
  };

  const handleToggleCrtAuto = async (next: boolean) => {
    setCrtAuto(next);
    try {
      await setConfig("global_glshader", next ? "crt-auto" : "default");
    } catch (e) {
      console.error("[settings] failed to save global_glshader:", e);
      setCrtAuto(!next); // revert on failure
    }
  };

  const handleToggleFullscreen = async (next: boolean) => {
    setDefaultFullscreen(next);
    try {
      await setConfig("default_fullscreen", next ? "fullscreen" : "window");
    } catch (e) {
      console.error("[settings] failed to save default_fullscreen:", e);
      setDefaultFullscreen(!next);
    }
  };

  const handleOpenLogFolder = async () => {
    setLogOpenError("");
    try {
      await openLogFolder();
    } catch (e) {
      setLogOpenError(`Could not open log folder: ${e}`);
    }
  };

  const confirmReset = async () => {
    const doDelete = deleteGameData();
    setShowResetDialog(false);
    setDeleteGameData(false);
    setResetError("");
    // Block the UI immediately so the user doesn't see a stale Library frame
    // while the reset (which may take seconds - DB clear + recursive deletes
    // for game folders + content packs) runs to completion. Closing the
    // settings dialog FIRST then setting `resetting()` puts the overlay over
    // whatever was behind the dialog (Library or Setup).
    setShowSettings(false);
    setResetting(true);
    console.log("[reset] calling factoryReset, deleteGameData=", doDelete);
    try {
      await factoryReset(doDelete);
      console.log("[reset] factoryReset succeeded, switching to setup");
      setPhase("setup");
      setDataDir("");
    } catch (e) {
      console.error("[reset] factoryReset failed:", e);
      setResetError(`Reset failed: ${e}`);
      setShowSettings(true);
    } finally {
      setResetting(false);
    }
  };

  return (
    <>
      <WindowFrame />

      <Show when={phase() === "loading"}>
        <div class="loading">Loading...</div>
      </Show>

      <Show when={phase() === "setup"}>
        <Setup onComplete={handleSetupComplete} />
      </Show>

      <Show when={phase() === "ready"}>
        <div class="top-bar">
          <div class="top-bar-center">
            <SearchBar />
          </div>
          <div class="top-bar-actions">
            <Show when={updateState()}>
              <button
                class={`update-pill update-pill-${updateState()!.status}`}
                disabled={updateState()!.status === "downloading"}
                title={
                  updateState()!.status === "available"
                    ? `Download and install Exodium ${updateState()!.version}`
                    : updateState()!.status === "ready"
                      ? "Restart Exodium to finish updating"
                      : "Downloading update…"
                }
                onClick={() =>
                  updateState()!.status === "available" ? startUpdate()
                  : updateState()!.status === "ready" ? restartToUpdate()
                  : undefined
                }
              >
                {updateState()!.status === "available" && `⬆ Update ${updateState()!.version}`}
                {updateState()!.status === "downloading" && "Downloading…"}
                {updateState()!.status === "ready" && "↻ Restart to update"}
              </button>
            </Show>
            {/* Offline is a mode with visible consequences (no downloads, no
                videos, no sharing), so it says so permanently rather than only
                inside Settings. */}
            <Show when={isOffline()}>
              <Tooltip.Root openDelay={300}>
                <Tooltip.Trigger asChild={(props) =>
                  <button {...props()} class="offline-badge" onClick={openSettings}>
                    <span class="offline-badge-dot" /> Offline
                  </button>
                } />
                <Portal><Tooltip.Positioner><Tooltip.Content class="ark-tooltip">
                  Torrent client is off - no downloads or previews. Click to change.
                </Tooltip.Content></Tooltip.Positioner></Portal>
              </Tooltip.Root>
            </Show>
            <DownloadIndicator />
            <Tooltip.Root openDelay={400}>
              <Tooltip.Trigger asChild={(props) =>
                <button {...props()} class="icon-btn icon-btn-heart" onClick={() => openUrl("https://ko-fi.com/tvollstaedt")}>
                  &#9829;
                </button>
              } />
              <Portal><Tooltip.Positioner><Tooltip.Content class="ark-tooltip">Support Exodium</Tooltip.Content></Tooltip.Positioner></Portal>
            </Tooltip.Root>
            <Tooltip.Root openDelay={400}>
              <Tooltip.Trigger asChild={(props) =>
                <button {...props()} class="icon-btn" onClick={openSettings}>
                  &#9881;
                </button>
              } />
              <Portal><Tooltip.Positioner><Tooltip.Content class="ark-tooltip">Settings</Tooltip.Content></Tooltip.Positioner></Portal>
            </Tooltip.Root>
          </div>
        </div>

        <Show when={showSettings()}>
        <Dialog.Root open={showSettings()} onOpenChange={(e) => setShowSettings(e.open)}>
          <Portal>
            <Dialog.Backdrop class="ark-dialog-backdrop" />
            <Dialog.Positioner class="ark-dialog-positioner">
              <Dialog.Content class="ark-dialog-content ark-dialog-settings">
                <Dialog.Title class="ark-dialog-title">Settings</Dialog.Title>
                <div class="settings-tabs">
                  <button
                    class={`settings-tab ${settingsTab() === "general" ? "active" : ""}`}
                    onClick={() => setSettingsTab("general")}
                  >General</button>
                  <button
                    class={`settings-tab ${settingsTab() === "packs" ? "active" : ""}`}
                    onClick={() => setSettingsTab("packs")}
                  >Content Packs</button>
                </div>

                <div class="settings-tab-body">
                  <Show when={settingsTab() === "general"}>
                    <div class="settings-body">
                      <section class="settings-section">
                        <h3 class="settings-section-title">Library</h3>
                        <div class="setting-row">
                          <span class="setting-label">Game folder</span>
                          <span class="setting-value">{gameFolderPath() || "Not set"}</span>
                          <Button variant="small" onClick={handleChangeDataDir}>Change</Button>
                        </div>
                        <div class="setting-row">
                          <span class="setting-label">Installed games</span>
                          <span class="setting-hint">Re-scan disk to detect already-downloaded games</span>
                          <Button variant="small" onClick={handleRescan} disabled={scanning()}>
                            {scanning() ? "Scanning…" : "Scan"}
                          </Button>
                        </div>
                        <Show when={scanResult()}>
                          <div class="setting-hint" style="margin-top:4px">{scanResult()}</div>
                        </Show>
                      </section>

                      <section class="settings-section">
                        <h3 class="settings-section-title">Game Defaults</h3>
                        <p class="settings-section-hint">Applied as a last-wins DOSBox config on every launch. Overrides per-game settings without modifying eXoDOS's bundled configs.</p>
                        <Toggle
                          checked={crtAuto()}
                          onChange={handleToggleCrtAuto}
                          label="Auto CRT shaders"
                          hint="DOSBox Staging picks a CRT shader matched to each game's video mode and your display resolution."
                        />
                        <Toggle
                          checked={defaultFullscreen()}
                          onChange={handleToggleFullscreen}
                          label="Launch in fullscreen"
                          hint="Start every game fullscreen instead of windowed. Alt+Enter still toggles at runtime."
                        />
                      </section>

                      <section class="settings-section">
                        <h3 class="settings-section-title">Network</h3>
                        <p class="settings-section-hint">Games are downloaded from the eXoDOS BitTorrent swarm.</p>
                        {/* A switch, not a checkbox: this one starts and stops
                            a network service, which is a mode rather than an
                            option among several. */}
                        <Toggle
                          checked={!isOffline()}
                          disabled={switchingMode()}
                          onChange={handleToggleOnline}
                          label={isOffline() ? "Offline mode" : "Online mode"}
                          hint={isOffline()
                            ? "The torrent client stays off - Exodium only launches games already on disk."
                            : "Games, previews and content packs are downloaded from the eXoDOS torrents."}
                        />
                        <Show when={modeError()}>
                          <div class="setting-hint" style="margin-top:4px">{modeError()}</div>
                        </Show>
                        {/* Kept visible but inert while offline: hiding it
                            would look like the setting disappeared, and its
                            state still matters for when you go back online. */}
                        <Toggle
                          checked={seeding() && !isOffline()}
                          disabled={isOffline()}
                          onChange={handleToggleSeeding}
                          label="Share with other players (seeding)"
                          hint={isOffline()
                            ? "Nothing is shared while offline. Your choice is kept for when you switch back."
                            : "Uploads parts of the games you have to other users while Exodium runs. Keeps the collection alive - but distributing game files carries legal risk in some countries. Off caps upload at 1 KB/s."}
                        />
                      </section>

                      <section class="settings-section">
                        <h3 class="settings-section-title">Diagnostics</h3>
                        <p class="settings-section-hint">If a download stalls or the app misbehaves, share <code>exodium.log</code> from the folder.</p>
                        <div class="setting-row">
                          <span class="setting-label">Log folder</span>
                          <span class="setting-hint">Open in your file explorer</span>
                          <Button variant="small" onClick={handleOpenLogFolder}>Open</Button>
                        </div>
                        <Show when={logOpenError()}>
                          <div class="error" style="margin-top:6px">{logOpenError()}</div>
                        </Show>
                      </section>

                      <section class="settings-section">
                        <h3 class="settings-section-title">Support Exodium</h3>
                        <p class="settings-section-hint">Exodium is free and open source. If it's useful to you, you can support its development.</p>
                        <div class="setting-row">
                          <span class="setting-label">Ko-fi</span>
                          <span class="setting-hint">One-time donation, no account needed</span>
                          <Button variant="small" onClick={() => openUrl("https://ko-fi.com/tvollstaedt")}>Open</Button>
                        </div>
                        <div class="setting-row">
                          <span class="setting-label">GitHub Sponsors</span>
                          <span class="setting-hint">One-time or monthly via GitHub</span>
                          <Button variant="small" onClick={() => openUrl("https://github.com/sponsors/tvollstaedt")}>Open</Button>
                        </div>
                      </section>

                      <section class="settings-section danger">
                        <h3 class="settings-section-title">Danger Zone</h3>
                        <div class="setting-row">
                          <span class="setting-label">Factory Reset</span>
                          <span class="setting-hint">Clears all data and returns to setup</span>
                          <button class="btn-danger" onClick={() => setShowResetDialog(true)}>Reset…</button>
                        </div>
                        <Show when={resetError()}>
                          <div class="error" style="margin-top:8px">{resetError()}</div>
                        </Show>
                      </section>
                    </div>
                  </Show>

                  <Show when={settingsTab() === "packs"}>
                    <div class="settings-body">
                      <ContentPackSettings />
                    </div>
                  </Show>
                </div>

                <div class="ark-dialog-actions">
                  <Dialog.CloseTrigger class="btn-secondary">Close</Dialog.CloseTrigger>
                </div>
              </Dialog.Content>
            </Dialog.Positioner>
          </Portal>
        </Dialog.Root>
        </Show>

        <Show when={showResetDialog()}>
        <Dialog.Root open={showResetDialog()} onOpenChange={(e) => { setShowResetDialog(e.open); if (!e.open) { setDeleteGameData(false); } }}>
          <Portal>
            <Dialog.Backdrop class="ark-dialog-backdrop" />
            <Dialog.Positioner class="ark-dialog-positioner">
              <Dialog.Content class="ark-dialog-content">
                <Dialog.Title class="ark-dialog-title">Factory Reset</Dialog.Title>
                <Dialog.Description class="ark-dialog-desc">
                  Clears the Exodium database and all settings. Your downloaded game files stay on disk and can be re-imported later.
                </Dialog.Description>
                <label class="reset-option">
                  <input
                    type="checkbox"
                    checked={deleteGameData()}
                    onChange={(e) => setDeleteGameData(e.currentTarget.checked)}
                  />
                  <span>Also delete game folder{gameFolderPath() ? ` (${gameFolderPath()})` : ""}</span>
                </label>
                <Show when={deleteGameData()}>
                  <p class="reset-warning">This will permanently delete all downloaded game files. This cannot be undone.</p>
                </Show>
                <div class="ark-dialog-actions">
                  <Dialog.CloseTrigger class="btn-secondary">Cancel</Dialog.CloseTrigger>
                  <Button variant="danger" onClick={confirmReset}>Reset</Button>
                </div>
              </Dialog.Content>
            </Dialog.Positioner>
          </Portal>
        </Dialog.Root>
        </Show>

        <Library />

        <WelcomeModal
          open={showWelcomeModal()}
          onClose={() => setShowWelcomeModal(false)}
        />

        <SeedingConsentDialog
          open={showSeedingConsent()}
          onDecide={handleSeedingConsent}
        />
      </Show>

      <ToastContainer />

      <Show when={resetting()}>
        <div class="reset-overlay">
          <div class="reset-overlay-card">
            <div class="reset-overlay-spinner" />
            <div class="reset-overlay-title">Resetting Exodium…</div>
            <div class="reset-overlay-hint">Clearing library, downloads and settings. This may take a few seconds.</div>
          </div>
        </div>
      </Show>
    </>
  );
}

export default App;
