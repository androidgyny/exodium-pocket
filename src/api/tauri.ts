import { invoke } from "@tauri-apps/api/core";

export interface Game {
  id: number | null;
  title: string;
  sort_title: string | null;
  platform: string;
  developer: string | null;
  publisher: string | null;
  release_date: string | null;
  year: number | null;
  genre: string | null;
  series: string | null;
  play_mode: string | null;
  rating: number | null;
  description: string | null;
  notes: string | null;
  source: string | null;
  application_path: string | null;
  dosbox_conf: string | null;
  status: string | null;
  region: string | null;
  max_players: number | null;
  language: string;
  shortcode: string | null;
  available_languages: string | null;
  /** Titles of the other language variants, unit-separated. Present only on
   *  merged multi-language rows; lets a local search match a localized title. */
  variant_titles: string | null;
  torrent_source: string | null;
  in_library: boolean;
  installed: boolean;
  favorited: boolean;
  game_torrent_index: number | null;
  gamedata_torrent_index: number | null;
  download_size: number | null;
  has_thumbnail: boolean;
  dosbox_variant: string | null;
  /** SHA-256(normalized title)[:16] - filename stem for the bundled or
   *  content-pack thumbnail. Null when no title was available at DB-build
   *  time (very rare). Frontend builds `<preview_dir>/${thumbnail_key}.jpg`. */
  thumbnail_key: string | null;
  manual_path: string | null;
  last_played: string | null;
}

export interface GameList {
  games: Game[];
  total: number;
}

export async function getGames(
  page?: number,
  perPage?: number,
  query?: string,
  genre?: string,
  sortBy?: string,
  collection?: string,
  favoritesOnly?: boolean,
  playlistId?: number | null
): Promise<GameList> {
  return invoke("get_games", { page, perPage, query, genre, sortBy, collection, favoritesOnly, playlistId });
}

export async function toggleFavorite(id: number): Promise<boolean> {
  return invoke("toggle_favorite", { id });
}

export async function cancelDownload(id: number): Promise<void> {
  return invoke("cancel_download", { id });
}

export async function getGenres(collection?: string): Promise<string[]> {
  return invoke("get_genres", { collection });
}

export async function getSectionKeys(
  sortBy?: string,
  query?: string,
  genre?: string,
  collection?: string,
  favoritesOnly?: boolean,
  playlistId?: number | null,
): Promise<string[]> {
  return invoke("get_section_keys", { sortBy, query, genre, collection, favoritesOnly, playlistId });
}

// ── Playlists ────────────────────────────────────────────────────────────────

export interface Playlist {
  id: number;
  name: string;
  /** "curated" (shipped with the catalog, read-only) or "user". */
  kind: "curated" | "user";
  description: string | null;
  game_count: number;
}

export async function getPlaylists(): Promise<Playlist[]> {
  return invoke("get_playlists");
}

export async function createPlaylist(name: string): Promise<number> {
  return invoke("create_playlist", { name });
}

export async function renamePlaylist(id: number, name: string): Promise<void> {
  return invoke("rename_playlist", { id, name });
}

export async function deletePlaylist(id: number): Promise<void> {
  return invoke("delete_playlist", { id });
}

export async function setPlaylistMembership(
  playlistId: number,
  gameId: number,
  member: boolean,
): Promise<void> {
  return invoke("set_playlist_membership", { playlistId, gameId, member });
}

export async function getGamePlaylists(gameId: number): Promise<number[]> {
  return invoke("get_game_playlists", { gameId });
}

export async function getThumbnailDir(collection: string): Promise<string> {
  return invoke("get_thumbnail_dir", { collection });
}

export async function getGameVariants(shortcode: string, collection: string): Promise<Game[]> {
  return invoke("get_game_variants", { shortcode, collection });
}

export async function getInstalledGames(): Promise<Game[]> {
  return invoke("get_installed_games");
}

export async function getRecentlyPlayed(limit?: number): Promise<Game[]> {
  return invoke("get_recently_played", { limit });
}

export interface GameSettings {
  glshader: string | null;
  fullscreen: string | null;
  cycles: string | null;
  custom_conf: string | null;
}

export async function getGameSettings(id: number): Promise<GameSettings> {
  return invoke("get_game_settings", { id });
}

export async function setGameSettings(
  id: number,
  glshader: string | null,
  fullscreen: string | null,
  cycles: string | null,
  customConf: string | null,
): Promise<void> {
  return invoke("set_game_settings", { id, glshader, fullscreen, cycles, customConf });
}

export async function getGame(id: number): Promise<Game | null> {
  return invoke("get_game", { id });
}


export async function launchGame(id: number): Promise<string> {
  return invoke("launch_game", { id });
}

/** Whether the game's printing features will be missing at launch (13 eXoDOS
 *  titles enable a virtual printer; Staging has none yet). The backend decides
 *  with the same engine-selection logic launch_game uses, so Windows + an
 *  installed ECE build correctly answers false. */
export async function gamePrintingUnavailable(id: number): Promise<boolean> {
  return invoke("game_printing_unavailable", { id });
}

export async function getConfig(key: string): Promise<string | null> {
  return invoke("get_config", { key });
}

export async function setConfig(key: string, value: string): Promise<void> {
  return invoke("set_config", { key, value });
}

// Opens a manual in the system viewer. Path validation happens in Rust
// (must be under the data dir), so no broad opener capability is needed.
export async function openManual(path: string): Promise<void> {
  return invoke("open_manual", { path });
}

export async function setSeedingEnabled(enabled: boolean): Promise<void> {
  return invoke("set_seeding_enabled", { enabled });
}

export interface TransferStats {
  download_bps: number;
  upload_bps: number;
  uploaded_bytes: number;
  /** Connected peers across all collections - the readout that shows liveness
   *  when the rates are zero. */
  peers: number;
  /** False when no torrent is live - distinct from a live 0 B/s. */
  active: boolean;
}

export async function getTransferStats(): Promise<TransferStats> {
  return invoke("get_transfer_stats");
}

/** Transfer caps in KB/s; `null` means unlimited. */
export async function setRateLimits(upKbps: number | null, downKbps: number | null): Promise<void> {
  return invoke("set_rate_limits", { upKbps, downKbps });
}

export interface TorrentInfo {
  name: string;
  file_count: number;
  total_size: number;
  metadata_size: number;
}

export interface DownloadProgress {
  file_index: number;
  file_name: string;
  downloaded_bytes: number;
  total_bytes: number;
  progress: number;
  finished: boolean;
  installed: boolean;
  error: string | null;
  /** "initializing" while librqbit hashes existing on-disk files; on Windows
   *  with thousands of placeholders this can take 5–10 minutes for a 250GB+
   *  torrent before any pieces transfer. */
  torrent_state?: string | null;
  /** 0..1 of whole-torrent progress. During init = validation pass; once live
   *  = cumulative download. Used for the "Validating…" UI status. */
  torrent_progress?: number | null;
  /** 0..1 progress of the game's extras (GameData: manuals, videos, music) -
   *  they keep downloading after the game itself is installed. */
  extras_progress?: number | null;
  extras_done?: boolean | null;
}

export interface SetupStatus {
  phase: string;
  metadata_progress: DownloadProgress | null;
  dosbox_metadata_progress: DownloadProgress | null;
  games_imported: number;
  ready: boolean;
}

export async function getDefaultDataDir(): Promise<string> {
  return invoke("get_default_data_dir");
}

export async function getTorrentInfo(): Promise<TorrentInfo> {
  return invoke("get_torrent_info");
}

export async function setupStart(dataDir: string): Promise<string> {
  return invoke("setup_start", { dataDir });
}

export async function getSetupStatus(): Promise<SetupStatus> {
  return invoke("get_setup_status");
}

export async function setupImport(): Promise<number> {
  return invoke("setup_import");
}

export async function setupFromLocal(exodosPath: string): Promise<number> {
  return invoke("setup_from_local", { exodosPath });
}

export interface ExodosValidation {
  valid: boolean;
  hint: string;
}

export async function validateExodosDir(path: string): Promise<ExodosValidation> {
  return invoke("validate_exodos_dir", { path });
}

export async function initDownloadManager(): Promise<boolean> {
  return invoke("init_download_manager");
}

export async function factoryReset(deleteGameData: boolean): Promise<void> {
  return invoke("factory_reset", { deleteGameData });
}

export async function uninstallGame(id: number): Promise<string> {
  return invoke("uninstall_game", { id });
}

export async function downloadGame(id: number): Promise<string> {
  return invoke("download_game", { id });
}

export async function getDownloadProgress(id: number): Promise<DownloadProgress | null> {
  return invoke("get_download_progress", { id });
}

export interface CollectionUpdate {
  collection: string;
  current_hash: string;
  latest_hash: string;
  new_game_count: number;
}

export interface CollectionInfo {
  id: string;
  display_name: string;
  torrent_file: string;
  /** Catalogue rows in this collection - shown on the collection shelf. */
  game_count: number;
}

export async function getAvailableCollections(): Promise<CollectionInfo[]> {
  return invoke("get_available_collections");
}

export async function scanInstalledGames(): Promise<number> {
  return invoke("scan_installed_games");
}

export async function getLogDir(): Promise<string> {
  return invoke("get_log_dir");
}

export async function openLogFolder(): Promise<void> {
  return invoke("open_log_folder");
}

// ── Content Packs ────────────────────────────────────────────────────────────

export interface ContentPackStatus {
  id: string;
  display_name: string;
  description: string;
  size_bytes: number;
  version: number;
  supersedes: string[];
  available: boolean;
  installed: boolean;
  installed_version?: number;
}

export interface ContentPackProgress {
  phase: string;
  downloaded_bytes: number;
  total_bytes: number;
  progress: number;
  finished: boolean;
  installed: boolean;
  error: string | null;
}

export async function listContentPacks(collection: string): Promise<ContentPackStatus[]> {
  return invoke("list_content_packs", { collection });
}

export async function installContentPack(collection: string, packId: string): Promise<void> {
  return invoke("install_content_pack", { collection, packId });
}

export async function uninstallContentPack(collection: string, packId: string): Promise<void> {
  return invoke("uninstall_content_pack", { collection, packId });
}

export async function getContentPackProgress(
  collection: string,
  packId: string,
): Promise<ContentPackProgress | null> {
  return invoke("get_content_pack_progress", { collection, packId });
}

export async function cancelContentPackInstall(collection: string, packId: string): Promise<void> {
  return invoke("cancel_content_pack_install", { collection, packId });
}

export async function getPreviewDir(collection: string): Promise<string> {
  return invoke("get_preview_dir", { collection });
}

export async function getPosterDir(collection: string): Promise<string> {
  return invoke("get_poster_dir", { collection });
}

export interface GameMetadata {
  manual_path: string | null;
  manual_kind: "pdf" | "txt" | "html" | null;
  /** Full-resolution paths - what the lightbox opens. */
  images: string[];
  /** Cached 160px copies, aligned 1:1 with `images` - what the strip renders.
   *  An entry equals its `images` counterpart when no thumbnail could be made. */
  thumbnails: string[];
}

export interface VideoStatus {
  /** "fetching" | "ready" | "none" | "error" */
  phase: string;
  /** 0..1 while fetching. */
  progress: number;
  total_bytes: number;
  path: string | null;
  error: string | null;
}

/** Start (or join) the fetch of a game's preview video. Returns immediately -
 *  the video is streamed out of the GameData archive, which can take a minute
 *  on a cold torrent. Poll getVideoStatus. */
export async function startGameVideo(id: number): Promise<VideoStatus> {
  return invoke("start_game_video", { id });
}

export async function getVideoStatus(id: number): Promise<VideoStatus | null> {
  return invoke("get_video_status", { id });
}

export async function cancelGameVideo(id: number): Promise<void> {
  return invoke("cancel_game_video", { id });
}

export async function getGameMetadata(
  collection: string,
  title: string,
  shortcode: string | null,
  manualPath: string | null,
): Promise<GameMetadata> {
  return invoke("get_game_metadata", { collection, title, shortcode, manualPath });
}
