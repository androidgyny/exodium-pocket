import { createSignal, createEffect, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { Dialog } from "@ark-ui/solid/dialog";
import {
  playlistDialog, setPlaylistDialog, createPlaylist, renamePlaylist,
  togglePlaylistMembership,
} from "../stores/playlists";

/** App-wide create/rename playlist dialog, driven by the playlistDialog
 *  store signal. Mounted once in Library. */
export function PlaylistNameDialog() {
  const [name, setName] = createSignal("");
  const [error, setError] = createSignal("");
  const [saving, setSaving] = createSignal(false);
  let inputRef: HTMLInputElement | undefined;

  const request = () => playlistDialog();

  createEffect(() => {
    const req = request();
    if (!req) { return; }
    setName(req.mode === "rename" ? req.playlist.name : "");
    setError("");
    // Focus after the dialog content mounts.
    requestAnimationFrame(() => inputRef?.select());
  });

  const close = () => setPlaylistDialog(null);

  const handleSave = async () => {
    const req = request();
    const trimmed = name().trim();
    if (!req || !trimmed || saving()) { return; }
    setSaving(true);
    setError("");
    try {
      if (req.mode === "create") {
        const id = await createPlaylist(trimmed);
        if (req.gameId != null) {
          await togglePlaylistMembership(id, req.gameId, true);
        }
      } else {
        await renamePlaylist(req.playlist.id, trimmed);
      }
      close();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <Show when={request()}>
      <Dialog.Root open onOpenChange={(e) => { if (!e.open) { close(); } }}>
        <Portal>
          <Dialog.Backdrop class="ark-dialog-backdrop" />
          <Dialog.Positioner class="ark-dialog-positioner">
            <Dialog.Content class="ark-dialog-content">
              <Dialog.Title class="ark-dialog-title">
                {request()!.mode === "create" ? "New playlist" : "Rename playlist"}
              </Dialog.Title>
              <input
                ref={inputRef}
                class="playlist-name-input"
                type="text"
                value={name()}
                placeholder="Playlist name"
                maxLength={80}
                onInput={(e) => setName(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") { handleSave(); }
                }}
              />
              <Show when={error()}>
                <div class="playlist-name-error">{error()}</div>
              </Show>
              <div class="ark-dialog-actions">
                <button class="btn-secondary" onClick={close}>Cancel</button>
                <button
                  class="btn-primary"
                  onClick={handleSave}
                  disabled={!name().trim() || saving()}
                >
                  {saving() ? "Saving…" : request()!.mode === "create" ? "Create" : "Rename"}
                </button>
              </div>
            </Dialog.Content>
          </Dialog.Positioner>
        </Portal>
      </Dialog.Root>
    </Show>
  );
}
