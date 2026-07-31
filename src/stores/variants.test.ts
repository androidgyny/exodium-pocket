import { describe, it, expect, vi, beforeEach } from "vitest";
import { invoke } from "@tauri-apps/api/core";

const mockInvoke = vi.mocked(invoke);

describe("variant cache", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    vi.resetModules();
  });

  // Rendering the whole catalogue mounts one card per game; before the cache
  // each multi-language card fired its own get_game_variants (~734 in a burst).
  it("shares one request between concurrent callers", async () => {
    mockInvoke.mockResolvedValue([{ id: 1 }]);
    const { loadVariants } = await import("./variants");

    const results = await Promise.all([
      loadVariants("MagCarp"),
      loadVariants("MagCarp"),
      loadVariants("MagCarp"),
    ]);

    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(results.every((r) => r.length === 1)).toBe(true);
  });

  it("serves later callers from cache", async () => {
    mockInvoke.mockResolvedValue([{ id: 1 }]);
    const { loadVariants } = await import("./variants");

    await loadVariants("SQ5");
    await loadVariants("SQ5");
    expect(mockInvoke).toHaveBeenCalledTimes(1);
  });

  it("refetches when forced - install state must not be served stale", async () => {
    mockInvoke.mockResolvedValue([{ id: 1 }]);
    const { loadVariants } = await import("./variants");

    await loadVariants("DESCENT");
    await loadVariants("DESCENT", true);
    expect(mockInvoke).toHaveBeenCalledTimes(2);
  });

  // A failed lookup must not be remembered, or one hiccup would leave a card
  // without variants for the rest of the session.
  it("does not cache failures", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("db locked"));
    mockInvoke.mockResolvedValue([{ id: 2 }]);
    const { loadVariants } = await import("./variants");

    await expect(loadVariants("BOOM")).rejects.toThrow("db locked");
    const second = await loadVariants("BOOM");
    expect(second).toEqual([{ id: 2 }]);
    expect(mockInvoke).toHaveBeenCalledTimes(2);
  });
});
