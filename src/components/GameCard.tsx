import { createSignal, createEffect, on, onCleanup, onMount, Show, For } from "solid-js";
import { Portal } from "solid-js/web";
import { convertFileSrc } from "@tauri-apps/api/core";
import { CircularProgress } from "./ProgressBar";
import type { Game } from "../api/tauri";
import { loadVariants } from "../stores/variants";
import { formatBytes, parseLangEntries, langBadgeClass, performUninstall } from "../util";
import { thumbnailCandidates } from "../stores/thumbnails";
import { observeNearViewport, unobserveNearViewport } from "../nearViewport";
import { downloads, cancelGameDownload } from "../stores/downloads";
import { isOffline } from "../stores/network";
import { toggleFavorite } from "../stores/games";
import { PlaylistMenu } from "./PlaylistMenu";

interface GameCardProps {
  game: Game;
  onFavoriteChanged?: (id: number, favorited: boolean) => void;
  showFavoriteBtn?: boolean;
  onDetail: (game: Game) => void;
}

export function GameCard(props: GameCardProps) {
  const [status, setStatus] = createSignal("");
  const [imgError, setImgError] = createSignal(false);
  // Index into `thumbnailCandidates()` - advances on each <img onError> so
  // a stale poster dir (shortcode-keyed files from a previous Exodium version)
  // still falls through to the bundled preview on 404.
  const [thumbIdx, setThumbIdx] = createSignal(0);
  const [favorited, setFavorited] = createSignal(props.game.favorited);
  const [variants, setVariants] = createSignal<Game[]>([]);
  const [contextMenu, setContextMenu] = createSignal<{x: number, y: number} | null>(null);
  const [playlistMenu, setPlaylistMenu] = createSignal<{x: number, y: number} | null>(null);
  const [confirmUninstall, setConfirmUninstall] = createSignal(false);
  const [favAnimating, setFavAnimating] = createSignal(false);
  let favAnimTimeout: number | undefined;
  onCleanup(() => { if (favAnimTimeout) { clearTimeout(favAnimTimeout); } });

  // Re-sync favorited from props only when the card is reused for a different game (For loop
  // key change). Do NOT run on favorited-flag-only changes - that would race with the
  // optimistic update in handleToggleFavorite and cause a visible flicker.
  createEffect(on(() => props.game.id, () => { setFavorited(props.game.favorited); }, { defer: true }));

  // Reset thumbnail state when the card is reused for a different game (For-loop key change).
  createEffect(on(() => props.game.id, () => { setImgError(false); setThumbIdx(0); }, { defer: true }));

  // Pre-load variant IDs for multi-lang games so download state is visible on main card.
  // createEffect re-runs when props.game.shortcode changes, handling component reuse in For loops.
  createEffect(() => {
    const shortcode = props.game.shortcode;
    if (!isMultiLang() || !shortcode) { return; }
    loadVariants(props.game)
      .then((v) => { if (props.game.shortcode === shortcode) { setVariants(v); } })
      .catch(() => {});
  });

  // Covers load once the card is within ~2 screens of the viewport instead of
  // when it nearly enters it - see nearViewport.ts. Sticky once true: a card
  // scrolled back into view must not refetch.
  const [nearViewport, setNearViewport] = createSignal(false);
  let cardRef: HTMLDivElement | undefined;
  onMount(() => {
    if (cardRef) { observeNearViewport(cardRef, () => setNearViewport(true)); }
  });
  onCleanup(() => { if (cardRef) { unobserveNearViewport(cardRef); } });

  const thumbCandidates = () => thumbnailCandidates(props.game.torrent_source, props.game.thumbnail_key);
  const thumbSrc = () => {
    if (!nearViewport()) { return null; }
    const path = thumbCandidates()[thumbIdx()];
    if (!path) { return null; }
    return convertFileSrc(path);
  };

  const handleImgError = () => {
    // Advance to next candidate (e.g. poster URL 404'd → try bundled preview).
    // If we've exhausted them all, hide the tile.
    if (thumbIdx() + 1 < thumbCandidates().length) {
      setThumbIdx(thumbIdx() + 1);
    } else {
      setImgError(true);
    }
  };

  const langEntries = () => parseLangEntries(props.game);
  const isMultiLang = () => langEntries().length > 1;

  // Read download state - check primary game and any loaded variants.
  // Tracks WHICH id the state came from: on a merged card the overlay can
  // show a variant's download, and the cancel button must target that id,
  // not always the primary (which would no-op).
  const dlEntry = () => {
    const dl = downloads();
    if (props.game.id != null && dl[props.game.id]) {
      return { id: props.game.id, state: dl[props.game.id] };
    }
    for (const v of variants()) {
      if (v.id != null && dl[v.id]?.downloading) { return { id: v.id, state: dl[v.id] }; }
    }
    return undefined;
  };
  const dlState = () => dlEntry()?.state;

  const handleContextMenu = (e: MouseEvent) => {
    if (props.game.id == null) { return; }
    e.preventDefault();
    setConfirmUninstall(false);
    setContextMenu({ x: e.clientX, y: e.clientY });
  };

  // Uninstall stays hidden while a download is in flight - performUninstall
  // would cancel it first, but exposing both actions side-by-side is confusing.
  const canUninstall = () =>
    (props.game.installed || props.game.in_library) && !isDownloading();

  const handleContextUninstall = async () => {
    setContextMenu(null);
    if (props.game.id == null) { return; }
    await performUninstall(props.game.id, setStatus, undefined, props.game.title);
  };

  const handleClick = (e: MouseEvent) => {
    e.stopPropagation();
    props.onDetail(props.game);
  };

  const handleToggleFavorite = async (e: MouseEvent) => {
    e.stopPropagation();
    if (props.game.id == null) { return; }
    const prev = favorited();
    setFavorited(!prev);
    // Retrigger CSS animation by flipping off-then-on across a frame - just
    // setting true-to-true wouldn't restart a keyframe animation already in
    // flight (e.g. double-click taps). Clear any previously-scheduled
    // turn-off so a second click within 500ms doesn't clip its own animation.
    if (favAnimTimeout) { clearTimeout(favAnimTimeout); }
    setFavAnimating(false);
    requestAnimationFrame(() => setFavAnimating(true));
    favAnimTimeout = window.setTimeout(() => setFavAnimating(false), 500);
    try {
      const next = await toggleFavorite(props.game.id);
      setFavorited(next);
      props.onFavoriteChanged?.(props.game.id, next);
    } catch {
      setFavorited(prev);
    }
  };

  const currentProgress = () => dlState()?.progress ?? 0;
  const isDownloading = () => dlState()?.downloading ?? false;

  return (
    <div ref={cardRef} class={`game-card ${props.game.installed || props.game.in_library ? "installed" : ""}`} onContextMenu={handleContextMenu} data-game-id={props.game.id != null ? String(props.game.id) : undefined}>
      <div onClick={handleClick}>
        <Show when={thumbSrc() && !imgError()}>
          <img
            class="game-card-thumb"
            src={thumbSrc()!}
            alt=""
            onError={handleImgError}
          />
        </Show>
        <Show when={isDownloading()}>
          <div class="game-card-download-overlay">
            <CircularProgress value={currentProgress()} size={64} strokeWidth={5}>
              <Show when={currentProgress() > 0} fallback={<span class="circular-progress-pct muted">…</span>}>
                <span class="circular-progress-pct">{Math.round(currentProgress() * 100)}%</span>
              </Show>
            </CircularProgress>
            <Show when={dlEntry() != null}>
              <button class="game-card-overlay-cancel"
                title="Cancel download"
                onClick={(e) => { e.stopPropagation(); cancelGameDownload(dlEntry()!.id); }}>✕</button>
            </Show>
          </div>
        </Show>
        <div class="game-card-body">
          <div class="game-card-title">{props.game.title}</div>
          <div class="game-card-meta">
            {props.game.year && <span>{props.game.year}</span>}
            {props.game.genre && <span class="genre">{props.game.genre}</span>}
          </div>
          <div class="game-card-footer">
            <For each={langEntries()}>
              {(entry) => (
                <span class={`badge badge-lang ${langBadgeClass(entry.state)}`}>
                  {entry.lang}
                </span>
              )}
            </For>
          </div>
          <div class="game-card-action-bar">
            <Show when={status()}>
              <span class="card-action-label action-downloading fade-swap">{status()}</span>
            </Show>
            <Show when={!status()}>
              <Show when={isDownloading()}>
                <span class="card-action-label action-downloading">{dlState()?.status}</span>
              </Show>
              <Show when={!isDownloading() && props.game.installed}>
                <span class="card-action-label action-installed">▶ Play</span>
              </Show>
              <Show when={!isDownloading() && !props.game.installed && props.game.in_library}>
                <span class="card-action-label action-incomplete">⚠ Incomplete</span>
              </Show>
              <Show when={!isDownloading() && !props.game.installed && !props.game.in_library}>
                <span class={`card-action-label ${isOffline() ? "action-offline" : "action-download"}`}>
                  <Show when={!isOffline()} fallback="Not installed">
                    {props.game.download_size ? `↓ ${formatBytes(props.game.download_size)}` : "↓ Download"}
                  </Show>
                </span>
              </Show>
            </Show>
          </div>
        </div>
      </div>

      <Show when={props.game.id != null && props.showFavoriteBtn !== false}>
        <button
          class={`favorite-btn${favorited() ? " is-favorited" : ""}${favAnimating() ? " animating" : ""}`}
          onClick={handleToggleFavorite}
          title={favorited() ? "Remove from favorites" : "Add to favorites"}
        >
          <span class="fav-star">★</span>
          <Show when={favAnimating() && favorited()}>
            <span class="fav-ring" />
            <span class="fav-sparks">
              <For each={[0, 1, 2, 3, 4, 5]}>
                {(i) => <span class="fav-spark" style={{ "--angle": `${i * 60}deg` }} />}
              </For>
            </span>
          </Show>
        </button>
      </Show>

      <Show when={contextMenu()}>
        <Portal>
          <div class="context-backdrop" onMouseDown={() => setContextMenu(null)} onContextMenu={(e) => { e.preventDefault(); setContextMenu(null); }} />
          <div class="context-menu" style={{ left: `${contextMenu()!.x}px`, top: `${contextMenu()!.y}px` }}>
            <button class="context-menu-item" onMouseDown={(e) => e.stopPropagation()} onClick={() => {
              const pos = contextMenu()!;
              setContextMenu(null);
              setPlaylistMenu(pos);
            }}>
              Add to playlist…
            </button>
            <Show when={canUninstall()}>
              <button class="context-menu-item danger" onMouseDown={(e) => e.stopPropagation()} onClick={() => {
                if (confirmUninstall()) {
                  handleContextUninstall();
                } else {
                  setConfirmUninstall(true);
                }
              }}>
                {confirmUninstall() ? "Confirm uninstall?" : "Uninstall"}
              </button>
            </Show>
          </div>
        </Portal>
      </Show>

      <Show when={playlistMenu()}>
        <PlaylistMenu
          x={playlistMenu()!.x}
          y={playlistMenu()!.y}
          gameId={props.game.id!}
          onClose={() => setPlaylistMenu(null)}
        />
      </Show>
    </div>
  );
}
