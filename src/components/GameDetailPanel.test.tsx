import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render } from "solid-js/web";
import { invoke } from "@tauri-apps/api/core";
import type { Game } from "../api/tauri";
import { GameDetailPanel } from "./GameDetailPanel";

const mockInvoke = vi.mocked(invoke);

/** Minimal row shaped like the merged card the grid hands to the panel. */
function makeGame(over: Partial<Game> = {}): Game {
  return {
    id: 1,
    title: "Magic Carpet Plus",
    sort_title: "Magic Carpet Plus",
    platform: "MS-DOS",
    developer: "Bullfrog Productions, Ltd.",
    publisher: "Electronic Arts, Inc.",
    release_date: null,
    year: 1995,
    genre: "Action;Flight Simulator",
    series: "Magic Carpet series",
    play_mode: "Single Player",
    rating: 5,
    description: "English description text.",
    notes: null,
    source: null,
    application_path: null,
    dosbox_conf: null,
    status: null,
    region: null,
    max_players: 8,
    language: "EN",
    shortcode: "MagCarp",
    torrent_source: "eXoDOS",
    in_library: false,
    installed: false,
    game_torrent_index: 10,
    gamedata_torrent_index: null,
    download_size: 268_000_000,
    has_thumbnail: true,
    dosbox_variant: null,
    favorited: false,
    thumbnail_key: "abc123",
    manual_path: "Manuals\\MS-DOS\\Magic Carpet Plus (1995).pdf",
    last_played: null,
    available_languages: null,
    ...over,
  } as Game;
}

const EMPTY_META = { manual_path: null, manual_kind: null, images: [], thumbnails: [] };
const VIDEO_READY = {
  phase: "ready", progress: 1, total_bytes: 2_000_000,
  path: "/data/content/videocache/eXoDOS_1.mp4", error: null,
};

/** Render into a detached container and return it plus a disposer. Solid's
 *  render() flushes effects, so anything that throws at effect time (a helper
 *  used before its `const` is initialised, say) surfaces here - which is
 *  exactly what type-checking cannot catch. */
function mount(game: Game) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <GameDetailPanel game={game} onClose={() => {}} />, host);
  return { host, dispose };
}

describe("GameDetailPanel", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_game_metadata") { return EMPTY_META; }
      if (cmd === "get_game_variants") { return []; }
      return null;
    });
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.useRealTimers();
  });

  // The panel asks for a video 400ms after settling on a game, then plays it
  // in place of the cover. Reproduces "no video plays at all".
  it("shows the preview video once the backend reports it ready", async () => {
    vi.useFakeTimers();
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_game_metadata") { return EMPTY_META; }
      if (cmd === "get_game_variants") { return []; }
      if (cmd === "start_game_video") { return VIDEO_READY; }
      if (cmd === "get_video_status") { return VIDEO_READY; }
      return null;
    });

    const { host, dispose } = mount(makeGame({ id: 42, shortcode: "VID42" }));
    await vi.advanceTimersByTimeAsync(1200);

    const video = host.ownerDocument.querySelector("video.game-detail-hero-video");
    expect(video, "the hero video element should be mounted").not.toBeNull();
    expect(video?.getAttribute("src") ?? "").toContain("videocache");
    // The cover crossfades out only once playback actually started.
    expect(video?.className).toContain("is-visible");
    dispose(); host.remove();
  });

  it("renders a single-language game without throwing", async () => {
    const { host, dispose } = mount(makeGame());
    await Promise.resolve();
    const text = document.body.textContent ?? "";
    expect(text).toContain("Magic Carpet Plus");
    expect(text).toContain("English description text.");
    expect(text).toContain("Bullfrog Productions, Ltd.");
    dispose();
    host.remove();
  });

  it("offers Download for an uninstalled game and Play once installed", async () => {
    const a = mount(makeGame());
    await Promise.resolve();
    expect(document.body.textContent).toContain("Download");
    a.dispose(); a.host.remove();
    document.body.innerHTML = "";

    const b = mount(makeGame({ installed: true }));
    await Promise.resolve();
    expect(document.body.textContent).toContain("Play");
    b.dispose(); b.host.remove();
  });

  // The header names the row every button acts on. PL/ES variants carry
  // genuinely different titles, so showing the English one while DE is
  // selected would misidentify what Play/Uninstall would touch.
  it("titles the panel after the selected variant", async () => {
    const variants: Game[] = [
      makeGame({ id: 1, shortcode: "OFFICE", language: "EN", title: "The Office", installed: false }),
      makeGame({ id: 2, shortcode: "OFFICE", language: "DE", title: "Das Amt", installed: true }),
    ];
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_game_variants") { return variants; }
      if (cmd === "get_game_metadata") { return EMPTY_META; }
      return null;
    });

    const { host, dispose } = mount(makeGame({
      shortcode: "OFFICE", title: "The Office", available_languages: "EN:0,DE:2",
    }));
    await new Promise((r) => setTimeout(r, 0));

    expect(host.ownerDocument.querySelector(".game-detail-title")?.textContent).toBe("Das Amt");
    dispose();
    host.remove();
  });

  it("shows one chip per language and selects the installed variant", async () => {
    const variants: Game[] = [
      makeGame({ id: 1, language: "EN", installed: false }),
      makeGame({ id: 2, language: "DE", installed: true, description: null, manual_path: null,
                 torrent_source: "eXoDOS_GLP", developer: null, publisher: null }),
    ];
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "get_game_variants") { return variants; }
      if (cmd === "get_game_metadata") { return EMPTY_META; }
      return null;
    });

    const { host, dispose } = mount(makeGame({ available_languages: "EN:0,DE:2" }));
    // Variants arrive from an awaited invoke, so let the microtask queue drain.
    await new Promise((r) => setTimeout(r, 0));

    const chips = host.ownerDocument.querySelectorAll(".variant-chip");
    expect(chips.length).toBe(2);
    const selectedChip = host.ownerDocument.querySelector(".variant-chip.is-selected");
    // DE is installed, so it wins the default selection over the EN row.
    expect(selectedChip?.textContent).toContain("DE");

    const text = document.body.textContent ?? "";
    // DE has no text of its own - the English one is shown, and labelled.
    expect(text).toContain("English description text.");
    expect(text).toContain("no German text");
    // Fields fall back to the English row rather than rendering blank.
    expect(text).toContain("Bullfrog Productions, Ltd.");
    dispose();
    host.remove();
  });
});
