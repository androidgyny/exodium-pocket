import { createSignal, onMount, onCleanup } from "solid-js";
import { getCurrentWindow } from "@tauri-apps/api/window";

const win = getCurrentWindow();

export function WindowFrame() {
  const [maximized, setMaximized] = createSignal(false);

  onMount(async () => {
    setMaximized(await win.isMaximized());
    const unlisten = await win.onResized(async () => {
      setMaximized(await win.isMaximized());
    });
    onCleanup(unlisten);
  });

  return (
    <div class="window-frame" data-tauri-drag-region>
      <span class="window-frame-title" data-tauri-drag-region>Exodium</span>
      <div class="window-frame-controls">
        <button
          class="window-frame-btn"
          onClick={() => win.minimize()}
          aria-label="Minimize"
          tabIndex={-1}
        >
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 5h10" stroke="currentColor" stroke-width="1" /></svg>
        </button>
        <button
          class="window-frame-btn"
          onClick={() => win.toggleMaximize()}
          aria-label={maximized() ? "Restore" : "Maximize"}
          tabIndex={-1}
        >
          {maximized() ? (
            <svg width="10" height="10" viewBox="0 0 10 10">
              <path d="M2 0h8v8h-2M0 2h8v8h-8z" fill="none" stroke="currentColor" stroke-width="1" />
            </svg>
          ) : (
            <svg width="10" height="10" viewBox="0 0 10 10">
              <path d="M0 0h10v10h-10z" fill="none" stroke="currentColor" stroke-width="1" />
            </svg>
          )}
        </button>
        <button
          class="window-frame-btn close"
          onClick={() => win.close()}
          aria-label="Close"
          tabIndex={-1}
        >
          <svg width="10" height="10" viewBox="0 0 10 10"><path d="M0 0l10 10M10 0l-10 10" stroke="currentColor" stroke-width="1" /></svg>
        </button>
      </div>
    </div>
  );
}
