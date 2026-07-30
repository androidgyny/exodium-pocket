import { createSignal } from "solid-js";
import {
  getPlaylists, createPlaylist as apiCreate, renamePlaylist as apiRename,
  deletePlaylist as apiDelete, setPlaylistMembership, type Playlist,
} from "../api/tauri";

const [playlists, setPlaylists] = createSignal<Playlist[]>([]);
export { playlists };

export const userPlaylists = () => playlists().filter(p => p.kind === "user");
export const curatedPlaylists = () => playlists().filter(p => p.kind === "curated");

// Bumped on any playlist mutation (create/rename/delete/membership) so
// views holding derived data (My Library shelves) know to refetch.
const [lastPlaylistChange, setLastPlaylistChange] = createSignal(0);
export { lastPlaylistChange };
function notifyChanged() {
  setLastPlaylistChange(Date.now());
}

// Single app-wide name dialog (mounted once in Library): "create" optionally
// carries a game to add to the fresh playlist; "rename" carries the playlist.
export type PlaylistDialogRequest =
  | { mode: "create"; gameId?: number }
  | { mode: "rename"; playlist: Playlist };
const [playlistDialog, setPlaylistDialog] = createSignal<PlaylistDialogRequest | null>(null);
export { playlistDialog, setPlaylistDialog };

export async function loadPlaylists(): Promise<void> {
  try {
    setPlaylists(await getPlaylists());
  } catch (e) {
    console.warn("[playlists] load failed:", e);
  }
}

// After a mutation the list refresh is fire-and-forget: the caller (e.g.
// the name dialog's "Saving..." state) only needs the WRITE to be durable,
// not the derived views to be repainted.
function refreshInBackground() {
  loadPlaylists().then(notifyChanged);
}

/// Create and return the new playlist's id.
export async function createPlaylist(name: string): Promise<number> {
  const id = await apiCreate(name);
  refreshInBackground();
  return id;
}

export async function renamePlaylist(id: number, name: string): Promise<void> {
  await apiRename(id, name);
  refreshInBackground();
}

export async function deletePlaylist(id: number): Promise<void> {
  await apiDelete(id);
  refreshInBackground();
}

export async function togglePlaylistMembership(
  playlistId: number,
  gameId: number,
  member: boolean,
): Promise<void> {
  await setPlaylistMembership(playlistId, gameId, member);
  // Counts changed; shelves and dropdown labels follow in the background.
  refreshInBackground();
}
