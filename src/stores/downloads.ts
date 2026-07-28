import { createSignal } from "solid-js";
import { cancelDownload, downloadGame, getDownloadProgress } from "../api/tauri";
import { refreshLoadedGames, notifyGameLibraryChanged } from "./games";
import { showToast } from "./toasts";

interface DownloadState {
  status: string;
  progress: number;
  downloading: boolean;
  /** True from the moment the game itself is playable (extras may still be
   *  downloading) - components must use this, not string-match the status. */
  installed?: boolean;
  title?: string;
}

const [downloads, setDownloads] = createSignal<Record<number, DownloadState>>({});

// Count of consecutive poll ticks where getDownloadProgress returned null
// despite the download being marked in-flight. If this stays high for >5s
// we surface a user-visible error instead of pretending we're still starting.
// Observed on Windows: if session.add_torrent() fails (MAX_PATH, port bind,
// etc.) the handle stays None forever and file_progress returns None silently.
const nullPollCount: Record<number, number> = {};
const NULL_POLL_THRESHOLD = 5; // ~5 seconds at 1s polling interval

// Track active polling intervals so they can be cancelled.
const intervals: Record<number, ReturnType<typeof setInterval>> = {};
// Track when a game first reached 100% without finishing (stuck detection).
const stuckSince: Record<number, number> = {};
// True while the download_game backend command is still in flight. Progress
// legitimately polls null during that window (torrent handle not attached
// yet, validation pass, first-ever torrent add), so the didn't-start verdict
// must not fire until the command has actually resolved.
const commandPending: Record<number, boolean> = {};
// Monotonic attempt counter per game: a cancelled attempt's still-resolving
// download_game promise (or orphaned interval tick) must not clobber the
// state of a NEWER attempt for the same game.
const attempts: Record<number, number> = {};
// Set once the game itself is installed while extras are still downloading -
// the library refresh must fire at that moment (game is playable), not only
// when the extras finish minutes later.
const announcedInstalled: Record<number, boolean> = {};
// Stall detection: timestamp + value of the last observed progress increase.
const lastProgressAt: Record<number, number> = {};
const lastProgressVal: Record<number, number> = {};
// Seconds without progress before the status turns into peer-wait feedback,
// and before it becomes an actionable stall warning.
const STALL_HINT_SECS = 15;
const STALL_WARN_SECS = 90;
// Highest progress seen per game - prevents bar from jumping backwards due to
// librqbit stats blips or component remounts resetting the CSS transition.
const maxProgress: Record<number, number> = {};
// Titles tracked separately so state updates inside the poll loop don't have
// to re-pass the title every time.
const titles: Record<number, string> = {};

export { downloads };

export function getDownloadState(gameId: number): DownloadState | undefined {
  return downloads()[gameId];
}

export function startGameDownload(gameId: number, title?: string) {
  const attempt = (attempts[gameId] ?? 0) + 1;
  attempts[gameId] = attempt;
  delete announcedInstalled[gameId];
  maxProgress[gameId] = 0;
  commandPending[gameId] = true;
  lastProgressVal[gameId] = -1;
  lastProgressAt[gameId] = Date.now();
  if (title) { titles[gameId] = title; }
  setDownloads((prev) => ({
    ...prev,
    [gameId]: { status: "Starting download...", progress: 0, downloading: true, title },
  }));

  const interval = setInterval(async () => {
    if (attempts[gameId] !== attempt) {
      clearInterval(interval);
      return;
    }
    try {
      const p = await getDownloadProgress(gameId);
      if (!p) {
        // Backend returned null - torrent handle not attached yet. While the
        // download_game command is still running that's expected (first-ever
        // torrent add + validation can take a while) - keep waiting. Only
        // once the command has resolved do consecutive misses indicate the
        // silent-stuck bug (observed on Windows: session.add_torrent()
        // failure leaves the handle None forever).
        if (commandPending[gameId]) {
          nullPollCount[gameId] = 0;
          // The backend can legitimately spend minutes here on the FIRST
          // download of a collection (placeholder creation + hash check of
          // 14k files, slow on Windows). Say so instead of sitting mute on
          // "Starting download..." - testers read that as a hang.
          const waited = (Date.now() - (lastProgressAt[gameId] ?? Date.now())) / 1000;
          if (waited > 8) {
            setDownloads((prev) => ({
              ...prev,
              [gameId]: {
                status: "Preparing the collection (one-time setup, can take a few minutes)…",
                progress: 0,
                downloading: true,
                title: titles[gameId],
              },
            }));
          }
          return;
        }
        nullPollCount[gameId] = (nullPollCount[gameId] ?? 0) + 1;
        if (nullPollCount[gameId] >= NULL_POLL_THRESHOLD) {
          clearInterval(interval);
          delete intervals[gameId];
          delete stuckSince[gameId];
          delete maxProgress[gameId];
          delete nullPollCount[gameId];
          delete commandPending[gameId];
          delete lastProgressAt[gameId];
          delete lastProgressVal[gameId];
          setDownloads((prev) => ({
            ...prev,
            [gameId]: {
              status: "Download didn't start - open Settings → Diagnostics to view exodium.log.",
              progress: 0,
              downloading: false,
              title: titles[gameId],
            },
          }));
          delete titles[gameId];
        }
        return;
      }
      delete nullPollCount[gameId];
      // Only allow progress to increase - prevents backwards jumps.
      const safeProgress = Math.max(maxProgress[gameId] ?? 0, p.progress);
      maxProgress[gameId] = safeProgress;

      if (p.error) {
        clearInterval(interval);
        delete intervals[gameId];
        delete stuckSince[gameId];
        delete maxProgress[gameId];
        delete lastProgressAt[gameId];
        delete lastProgressVal[gameId];
        delete announcedInstalled[gameId];
        delete commandPending[gameId];
        setDownloads((prev) => ({
          ...prev,
          [gameId]: { status: p.error!, progress: 0, downloading: false, title: titles[gameId] },
        }));
        showToast(
          titles[gameId] ? `Download failed: ${titles[gameId]}` : "Download failed",
          "error",
          { detail: p.error! },
        );
        delete titles[gameId];
      } else if (p.installed) {
        // The game is playable now, but its extras (GameData: manuals,
        // videos, music) may still be downloading - keep polling and show
        // that second phase instead of letting it finish invisibly.
        const extrasPending = p.extras_done === false;
        if (extrasPending) {
          const pct = ((p.extras_progress ?? 0) * 100).toFixed(0);
          if (!announcedInstalled[gameId]) {
            announcedInstalled[gameId] = true;
            refreshLoadedGames();
            notifyGameLibraryChanged(gameId);
          }
          setDownloads((prev) => ({
            ...prev,
            [gameId]: {
              status: `Installed - downloading extras… ${pct}%`,
              progress: 1,
              downloading: false,
              installed: true,
              title: titles[gameId],
            },
          }));
          return;
        }
        clearInterval(interval);
        delete intervals[gameId];
        delete stuckSince[gameId];
        delete maxProgress[gameId];
        delete lastProgressAt[gameId];
        delete lastProgressVal[gameId];
        delete announcedInstalled[gameId];
        delete commandPending[gameId];
        setDownloads((prev) => ({
          ...prev,
          [gameId]: { status: "Installed!", progress: 1, downloading: false, installed: true, title: titles[gameId] },
        }));
        delete titles[gameId];
        refreshLoadedGames();
        // Fires metadata-cache invalidation: when extras finished AFTER the
        // game, this is what makes the manual button resolve on its own.
        notifyGameLibraryChanged(gameId);
        // Delay cleanup so isInstalled() stays true until fetchGames() propagates the
        // updated installed flag from the DB into the games store.
        setTimeout(() => {
          setDownloads((prev) => {
            const next = { ...prev };
            delete next[gameId];
            return next;
          });
        }, 5000);
      } else if (p.finished) {
        delete stuckSince[gameId];
        setDownloads((prev) => ({
          ...prev,
          [gameId]: { status: "Extracting...", progress: safeProgress, downloading: true, title: titles[gameId] },
        }));
      } else if (safeProgress >= 0.999) {
        // 100% but ZIP not yet assembled - detect if stuck.
        if (!stuckSince[gameId]) { stuckSince[gameId] = Date.now(); }
        const elapsed = (Date.now() - stuckSince[gameId]) / 1000;
        const status = elapsed > 30
          ? "Waiting for last pieces… try cancelling and re-downloading if this persists"
          : "100%";
        setDownloads((prev) => ({
          ...prev,
          [gameId]: { status, progress: safeProgress, downloading: true, title: titles[gameId] },
        }));
      } else if (p.torrent_state === "initializing") {
        // librqbit is hash-checking the entire torrent's existing on-disk
        // content before any peer pieces are requested. On Windows with
        // thousands of placeholder files this can take 5–10 minutes the
        // first time. Per-file progress stays at 0 the whole time, so we
        // surface the torrent-level validation progress to the user.
        delete stuckSince[gameId];
        const tp = typeof p.torrent_progress === "number" ? p.torrent_progress : 0;
        const pct = (tp * 100).toFixed(0);
        setDownloads((prev) => ({
          ...prev,
          [gameId]: {
            status: `Validating torrent ${pct}% (first run can take several minutes)`,
            progress: tp,
            downloading: true,
            title: titles[gameId],
          },
        }));
      } else {
        delete stuckSince[gameId];
        // Stall feedback: a torrent with no peers (or a dropped connection)
        // otherwise sits at "0%" forever with no signal that anything is
        // wrong. Track the last progress increase and escalate the status.
        const now = Date.now();
        if (safeProgress > (lastProgressVal[gameId] ?? -1)) {
          lastProgressVal[gameId] = safeProgress;
          lastProgressAt[gameId] = now;
        }
        const stalledSecs = (now - (lastProgressAt[gameId] ?? now)) / 1000;
        const pct = `${(safeProgress * 100).toFixed(0)}%`;
        let status = pct;
        if (stalledSecs >= STALL_WARN_SECS) {
          status = `Stalled at ${pct} - no data received. Check your connection, or cancel and retry.`;
        } else if (stalledSecs >= STALL_HINT_SECS) {
          status = safeProgress === 0 ? "Looking for peers…" : `${pct} - waiting for peers…`;
        }
        setDownloads((prev) => ({
          ...prev,
          [gameId]: {
            status,
            progress: safeProgress,
            downloading: true,
            title: titles[gameId],
          },
        }));
      }
    } catch (e) {
      console.error(`[downloads] poll error for game ${gameId}:`, e);
    }
  }, 1000);

  intervals[gameId] = interval;

  // Fire download command
  downloadGame(gameId).then(() => {
    if (attempts[gameId] !== attempt) { return; }
    commandPending[gameId] = false;
  }).catch((e) => {
    if (attempts[gameId] !== attempt) { return; }
    clearInterval(interval);
    delete intervals[gameId];
    delete stuckSince[gameId];
    delete maxProgress[gameId];
    delete nullPollCount[gameId];
    delete commandPending[gameId];
    delete lastProgressAt[gameId];
    delete lastProgressVal[gameId];
    delete announcedInstalled[gameId];
    setDownloads((prev) => ({
      ...prev,
      [gameId]: { status: `Error: ${e}`, progress: 0, downloading: false, title: titles[gameId] },
    }));
    showToast(
      titles[gameId] ? `Couldn't start download: ${titles[gameId]}` : "Couldn't start download",
      "error",
      { detail: String(e) },
    );
    delete titles[gameId];
  });
}

/** Stop any polling/UI state for a game regardless of phase - used by
 *  uninstall, which may run during the extras phase where downloading is
 *  false but a poll interval is still alive (it would otherwise resurrect a
 *  phantom stuck/failed card for the freshly uninstalled game). */
export function stopGameDownloadTracking(gameId: number) {
  attempts[gameId] = (attempts[gameId] ?? 0) + 1;
  clearInterval(intervals[gameId]);
  delete intervals[gameId];
  delete stuckSince[gameId];
  delete maxProgress[gameId];
  delete nullPollCount[gameId];
  delete commandPending[gameId];
  delete lastProgressAt[gameId];
  delete lastProgressVal[gameId];
  delete announcedInstalled[gameId];
  delete titles[gameId];
  setDownloads((prev) => {
    if (!prev[gameId]) { return prev; }
    const next = { ...prev };
    delete next[gameId];
    return next;
  });
}

/** Restart-resume for the extras phase: an installed game whose GameData
 *  was still downloading when the app quit resumes invisibly (librqbit
 *  session restore) - poll it so the phase stays visible and the completion
 *  refresh fires. No-op when a tracker already exists or extras are done. */
export async function watchExtrasIfPending(gameId: number, title?: string) {
  if (intervals[gameId] || getDownloadState(gameId)) { return; }
  try {
    const p = await getDownloadProgress(gameId);
    if (!p || !p.installed || p.extras_done !== false) { return; }
  } catch { return; }
  startGameDownload(gameId, title);
}

export async function cancelGameDownload(gameId: number) {
  attempts[gameId] = (attempts[gameId] ?? 0) + 1; // invalidate in-flight handlers
  delete announcedInstalled[gameId];
  clearInterval(intervals[gameId]);
  delete intervals[gameId];
  delete stuckSince[gameId];
  delete maxProgress[gameId];
  delete nullPollCount[gameId];
  delete commandPending[gameId];
  delete lastProgressAt[gameId];
  delete lastProgressVal[gameId];
  delete titles[gameId];
  setDownloads((prev) => {
    const next = { ...prev };
    delete next[gameId];
    return next;
  });
  try {
    await cancelDownload(gameId);
    refreshLoadedGames();
  } catch {}
}
