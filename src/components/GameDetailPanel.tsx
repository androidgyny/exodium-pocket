import { createSignal, createEffect, Show, For, onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
import { convertFileSrc } from "@tauri-apps/api/core";
import { AutoProgress } from "./ProgressBar";
import { Lightbox } from "./Lightbox";
import { ManualViewer } from "./ManualViewer";
import { GameSettingsDialog } from "./GameSettingsDialog";
import type { Game, GameMetadata } from "../api/tauri";
import { launchGame, getGameVariants } from "../api/tauri";
import { formatBytes, parseLangEntries, langBadgeClass, performUninstall } from "../util";
import { showToast } from "../stores/toasts";
import { bestThumbnailPath } from "../stores/thumbnails";
import { downloads, startGameDownload, getDownloadState, cancelGameDownload } from "../stores/downloads";
import { loadGameMetadata } from "../stores/metadata";

interface Props {
  game: Game | null;
  onClose: () => void;
  onDownloadStart?: (gameId: number) => void;
}

export function GameDetailPanel(props: Props) {
  const [variants, setVariants] = createSignal<Game[]>([]);
  const [status, setStatus] = createSignal("");
  const [imgError, setImgError] = createSignal(false);
  const [metadata, setMetadata] = createSignal<GameMetadata | null>(null);
  const [metadataLoading, setMetadataLoading] = createSignal(false);
  const [brokenImages, setBrokenImages] = createSignal(new Set<number>());
  const [lightboxOpen, setLightboxOpen] = createSignal(false);
  const [lightboxStart, setLightboxStart] = createSignal(0);
  const [manualOpen, setManualOpen] = createSignal(false);
  const [settingsOpen, setSettingsOpen] = createSignal(false);
  const [launchingId, setLaunchingId] = createSignal<number | null>(null);
  let launchTimer: number | undefined;
  onCleanup(() => { if (launchTimer) { clearTimeout(launchTimer); } });

  createEffect(() => {
    const g = props.game;
    if (!g) { return; }
    setImgError(false);
    setStatus("");
    setVariants([]);
    setMetadata(null);
    setBrokenImages(new Set<number>());
    setLightboxOpen(false);
    setManualOpen(false);
    if (g.shortcode && isMultiLang()) {
      const shortcode = g.shortcode;
      getGameVariants(shortcode).then((v) => {
        // Guard: game may have changed while the async call was in flight
        if (props.game?.shortcode === shortcode) { setVariants(v); }
      }).catch(() => {});
    }
    // Fetch metadata for the detail panel's Media section. Returns null
    // silently when no pack is installed or the title has no entry in the
    // extracted metadata zip.
    if (g.title && g.torrent_source) {
      const gameId = g.id;
      setMetadataLoading(true);
      loadGameMetadata(g.torrent_source, g.title, g.shortcode ?? null, g.manual_path ?? null)
        .then((m) => { if (props.game?.id === gameId) { setMetadata(m); } })
        .finally(() => setMetadataLoading(false));
    }
  });

  // Refresh variant list when any download completes so badges/buttons stay current
  createEffect(() => {
    const g = props.game;
    if (!g?.shortcode || !isMultiLang()) { return; }
    const dl = downloads();
    if (Object.values(dl).some((d) => d.status === "Installed!" && !d.downloading)) {
      const shortcode = g.shortcode;
      getGameVariants(shortcode).then((v) => {
        if (props.game?.shortcode === shortcode) { setVariants(v); }
      }).catch(() => {});
    }
  });

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") { props.onClose(); }
  };

  // Register once for the lifetime of the component - the handler reads props.onClose()
  // reactively through the Proxy so it always calls the current callback.
  onMount(() => {
    window.addEventListener("keydown", handleKeyDown);
    onCleanup(() => window.removeEventListener("keydown", handleKeyDown));
  });

  const thumbSrc = () => {
    const g = props.game;
    if (!g) { return null; }
    const path = bestThumbnailPath(g.torrent_source, g.thumbnail_key);
    if (!path) { return null; }
    return convertFileSrc(path);
  };

  const langEntries = () => props.game ? parseLangEntries(props.game) : [];
  const isMultiLang = () => langEntries().length > 1;

  const dlState = () => {
    const g = props.game;
    if (!g) { return undefined; }
    const dl = downloads();
    if (g.id != null && dl[g.id]) { return dl[g.id]; }
    for (const v of variants()) {
      if (v.id != null && dl[v.id]?.downloading) { return dl[v.id]; }
    }
    return undefined;
  };

  const isDownloading = () => dlState()?.downloading ?? false;
  const isInstalled = () => (props.game?.installed ?? false) || dlState()?.status === "Installed!";
  const currentProgress = () => dlState()?.progress ?? 0;
  const currentStatus = () => {
    const dl = dlState();
    if (dl) {
      if (dl.status === "Installed!") { return "Installed!"; }
      if (dl.status === "Extracting...") { return "Installing…"; }
      if (dl.downloading) { return "Downloading…"; }
      return dl.status; // error messages
    }
    return status();
  };

  const handleDownload = (gameId: number, title?: string) => {
    startGameDownload(gameId, title ?? props.game?.title);
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
    // Capture shortcode + title now - props.game may change before the async callback runs.
    const shortcode = props.game?.shortcode;
    const title = variants().find((v) => v.id === gameId)?.title ?? props.game?.title;
    await performUninstall(gameId, setStatus, async () => {
      if (shortcode) {
        const v = await getGameVariants(shortcode).catch(() => []);
        setVariants(v);
      }
    }, title);
  };

  const ratingStars = (rating: number | null) => {
    if (rating == null) { return null; }
    // eXoDOS ratings are 0–5 scale
    const full = Math.round(rating);
    const empty = 5 - full;
    return "★".repeat(full) + "☆".repeat(empty);
  };

  // Shared "Play" button - same disabled+spinner UX whether it's the main
  // single-language action or one row of the multi-language variant list.
  const PlayButton = (p: { id: number; class: string }) => (
    <button
      class={p.class}
      onClick={() => handleLaunch(p.id)}
      disabled={launchingId() === p.id}
    >
      <Show when={launchingId() === p.id} fallback={<>▶ Play</>}>
        <span class="btn-spinner" /> Starting…
      </Show>
    </button>
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
            <Show when={thumbSrc() && !imgError()}>
              <img
                class="game-detail-thumb"
                src={thumbSrc()!}
                alt=""
                onError={() => setImgError(true)}
                onClick={() => { setLightboxStart(0); setLightboxOpen(true); }}
              />
            </Show>
            <Show when={!thumbSrc() || imgError()}>
              <div class="game-detail-thumb-placeholder" />
            </Show>
            <div class="game-detail-hero-info">
              <div class="game-detail-title">{props.game!.title}</div>
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

            {/* Single-language action */}
            <Show when={!isMultiLang()}>
              <div class="game-detail-actions">
                <Show when={isInstalled()}>
                  <PlayButton id={props.game!.id!} class="game-detail-btn btn-play" />
                </Show>
                {/* Manual: render a placeholder while metadata is loading so the
                    real button doesn't pop in and shift the action row. After
                    load, swap to real button (when manual exists) or hide. */}
                <Show when={isInstalled() && (metadataLoading() || metadata()?.manual_path)}>
                  <button
                    class="game-detail-btn btn-manual"
                    onClick={() => { if (metadata()?.manual_path) { setManualOpen(true); } }}
                    disabled={metadataLoading()}
                  >
                    <Show when={metadataLoading()} fallback={<>⊞ Manual</>}>
                      <span class="btn-spinner" /> Manual
                    </Show>
                  </button>
                </Show>
                <Show when={isInstalled()}>
                  <button class="game-detail-btn btn-settings" onClick={() => setSettingsOpen(true)}>
                    ⚙
                  </button>
                </Show>
                <Show when={!isInstalled() && isDownloading()}>
                  <div class="game-detail-btn btn-downloading">
                    <AutoProgress
                      value={currentProgress()}
                      class="mini"
                      indeterminate={dlState()?.status?.startsWith("Waiting") || dlState()?.status?.startsWith("Extracting") || undefined}
                    />
                    <span>{dlState()?.status}</span>
                  </div>
                  <button class="game-detail-btn btn-cancel" onClick={() => cancelGameDownload(props.game!.id!)}>
                    ✕ Cancel
                  </button>
                </Show>
                <Show when={!isInstalled() && !isDownloading() && props.game!.game_torrent_index != null}>
                  <button class="game-detail-btn btn-download" onClick={() => handleDownload(props.game!.id!)}>
                    {props.game!.in_library
                      ? "↓ Re-download"
                      : `↓ Download ${props.game!.download_size ? formatBytes(props.game!.download_size) : ""}`}
                  </button>
                </Show>
                <Show when={!isDownloading() && (isInstalled() || props.game!.in_library) && props.game!.id != null}>
                  <button class="game-detail-btn btn-uninstall" onClick={() => handleUninstall(props.game!.id!)}>
                    Uninstall
                  </button>
                </Show>
              </div>
            </Show>

            {/* Multi-language variant list */}
            <Show when={isMultiLang()}>
              <div class="game-detail-langs">
                <div class="game-detail-section-label">Versions</div>
                <Show when={variants().length === 0}>
                  <div class="game-detail-loading">Loading…</div>
                </Show>
                <For each={variants()}>
                  {(variant) => {
                    const vId = () => variant.id;
                    const vDl = () => vId() != null ? getDownloadState(vId()!) : undefined;
                    return (
                      <div class="game-detail-lang-row">
                        <span class={`badge badge-lang ${langBadgeClass(variant.installed ? 2 : variant.in_library ? 1 : 0)}`}>
                          {variant.language}
                        </span>
                        <span class="game-detail-lang-title">{variant.title}</span>
                        <Show when={vDl()?.downloading}>
                          <div class="game-detail-lang-progress">
                            <AutoProgress value={vDl()?.progress ?? 0} class="mini" />
                          </div>
                          <button class="lang-picker-btn action-cancel" onClick={() => cancelGameDownload(vId()!)}>✕</button>
                        </Show>
                        <Show when={!vDl()?.downloading && variant.installed}>
                          <PlayButton id={vId()!} class="lang-picker-btn action-play" />
                          <button class="lang-picker-btn action-uninstall" onClick={() => handleUninstall(vId()!)}>✕</button>
                        </Show>
                        <Show when={!vDl()?.downloading && !variant.installed}>
                          <button
                            class="lang-picker-btn action-download"
                            onClick={() => { if (variant.game_torrent_index != null) { handleDownload(vId()!, variant.title); } }}
                          >
                            {variant.game_torrent_index != null ? `↓ ${formatBytes(variant.download_size ?? 0)}` : "-"}
                          </button>
                        </Show>
                      </div>
                    );
                  }}
                </For>
              </div>
            </Show>

            {/* Detail fields - structured key/value rows for metadata that
                doesn't fit in chips. */}
            <div class="game-detail-fields">
              <Show when={props.game!.developer}>
                <div class="game-detail-field">
                  <span class="game-detail-field-label">Developer</span>
                  <span>{props.game!.developer}</span>
                </div>
              </Show>
              <Show when={props.game!.publisher}>
                <div class="game-detail-field">
                  <span class="game-detail-field-label">Publisher</span>
                  <span>{props.game!.publisher}</span>
                </div>
              </Show>
              <Show when={props.game!.series}>
                <div class="game-detail-field">
                  <span class="game-detail-field-label">Series</span>
                  <span>{props.game!.series}</span>
                </div>
              </Show>
              <Show when={allGenres()}>
                <div class="game-detail-field">
                  <span class="game-detail-field-label">Genre</span>
                  <span>{allGenres()}</span>
                </div>
              </Show>
              <Show when={props.game!.play_mode}>
                <div class="game-detail-field">
                  <span class="game-detail-field-label">Mode</span>
                  <span>{props.game!.play_mode}</span>
                </div>
              </Show>
              <Show when={props.game!.region}>
                <div class="game-detail-field">
                  <span class="game-detail-field-label">Region</span>
                  <span>{props.game!.region}</span>
                </div>
              </Show>
              <Show when={props.game!.max_players != null}>
                <div class="game-detail-field">
                  <span class="game-detail-field-label">Players</span>
                  <span>{props.game!.max_players}</span>
                </div>
              </Show>
              <Show when={props.game!.rating != null}>
                <div class="game-detail-field">
                  <span class="game-detail-field-label">Rating</span>
                  <span class="game-detail-stars">{ratingStars(props.game!.rating)}</span>
                </div>
              </Show>
            </div>

            {/* Scrollable section: long-form text. Pinned between fixed fields
                above and screenshots below so the gallery is always reachable
                without scrolling past the description first. */}
            <div class="game-detail-scroll">
              <Show when={props.game!.description}>
                <div class="game-detail-description">{props.game!.description}</div>
              </Show>
              <Show when={props.game!.notes}>
                <div class="game-detail-notes">{props.game!.notes}</div>
              </Show>
              <Show when={metadataLoading()}>
                <div class="game-detail-loading">Loading media…</div>
              </Show>
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
                              src={convertFileSrc(path)}
                              class="gallery-thumb"
                              loading="lazy"
                              alt=""
                              onClick={() => {
                                const vi = visible().indexOf(path);
                                setLightboxStart(vi >= 0 ? vi : 0);
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
          gameId={props.game?.id ?? null}
          gameTitle={props.game?.title ?? ""}
          open={settingsOpen()}
          onClose={() => setSettingsOpen(false)}
        />
      </Portal>
    </Show>
  );
}
