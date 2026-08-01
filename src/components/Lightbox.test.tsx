import { describe, it, expect, afterEach } from "vitest";
import { render } from "solid-js/web";
import { Lightbox } from "./Lightbox";

/** The preview video occupies entry 0, so every screenshot index shifts by one.
 *  Getting that wrong opened the neighbouring screenshot. */
describe("Lightbox with a preview video", () => {
  afterEach(() => { document.body.innerHTML = ""; });

  function mount(props: any) {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <Lightbox {...props} />, host);
    return { host, dispose };
  }

  const images = ["/a.jpg", "/b.jpg", "/c.jpg"];

  it("opens the video when asked for entry 0", () => {
    const { dispose } = mount({ images, video: "asset://v.mp4", startIndex: 0, open: true, onClose: () => {} });
    expect(document.querySelector("video.lightbox-video")).not.toBeNull();
    expect(document.querySelector("img.lightbox-image")).toBeNull();
    dispose();
  });

  it("opens the screenshot the caller meant", () => {
    // Entry 2 = second screenshot, because the video sits at 0.
    const { dispose } = mount({ images, video: "asset://v.mp4", startIndex: 2, open: true, onClose: () => {} });
    const img = document.querySelector("img.lightbox-image");
    expect(img?.getAttribute("src") ?? "").toContain("b.jpg");
    dispose();
  });

  it("can still reach the last screenshot", () => {
    // Four entries with a video; the clamp used to cut this one off.
    const { dispose } = mount({ images, video: "asset://v.mp4", startIndex: 3, open: true, onClose: () => {} });
    expect(document.querySelector("img.lightbox-image")?.getAttribute("src") ?? "").toContain("c.jpg");
    dispose();
  });

  it("behaves like before when there is no video", () => {
    const { dispose } = mount({ images, video: null, startIndex: 1, open: true, onClose: () => {} });
    expect(document.querySelector("img.lightbox-image")?.getAttribute("src") ?? "").toContain("b.jpg");
    dispose();
  });
});
