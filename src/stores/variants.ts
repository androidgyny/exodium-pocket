import { createEffect } from "solid-js";
import { getGameVariants, type Game } from "../api/tauri";
import { lastGameLibraryChange } from "./games";

/** Shared cache for `get_game_variants`.
 *
 *  Every multi-language GameCard asks for its group's variants from an effect.
 *  Rendering a page is fine; rendering the whole catalogue (what a jump-bar
 *  jump does) fired one IPC call per multi-language card - ~734 of them in a
 *  burst. Requests are now deduplicated by shortcode, and the resolved list is
 *  reused until something changes a game's library state.
 *
 *  In-flight promises are cached too, so N cards mounting in the same frame
 *  share a single round trip. */
const cache = new Map<string, Promise<Game[]>>();

// installed/in_library flags are baked into the cached rows, so anything that
// changes them invalidates the cache wholesale (there are at most a few
// hundred entries; targeted eviction isn't worth the bookkeeping).
createEffect(() => {
  lastGameLibraryChange();
  cache.clear();
});

export function loadVariants(shortcode: string, force = false): Promise<Game[]> {
  if (force) { cache.delete(shortcode); }
  const hit = cache.get(shortcode);
  if (hit) { return hit; }
  const request = getGameVariants(shortcode).catch((e) => {
    // Don't cache a failure - the next card (or a retry) should try again.
    cache.delete(shortcode);
    throw e;
  });
  cache.set(shortcode, request);
  return request;
}

/** Drop cached rows for one group - used after an uninstall, where the panel
 *  needs the fresh state immediately rather than at the next invalidation. */
export function invalidateVariants(shortcode: string) {
  cache.delete(shortcode);
}
