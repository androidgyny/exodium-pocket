import { createSignal, createEffect, Show, For, onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import { convertFileSrc } from "@tauri-apps/api/core";
import { AutoProgress } from "./ProgressBar";
import { Lightbox } from "./Lightbox";
import { ManualViewer } from "./ManualViewer";
import { GameSettingsDialog } from "./GameSettingsDialog";
import { PlaylistMenu } from "./PlaylistMenu";
import { Button } from "./Button";
import type { Game, GameMetadata } from "../api/tauri";
import { launchGame } from "../api/tauri";
import { formatBytes, parseLangEntries, langBadgeClass, performUninstall } from "../util";
import { showToast } from "../stores/toasts";
import { bestThumbnailPath } from "../stores/thumbnails";
import { downloads, startGameDownload, getDownloadState, cancelGameDownload, watchExtrasIfPending } from "../stores/downloads";
import { loadGameMetadata } from "../stores/metadata";
import { isOffline } from "../stores/network";
import { loadVariants } from "../stores/variants";
import { videos, requestVideo, releaseVideo, setForegroundVideo, getVideoState, PHASE_QUEUED, PHASE_PROBING } from "../stores/videos";

interface Props {
  game: Game | null;
  onClose: () => void;
  onDownloadStart?: (gameId: number) => void;
}

const isWindows = typeof navigator !== "undefined"
  && /Win/i.test(navigator.platform || navigator.userAgent || "");

/** Language codes seen in the eXoDOS catalogue, spelled out for prose like
 *  "no German description". Unknown codes fall back to the raw code. */
const LANGUAGE_NAMES: Record<string, string> = {
  EN: "English", DE: "German", PL: "Polish", ES: "Spanish",
  FR: "French", IT: "Italian", RU: "Russian", CZ: "Czech",
  NL: "Dutch", PT: "Portuguese", SV: "Swedish", HU: "Hungarian",
};
const languageName = (code: string | null | undefined) =>
  (code && LANGUAGE_NAMES[code]) || code || "";

export function GameDetailPanel(props: Props) {
  const [variants, setVariants] = createSignal<Game[]>([]);
  const [status, setStatus] = createSignal("");
  const [imgError, setImgError] = createSignal(false);
  const [metadata, setMetadata] = createSignal<GameMetadata | null>(null);
  const [metadataLoading, setMetadataLoading] = createSignal(false);
  const [brokenImages, setBrokenImages] = createSignal(new Set<number>());
  const [lightboxOpen, setLightboxOpen] = createSignal(false);
  const [lightboxStart, setLightboxStart] = createSignal(0);
  /** The lightbox lists the preview video as entry 0 when there is one, so an
   *  index into the screenshot array has to be shifted to match. */
  const lightboxIndexOfImage = (imageIndex: number) => (videoSrc() ? imageIndex + 1 : imageIndex);
  const [manualOpen, setManualOpen] = createSignal(false);
  const [settingsOpen, setSettingsOpen] = createSignal(false);
  const [playlistMenu, setPlaylistMenu] = createSignal<{x: number, y: number} | null>(null);
  // Preview video. Fetching starts on open (see the effect below); this is only
  // the playback state - the video takes over the hero while it plays and hands
  // the cover back when it ends.
  const [videoPlaying, setVideoPlaying] = createSignal(false);
  let heroVideoRef: HTMLVideoElement | undefined;
  const openPlaylistMenu = (e: MouseEvent & { currentTarget: HTMLElement }) => {
    const rect = e.currentTarget.getBoundingClientRect();
    setPlaylistMenu({ x: rect.left, y: rect.bottom + 4 });
  };
  const [launchingId, setLaunchingId] = createSignal<number | null>(null);
  const [uninstallingId, setUninstallingId] = createSignal<number | null>(null);
  // The panel always describes exactly ONE row. Multi-language cards are a
  // merged group, so the user picks which language everything below the title
  // refers to - actions, description, manual, screenshots. Before this, the
  // header described the EN row while the Versions list acted on variant rows,
  // and nothing said which one the description or the manual belonged to.
  const [selectedId, setSelectedId] = createSignal<number | null>(null);
  let launchTimer: number | undefined;
  onCleanup(() => { if (launchTimer) { clearTimeout(launchTimer); } });

  const langEntries = () => props.game ? parseLangEntries(props.game) : [];
  const isMultiLang = () => langEntries().length > 1;


  // ── Selected variant ───────────────────────────────────────────────────
  // Single-language games have exactly one row (the game itself), so every
  // rule below collapses to it - the panel has ONE rendering path, which is
  // what previously drifted apart (the Manual button existed only on the
  // single-language branch).
  const rows = (): Game[] => {
    const v = variants();
    if (v.length > 0) { return v; }
    return props.game ? [props.game] : [];
  };
  const selected = (): Game | null => {
    const list = rows();
    return list.find((r) => r.id === selectedId()) ?? list[0] ?? null;
  };
  /** Default pick when a game opens: whatever the user can act on right now -
   *  an installed version first (EN among equals), then one being fetched,
   *  then the English row. */
  const defaultVariant = (list: Game[]): Game | undefined =>
    list.find((v) => v.installed && v.language === "EN")
    ?? list.find((v) => v.installed)
    ?? list.find((v) => v.in_library)
    ?? list.find((v) => v.language === "EN")
    ?? list[0];

  const selectedDl = () => {
    const id = selected()?.id;
    return id != null ? downloads()[id] : undefined;
  };
  const selectedDownloading = () => selectedDl()?.downloading ?? false;
  const selectedInstalled = () =>
    (selected()?.installed ?? false) || (selectedDl()?.installed ?? false);

  /** LP rows carry almost no catalogue text of their own (developer, genre and
   *  friends live on the EN row), so every field falls back to the primary. */
  const field = <K extends keyof Game>(key: K): Game[K] | undefined => {
    const v = selected()?.[key];
    if (v !== null && v !== undefined && v !== "") { return v; }
    return props.game?.[key];
  };

  /** Which row's description we're showing, and whether that's a fallback.
   *  Only 98 of 648 German rows have their own text; Polish and Spanish have
   *  none - so saying "English text, no German available" beats silently
   *  showing English under a DE badge. */
  const descriptionSource = () => {
    const sel = selected();
    const primary = props.game;
    if (sel?.description) {
      return { text: sel.description, notes: sel.notes, fallbackFrom: null as string | null };
    }
    if (primary?.description) {
      const differs = sel?.language && primary.language && sel.language !== primary.language;
      return {
        text: primary.description,
        notes: primary.notes,
        fallbackFrom: differs ? sel!.language : null,
      };
    }
    return null;
  };

  /** The manual to open for the selected variant: its own if the catalogue
   *  lists one, otherwise the English manual (the backend's metadata scan
   *  already falls back to the eXoDOS pack for assets). */
  const manualRow = (): Game | null => {
    const sel = selected();
    if (sel?.manual_path) { return sel; }
    return props.game?.manual_path ? props.game : null;
  };
  const manualIsFallback = () => {
    const sel = selected();
    const row = manualRow();
    return !!row && !!sel?.language && !!row.language && row.language !== sel.language;
  };
  // Download progress used to be echoed here too; the action bar now renders
  // it for the selected variant and the chips show it for the others, so this
  // line is only for launch/uninstall messages.
  const currentStatus = () => status();


  // Reset media state only when the DISPLAYED GAME changes - background
  // library refreshes (install/uninstall completing) replace the game object
  // with a fresh one for the same id, and resetting on those made the cover
  // image and media strip flicker on every state change.
  let lastGameId: number | null | undefined = undefined;
  let lastMetaKey: string | null = null;
  createEffect(() => {
    const g = props.game;
    if (!g) { lastGameId = null; return; }
    if (g.id === lastGameId) { return; }
    lastGameId = g.id;
    // Restart-resume: extras may still be downloading for this installed
    // game with no live tracker (app restarted mid-extras).
    if (g.installed && g.id != null && g.gamedata_torrent_index != null) {
      watchExtrasIfPending(g.id, g.title);
    }
    setImgError(false);
    setStatus("");
    setVariants([]);
    setMetadata(null);
    setBrokenImages(new Set<number>());
    setLightboxOpen(false);
    setManualOpen(false);
    setSelectedId(g.id ?? null);
    setVideoPlaying(false);
    // Force a metadata refetch: the cache key below would otherwise match the
    // previous visit to this same game and leave the panel with the null
    // metadata this reset just wrote (no screenshots, no manual).
    lastMetaKey = null;
    if (g.shortcode && isMultiLang()) {
      const shortcode = g.shortcode;
      loadVariants(shortcode).then((v) => {
        // Guard: game may have changed while the async call was in flight
        if (props.game?.shortcode !== shortcode) { return; }
        setVariants(v);
        setSelectedId(defaultVariant(v)?.id ?? g.id ?? null);
      }).catch(() => {});
    }
  });

  // Metadata (screenshots + manual) belongs to the SELECTED variant, not to
  // the group: an LP metadata pack can ship its own screenshots, and the
  // manual differs per language where one exists. Keyed on id+source+manual so
  // the background variant refresh (same rows, new objects) doesn't refetch.
  createEffect(() => {
    const v = selected();
    const row = manualRow();
    if (!v?.title || !v.torrent_source) { return; }
    const key = `${v.id}:${v.torrent_source}:${row?.manual_path ?? ""}`;
    if (key === lastMetaKey) { return; }
    lastMetaKey = key;
    setMetadata(null);
    setBrokenImages(new Set<number>());
    // The previous variant's cover may have 404'd; the new one gets a fresh
    // chance rather than inheriting the placeholder.
    setImgError(false);
    setMetadataLoading(true);
    loadGameMetadata(v.torrent_source, v.title, v.shortcode ?? null, row?.manual_path ?? null)
      .then((m) => { if (selected()?.id === v.id) { setMetadata(m); } })
      .finally(() => setMetadataLoading(false));
  });

  // Refresh variant list when a download transitions to installed so
  // badges/buttons stay current. Tracks per-id transitions via the store's
  // `installed` flag (its documented contract - status text keeps changing
  // during the extras phase, and re-matching it on every poll tick both
  // missed that phase and refetched variants redundantly).
  const announcedInstalls = new Set<number>();
  createEffect(() => {
    const dl = downloads();
    let freshInstall = false;
    for (const [idStr, d] of Object.entries(dl)) {
      const id = Number(idStr);
      if (d.installed && !announcedInstalls.has(id)) {
        announcedInstalls.add(id);
        freshInstall = true;
      }
    }
    for (const id of [...announcedInstalls]) {
      if (!(id in dl)) { announcedInstalls.delete(id); }
    }
    if (!freshInstall) { return; }
    const g = props.game;
    if (!g?.shortcode || !isMultiLang()) { return; }
    const shortcode = g.shortcode;
    loadVariants(shortcode, true).then((v) => {
      if (props.game?.shortcode === shortcode) { setVariants(v); }
    }).catch(() => {});
  });

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key !== "Escape") { return; }
    // A stacked overlay (lightbox, manual, settings) handles this Escape
    // itself - closing the panel underneath in the same press would yank
    // the user two levels at once.
    if (lightboxOpen() || manualOpen() || settingsOpen()) { return; }
    if (playlistMenu()) { setPlaylistMenu(null); return; }
    props.onClose();
  };

  // Register once for the lifetime of the component - the handler reads props.onClose()
  // reactively through the Proxy so it always calls the current callback.
  onMount(() => {
    // Capture phase: the overlay-open guard must read the signals BEFORE
    // Ark's document-level handler closes the overlay in the same keypress.
    window.addEventListener("keydown", handleKeyDown, true);
    onCleanup(() => window.removeEventListener("keydown", handleKeyDown, true));
  });

  const thumbSrc = () => {
    const g = selected() ?? props.game;
    if (!g) { return null; }
    // LP rows usually inherit the EN thumbnail_key, but fall back explicitly
    // for the ones whose key never got propagated.
    const path = bestThumbnailPath(g.torrent_source, g.thumbnail_key)
      ?? bestThumbnailPath(props.game?.torrent_source, props.game?.thumbnail_key);
    if (!path) { return null; }
    return convertFileSrc(path);
  };

  const handleDownload = (gameId: number, title?: string) => {
    startGameDownload(gameId, title ?? props.game?.title);
  };

  // ── Preview video ──────────────────────────────────────────────────────
  const videoState = () => {
    const id = selected()?.id;
    videos(); // subscribe
    return id != null ? getVideoState(id) : undefined;
  };
  const videoReady = () => videoState()?.phase === "ready" && !!videoState()?.path;
  // Finding out whether a game has a video means reading the archive index over
  // the torrent, which can take tens of seconds. Staying silent through that
  // just looks broken, so each stage says what it is - including the negative
  // answer, which then fades out rather than lingering.
  const videoConfirmed = () => (videoState()?.total_bytes ?? 0) > 0;
  const videoProbing = () => videoState()?.phase === PHASE_PROBING;
  const videoFetching = () => videoState()?.phase === "fetching" && videoConfirmed();
  const videoQueued = () => videoState()?.phase === PHASE_QUEUED;
  const videoFailed = () => videoState()?.phase === "error";

  // "No video" is shown briefly and then disappears - it is closure, not a
  // permanent label on the game.
  const [showNoVideo, setShowNoVideo] = createSignal(false);
  createEffect(() => {
    if (videoState()?.phase !== "none") { return; }
    // Offline reports "none" because nothing can be fetched, which is not the
    // same statement as "this game has no video" - so say nothing at all.
    if (isOffline()) { return; }
    setShowNoVideo(true);
    const timer = window.setTimeout(() => setShowNoVideo(false), 2600);
    onCleanup(() => clearTimeout(timer));
  });
  const videoSrc = () => {
    const p = videoState()?.path;
    return p ? convertFileSrc(p) : null;
  };

  // Start the fetch a beat after the panel settles on a game. The delay is the
  // point: clicking through the grid would otherwise queue a torrent read per
  // card, and each one can pull tens of megabytes.
  createEffect(() => {
    const id = selected()?.id;
    if (id == null) { return; }
    setForegroundVideo(id);
    const timer = window.setTimeout(() => requestVideo(id), 400);
    onCleanup(() => {
      // Only the not-yet-started fetch is dropped. One that is already running
      // keeps going in the background so the video is simply there next time.
      clearTimeout(timer);
      releaseVideo(id);
    });
  });

  // Autoplay as soon as it lands, muted - a preview that demands a click is
  // barely better than no preview, and unmuted autoplay is a good way to make
  // someone close the app.
  createEffect(() => {
    if (!videoReady()) { return; }
    // Deferred: the <video> mounts from the same signal change, so the ref is
    // not assigned yet while this effect body runs.
    queueMicrotask(() => {
      const el = heroVideoRef;
      if (!el) { return; }
      try {
        el.currentTime = 0;
        const started = el.play();
        // Older WebKit returns undefined here instead of a promise. Calling
        // .then on that throws INSIDE the effect, and Solid propagates the
        // exception back to whoever set the signal - which made the video
        // store record a fetch error for a video that had arrived fine.
        if (started && typeof started.then === "function") {
          started.then(() => setVideoPlaying(true)).catch(() => setVideoPlaying(false));
        } else {
          setVideoPlaying(true);
        }
      } catch {
        setVideoPlaying(false);
      }
    });
  });

  const handleManualClick = () => {
    if (metadata()?.manual_path) { setManualOpen(true); }
  };

  const handleLaunch = async (gameId: number) => {
    if (launchingId() != null) { return; }
    setLaunchingId(gameId);
    setStatus("");
    const startedAt = Date.now();
    try {
      await launchGame(gameId);
      // DOSBox spawns immediately but the window can take 1-3s to paint
      // (codesign re-sign on macOS dev, asset preload). Hold the spinner
      // for at least 4s so the user sees it before the button reverts.
      const elapsed = Date.now() - startedAt;
      const remaining = Math.max(0, 4000 - elapsed);
      if (launchTimer) { clearTimeout(launchTimer); }
      launchTimer = window.setTimeout(() => setLaunchingId(null), remaining);
    } catch (e) {
      setLaunchingId(null);
      setStatus("");
      const detail = String(e).replace(/^Error:\s*/, "");
      showToast(`Couldn't launch ${props.game?.title ?? "game"}`, "error", { detail });
    }
  };

  const handleUninstall = async (gameId: number) => {
    if (uninstallingId() != null) { return; }
    // Capture shortcode + title now - props.game may change before the async callback runs.
    const shortcode = props.game?.shortcode;
    const title = variants().find((v) => v.id === gameId)?.title ?? props.game?.title;
    setUninstallingId(gameId);
    // The action row renders the "Uninstalling" state itself now - swallow
    // performUninstall's identical status text (it also feeds GameCard,
    // which has no action row) so the panel doesn't show it twice.
    const statusSink = (msg: string) => {
      if (msg === "Uninstalling...") { return; }
      setStatus(msg);
    };
    try {
      await performUninstall(gameId, statusSink, async () => {
        if (shortcode) {
          const v = await loadVariants(shortcode, true).catch(() => []);
          setVariants(v);
        }
      }, title);
    } finally {
      setUninstallingId(null);
    }
  };

  const ratingStars = (rating: number | null) => {
    if (rating == null) { return null; }
    // eXoDOS ratings are 0–5 scale
    const full = Math.round(rating);
    const empty = 5 - full;
    return "★".repeat(full) + "☆".repeat(empty);
  };

  // Manual: shown iff the catalogue lists one for the selected variant or, as
  // a fallback, for the English row - in which case the label says so, because
  // "Manual" on a DE selection silently opening the English PDF is exactly the
  // ambiguity this panel is meant to remove. Unresolved = its GameData ZIP is
  // still downloading; clicking retries the lookup, so it self-heals.
  /** True once the file behind the catalogue's promise actually exists. */
  const manualAvailable = () => !!metadata()?.manual_path;

  const ManualButton = () => (
    <Button
      variant="action"
      class="btn-manual"
      onClick={handleManualClick}
      // Not yet extracted: the button is simply inert rather than clickable
      // into a "not available" message. The metadata cache is invalidated when
      // a download finishes, so it enables itself once the extras land.
      disabled={metadataLoading() || !manualAvailable()}
      title={
        !metadataLoading() && !manualAvailable()
          ? "Arrives with the game's extras download"
          : manualIsFallback()
            ? `Only the ${languageName(manualRow()?.language)} manual is in the catalogue`
            : undefined
      }
    >
      Manual
      <Show when={!metadataLoading() && manualAvailable() && manualIsFallback()}>
        <span class="btn-suffix">{manualRow()?.language}</span>
      </Show>
    </Button>
  );

  // Shared "Play" button - same disabled+spinner UX whether it's the main
  // single-language action or one row of the multi-language variant list.
  const PlayButton = (p: { id: number; class?: string; disabled?: boolean }) => (
    <Button
      variant="action"
      class={p.class}
      onClick={() => handleLaunch(p.id)}
      disabled={p.disabled}
      loading={launchingId() === p.id}
      loadingLabel="Starting…"
    >
      ▶ Play
    </Button>
  );

  // Genre column is semicolon-joined. The hero chip shows the first piece
  // alone (the "primary" genre); the fields row joins all pieces with " · ".
  const genreList = (): string[] => {
    const raw = props.game?.genre;
    if (!raw) { return []; }
    return raw.split(";").map((p) => p.trim()).filter(Boolean);
  };
  const primaryGenre = (): string | null => genreList()[0] ?? null;
  const allGenres = (): string | null => {
    const list = genreList();
    return list.length > 0 ? list.join(" · ") : null;
  };

  return (
    <Show when={props.game}>
      <Portal>
        <div class="game-detail-backdrop" onClick={props.onClose} />
        <div class="game-detail-panel">
          {/* Close button */}
          <button class="game-detail-close" onClick={props.onClose} title="Close">✕</button>

          {/* Hero: thumbnail + title */}
          <div class="game-detail-hero">
            <div class="game-detail-hero-art">
            <Show when={thumbSrc() && !imgError()}>
              <img
                class={`game-detail-thumb${videoPlaying() ? " is-behind-video" : ""}`}
                src={thumbSrc()!}
                alt=""
                onError={() => setImgError(true)}
                onClick={() => { setLightboxStart(lightboxIndexOfImage(0)); setLightboxOpen(true); }}
              />
            </Show>
            <Show when={!thumbSrc() || imgError()}>
              <div class="game-detail-thumb-placeholder" />
            </Show>

            {/* The preview takes the cover's place while it runs, then fades
                back out - it stays reachable in the lightbox afterwards. */}
            <Show when={videoSrc()}>
              <video
                ref={heroVideoRef}
                class={`game-detail-hero-video${videoPlaying() ? " is-visible" : ""}`}
                src={videoSrc()!}
                muted
                playsinline
                preload="auto"
                onEnded={() => setVideoPlaying(false)}
                onPause={() => setVideoPlaying(false)}
                onPlay={() => setVideoPlaying(true)}
                onClick={() => { setLightboxStart(0); setLightboxOpen(true); }}
              />
            </Show>

            {/* Status while the bytes are still coming over the torrent. */}
            <Show when={videoProbing() || videoFetching() || videoQueued()}>
              <div class="game-detail-video-status">
                <span class="btn-spinner" />
                <Show when={videoQueued()} fallback={
                  <Show when={videoConfirmed()} fallback={<>Looking for a video…</>}>
                    Loading video {Math.round((videoState()?.progress ?? 0) * 100)}%
                  </Show>
                }>
                  Video queued…
                </Show>
              </div>
            </Show>

            {/* The negative answer, shown long enough to read and then gone. */}
            <Show when={showNoVideo()}>
              <div class="game-detail-video-status is-fading-out">No video for this game</div>
            </Show>

            {/* A failed fetch must not look like "this game has no video" -
                a stalled torrent read is worth retrying, a missing video is not. */}
            <Show when={videoFailed()}>
              <button
                class="game-detail-video-status game-detail-video-retry"
                title={videoState()?.error ?? undefined}
                onClick={() => { const id = selected()?.id; if (id != null) { requestVideo(id); } }}
              >↻ Video retry</button>
            </Show>

            {/* Replay control once it has run its course. */}
            <Show when={videoReady() && !videoPlaying()}>
              <button
                class="game-detail-video-replay"
                title="Play the preview again"
                onClick={() => heroVideoRef?.play()}
              >▶</button>
            </Show>
            </div>

            <div class="game-detail-hero-info">
              <div class="game-detail-title">{selected()?.title ?? props.game!.title}</div>
              <div class="game-detail-chips">
                {props.game!.year && <span class="badge">{props.game!.year}</span>}
                {primaryGenre() && <span class="badge badge-genre">{primaryGenre()}</span>}
              </div>
            </div>
          </div>

          <div class="game-detail-body">
            {/* Status message */}
            <Show when={currentStatus()}>
              <div class="game-detail-status">{currentStatus()}</div>
            </Show>

            {/* Emulator note: ECE-tuned games run under DOSBox Staging on
                non-Windows platforms (ECE ships Windows binaries only). */}
            <Show when={!isWindows && props.game?.dosbox_variant?.startsWith("ece")}>
              <div class="game-detail-note">
                This game is tuned for DOSBox ECE, which only exists on
                Windows. Exodium runs it with DOSBox Staging - the experience
                may vary slightly.
              </div>
            </Show>

            {/* Language switcher: picking a chip re-points the whole panel -
                actions, description, manual and screenshots all follow it. */}
            <Show when={isMultiLang()}>
              <div class="variant-switcher" role="group" aria-label="Language versions">
                <Show when={rows().length < 2}>
                  <div class="game-detail-loading">Loading versions…</div>
                </Show>
                <For each={rows()}>
                  {(variant) => {
                    const vId = () => variant.id;
                    const vDl = () => vId() != null ? getDownloadState(vId()!) : undefined;
                    const state = () => variant.installed ? 2 : variant.in_library ? 1 : 0;
                    return (
                      <button
                        class={`variant-chip${selected()?.id === vId() ? " is-selected" : ""}`}
                        onClick={() => { if (vId() != null) { setSelectedId(vId()!); } }}
                        title={languageName(variant.language)}
                      >
                        <span class={`badge badge-lang ${langBadgeClass(state())}`}>
                          {variant.language}
                        </span>
                        <span class="variant-chip-state">
                          <Show when={vDl()?.downloading} fallback={
                            <Show when={variant.installed} fallback={
                              <Show when={!isOffline() && variant.game_torrent_index != null} fallback={<>Not installed</>}>
                                ↓ {formatBytes(variant.download_size ?? 0)}
                              </Show>
                            }>
                              ✓ Installed
                            </Show>
                          }>
                            {Math.round((vDl()?.progress ?? 0) * 100)}%
                          </Show>
                        </span>
                      </button>
                    );
                  }}
                </For>
              </div>
            </Show>

            {/* One action bar for every game. Everything here targets the
                SELECTED row, so a merged card can play the German version and
                open the German manual without a second code path. */}
            <Show when={selected()}>
              {(sel) => (
                <Show when={uninstallingId() !== sel().id} fallback={
                  <div class="game-detail-actions fade-swap">
                    <div class="game-detail-btn btn-uninstalling">
                      <span class="btn-spinner" /> Uninstalling…
                    </div>
                  </div>
                }>
                  <div class="game-detail-actions fade-swap">
                    <Show when={selectedInstalled() && sel().id != null}>
                      <PlayButton id={sel().id!} class="btn-play" />
                    </Show>
                    <Show when={selectedInstalled() && manualRow()}>
                      <ManualButton />
                    </Show>
                    <Show when={selectedInstalled()}>
                      <Button variant="action" class="btn-settings" title="Game settings" onClick={() => setSettingsOpen(true)}>
                        ⚙
                      </Button>
                    </Show>
                    <Show when={!selectedInstalled() && selectedDownloading()}>
                      <div class="game-detail-btn btn-downloading">
                        <AutoProgress
                          value={selectedDl()?.progress ?? 0}
                          class="mini"
                          indeterminate={selectedDl()?.status?.startsWith("Waiting") || selectedDl()?.status?.startsWith("Extracting") || undefined}
                        />
                        <span>{selectedDl()?.status}</span>
                      </div>
                      <Button variant="action" class="btn-cancel" onClick={() => cancelGameDownload(sel().id!)}>
                        ✕ Cancel
                      </Button>
                    </Show>
                    <Show when={!selectedInstalled() && !selectedDownloading() && sel().game_torrent_index != null && !isOffline()}>
                      <Button
                        variant="action"
                        class="btn-download"
                        onClick={() => handleDownload(sel().id!, isMultiLang() ? `${sel().title} [${sel().language}]` : sel().title)}
                      >
                        {sel().in_library
                          ? "↓ Re-download"
                          : `↓ Download ${sel().download_size ? formatBytes(sel().download_size!) : ""}`}
                      </Button>
                    </Show>
                    <Show when={!selectedInstalled() && !selectedDownloading() && isOffline()}>
                      <div class="game-detail-btn btn-offline" title="Enable downloads in Settings → Network">
                        Not installed - offline mode
                      </div>
                    </Show>
                    <Show when={!selectedDownloading() && (selectedInstalled() || sel().in_library) && sel().id != null}>
                      <Button
                        variant="action"
                        class="btn-uninstall"
                        disabled={launchingId() != null}
                        onClick={() => handleUninstall(sel().id!)}
                      >
                        Uninstall
                      </Button>
                    </Show>
                    <Show when={props.game!.id != null}>
                      <Button
                        variant="action"
                        class="btn-playlist"
                        title="Add to playlist"
                        onClick={openPlaylistMenu}
                      >
                        ＋ Playlist
                      </Button>
                    </Show>
                  </div>
                </Show>
              )}
            </Show>

            {/* Two columns side by side on a wide panel, stacked when it's
                narrow (flex-wrap, no breakpoint) - the pair is what keeps the
                screenshots on screen without scrolling. */}
            <div class="game-detail-scroll">
              <div class="game-detail-columns">
                {/* Catalogue fields. Values come from the selected row where it
                    has them and from the English row otherwise - LP rows carry
                    little more than a title. */}
                <div class="game-detail-fields">
                  <Show when={field("developer")}>
                    <div class="game-detail-field">
                      <span class="game-detail-field-label">Developer</span>
                      <span>{field("developer")}</span>
                    </div>
                  </Show>
                  <Show when={field("publisher")}>
                    <div class="game-detail-field">
                      <span class="game-detail-field-label">Publisher</span>
                      <span>{field("publisher")}</span>
                    </div>
                  </Show>
                  <Show when={field("series")}>
                    <div class="game-detail-field">
                      <span class="game-detail-field-label">Series</span>
                      <span>{field("series")}</span>
                    </div>
                  </Show>
                  <Show when={allGenres()}>
                    <div class="game-detail-field">
                      <span class="game-detail-field-label">Genre</span>
                      <span>{allGenres()}</span>
                    </div>
                  </Show>
                  <Show when={field("play_mode")}>
                    <div class="game-detail-field">
                      <span class="game-detail-field-label">Mode</span>
                      <span>{field("play_mode")}</span>
                    </div>
                  </Show>
                  <Show when={field("region")}>
                    <div class="game-detail-field">
                      <span class="game-detail-field-label">Region</span>
                      <span>{field("region")}</span>
                    </div>
                  </Show>
                  <Show when={field("max_players") != null}>
                    <div class="game-detail-field">
                      <span class="game-detail-field-label">Players</span>
                      <span>{field("max_players")}</span>
                    </div>
                  </Show>
                  <Show when={field("rating") != null}>
                    <div class="game-detail-field">
                      <span class="game-detail-field-label">Rating</span>
                      <span class="game-detail-stars">{ratingStars(field("rating") as number)}</span>
                    </div>
                  </Show>
                </div>

                <div class="game-detail-text">
                  <Show when={descriptionSource()}>
                    {(src) => (
                      <>
                        <Show when={src().fallbackFrom}>
                          <div class="game-detail-fallback-note">
                            English description - the catalogue has no{" "}
                            {languageName(src().fallbackFrom)} text for this game.
                          </div>
                        </Show>
                        <div class="game-detail-description">{src().text}</div>
                        <Show when={src().notes}>
                          <div class="game-detail-notes">{src().notes}</div>
                        </Show>
                      </>
                    )}
                  </Show>
                  <Show when={metadataLoading()}>
                    <div class="game-detail-loading">Loading media…</div>
                  </Show>
                </div>
              </div>
            </div>

            {/* Media: screenshots/art - only renders if the metadata content
                pack has assets for this game. Pinned to the bottom of the
                panel so it's always visible. */}
            <Show when={!metadataLoading() && metadata() && metadata()!.images.length > 0}>
              <div class="game-detail-media">
                {(() => {
                  const visible = () => (metadata()?.images ?? []).filter((_, i) => !brokenImages().has(i));
                  return (
                    <Show when={metadata()!.images.length > 0 && visible().length > 0}>
                      <div class="game-detail-section-label">
                        Screenshots &amp; Art
                        <span class="section-count">{visible().length}</span>
                      </div>
                      <div class="game-detail-gallery-strip">
                        <For each={metadata()!.images}>
                          {(path, i) => (
                            <img
                              // Strip shows the cached 160px copy; the lightbox
                              // opens the full-resolution file behind it.
                              src={convertFileSrc(metadata()!.thumbnails[i()] ?? path)}
                              class="gallery-thumb"
                              loading="lazy"
                              alt=""
                              onClick={() => {
                                const vi = visible().indexOf(path);
                                setLightboxStart(lightboxIndexOfImage(vi >= 0 ? vi : 0));
                                setLightboxOpen(true);
                              }}
                              onError={() => setBrokenImages((prev) => new Set(prev).add(i()))}
                              style={{ display: brokenImages().has(i()) ? "none" : undefined }}
                            />
                          )}
                        </For>
                      </div>
                    </Show>
                  );
                })()}
              </div>
            </Show>
          </div>
        </div>

        <Lightbox
          video={videoSrc()}
          images={(() => {
            const filtered = (metadata()?.images ?? []).filter((_, i) => !brokenImages().has(i));
            if (filtered.length > 0) { return filtered; }
            // Fallback: use the hero thumbnail so clicking box art works even
            // without the metadata pack installed.
            const hero = bestThumbnailPath(props.game?.torrent_source, props.game?.thumbnail_key);
            return hero ? [hero] : [];
          })()}
          startIndex={lightboxStart()}
          open={lightboxOpen()}
          onClose={() => setLightboxOpen(false)}
        />
        <ManualViewer
          path={metadata()?.manual_path ?? null}
          kind={metadata()?.manual_kind ?? null}
          open={manualOpen()}
          onClose={() => setManualOpen(false)}
        />
        <GameSettingsDialog
          gameId={selected()?.id ?? null}
          gameTitle={selected()?.title ?? props.game?.title ?? ""}
          open={settingsOpen()}
          onClose={() => setSettingsOpen(false)}
        />
        <Show when={playlistMenu() && props.game?.id != null}>
          <PlaylistMenu
            x={playlistMenu()!.x}
            y={playlistMenu()!.y}
            gameId={props.game!.id!}
            onClose={() => setPlaylistMenu(null)}
          />
        </Show>
      </Portal>
    </Show>
  );
}
