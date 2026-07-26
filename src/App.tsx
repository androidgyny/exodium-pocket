import { createSignal, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { open } from "@tauri-apps/plugin-dialog";
import { Dialog } from "@ark-ui/solid/dialog";
import { Tooltip } from "@ark-ui/solid/tooltip";
import { Library } from "./pages/Library";
import { Setup } from "./pages/Setup";
import { SearchBar } from "./components/SearchBar";
import { WelcomeModal } from "./components/WelcomeModal";
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
import { loadThumbnailDir } from "./stores/thumbnails";
import { refreshInstalledPacks } from "./stores/contentPacks";
import { showToast } from "./stores/toasts";
import "./styles/main.css";

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

  onMount(async () => {
    try {
      const status = await getSetupStatus();
      if (status.ready) {
        setPhase("ready");
        try {
          await initDownloadManager();
        } catch (e) {
          console.error("Failed to init download manager:", e);
        }
        const dir = await getConfig("data_dir");
        if (dir) { setDataDir(dir); }
        loadThumbnailDir();
        refreshInstalledPacks();
        notifyCatalogUpdates();
        checkForAppUpdate();
      } else {
        setPhase("setup");
      }
    } catch {
      setPhase("setup");
    }
  });

  const handleSetupComplete = async () => {
    setPhase("ready");
    const dir = await getConfig("data_dir");
    if (dir) { setDataDir(dir); }
    loadThumbnailDir();
    refreshInstalledPacks();
    fetchGames();
    checkForAppUpdate();

    // Show the welcome modal if the user hasn't seen it yet.
    const welcomeSeen = await getConfig("welcome_seen");
    if (welcomeSeen !== "1") {
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
  const [crtAuto, setCrtAuto] = createSignal(false);
  const [defaultFullscreen, setDefaultFullscreen] = createSignal(false);

  const [seeding, setSeeding] = createSignal(true);

  const loadGameDefaults = async () => {
    try {
      const [shader, fs, seed] = await Promise.all([
        getConfig("global_glshader"),
        getConfig("default_fullscreen"),
        getConfig("seeding_enabled"),
      ]);
      setCrtAuto(shader == null || shader === "crt-auto");
      setDefaultFullscreen(fs === "fullscreen");
      setSeeding(seed !== "0");
    } catch (e) {
      console.warn("[settings] failed to load game defaults:", e);
    }
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
            <DownloadIndicator />
            <Tooltip.Root openDelay={400}>
              <Tooltip.Trigger asChild={(props) =>
                <button {...props()} class="icon-btn" onClick={() => setShowSettings(true)}>
                  &#9881;
                </button>
              } />
              <Portal><Tooltip.Positioner><Tooltip.Content class="ark-tooltip">Settings</Tooltip.Content></Tooltip.Positioner></Portal>
            </Tooltip.Root>
          </div>
        </div>

        <Show when={showSettings()}>
        <Dialog.Root open={showSettings()} onOpenChange={(e) => {
          setShowSettings(e.open);
          if (e.open) { loadGameDefaults(); setLogOpenError(""); setSettingsTab("general"); }
        }}>
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
                          <button class="btn-small" onClick={handleChangeDataDir}>Change</button>
                        </div>
                        <div class="setting-row">
                          <span class="setting-label">Installed games</span>
                          <span class="setting-hint">Re-scan disk to detect already-downloaded games</span>
                          <button class="btn-small" onClick={handleRescan} disabled={scanning()}>
                            {scanning() ? "Scanning…" : "Scan"}
                          </button>
                        </div>
                        <Show when={scanResult()}>
                          <div class="setting-hint" style="margin-top:4px">{scanResult()}</div>
                        </Show>
                      </section>

                      <section class="settings-section">
                        <h3 class="settings-section-title">Game Defaults</h3>
                        <p class="settings-section-hint">Applied as a last-wins DOSBox config on every launch. Overrides per-game settings without modifying eXoDOS's bundled configs.</p>
                        <label class="setting-toggle">
                          <input
                            type="checkbox"
                            checked={crtAuto()}
                            onChange={(e) => handleToggleCrtAuto(e.currentTarget.checked)}
                          />
                          <span class="setting-toggle-info">
                            <span class="setting-toggle-label">Auto CRT shaders</span>
                            <span class="setting-toggle-hint">DOSBox Staging picks a CRT shader matched to each game's video mode and your display resolution.</span>
                          </span>
                        </label>
                        <label class="setting-toggle">
                          <input
                            type="checkbox"
                            checked={defaultFullscreen()}
                            onChange={(e) => handleToggleFullscreen(e.currentTarget.checked)}
                          />
                          <span class="setting-toggle-info">
                            <span class="setting-toggle-label">Launch in fullscreen</span>
                            <span class="setting-toggle-hint">Start every game fullscreen instead of windowed. Alt+Enter still toggles at runtime.</span>
                          </span>
                        </label>
                      </section>

                      <section class="settings-section">
                        <h3 class="settings-section-title">Network</h3>
                        <p class="settings-section-hint">Games are downloaded from the eXoDOS BitTorrent swarm. While Exodium runs, it also uploads pieces you already have to other players.</p>
                        <label class="setting-toggle">
                          <input
                            type="checkbox"
                            checked={seeding()}
                            onChange={(e) => handleToggleSeeding(e.currentTarget.checked)}
                          />
                          <span class="setting-toggle-info">
                            <span class="setting-toggle-label">Share with other players (seeding)</span>
                            <span class="setting-toggle-hint">Keeps the collection alive for everyone. Turning this off caps upload at 1 KB/s.</span>
                          </span>
                        </label>
                      </section>

                      <section class="settings-section">
                        <h3 class="settings-section-title">Diagnostics</h3>
                        <p class="settings-section-hint">If a download stalls or the app misbehaves, share <code>exodium.log</code> from the folder.</p>
                        <div class="setting-row">
                          <span class="setting-label">Log folder</span>
                          <span class="setting-hint">Open in your file explorer</span>
                          <button class="btn-small" onClick={handleOpenLogFolder}>Open</button>
                        </div>
                        <Show when={logOpenError()}>
                          <div class="error" style="margin-top:6px">{logOpenError()}</div>
                        </Show>
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
                  <button class="btn-danger" onClick={confirmReset}>Reset</button>
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
