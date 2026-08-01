import { createSignal } from "solid-js";
import { getTransferStats, type TransferStats } from "../api/tauri";
import { isOffline } from "./network";

const IDLE_MS = 4000;
/** While bytes are moving the badge is a live readout, so it updates faster. */
const ACTIVE_MS = 1500;

const [stats, setStats] = createSignal<TransferStats | null>(null);
export { stats as transferStats };

let timer: ReturnType<typeof setTimeout> | null = null;
let running = false;

function schedule(delay: number) {
  if (!running) { return; }
  timer = setTimeout(poll, delay);
}

async function poll() {
  if (!running) { return; }
  // Offline drops every manager, so the command would only ever answer zeroes.
  // Keep polling (cheaply) rather than stopping: the user can switch back.
  if (isOffline()) {
    setStats(null);
    schedule(IDLE_MS);
    return;
  }
  try {
    const next = await getTransferStats();
    setStats(next);
    const moving = next.download_bps > 0 || next.upload_bps > 0;
    schedule(moving ? ACTIVE_MS : IDLE_MS);
  } catch (e) {
    // A missing manager is normal right after a mode switch; don't spam.
    console.debug("[transfer] stats unavailable:", e);
    setStats(null);
    schedule(IDLE_MS);
  }
}

/** Start the shared poll loop. Idempotent - the badge and settings panel both
 *  read the same signal rather than each polling on their own. */
export function startTransferPolling() {
  if (running) { return; }
  running = true;
  poll();
}

export function stopTransferPolling() {
  running = false;
  if (timer) { clearTimeout(timer); timer = null; }
  setStats(null);
}

/** Bytes/s as a short label. Below 1 KB/s reads as idle: BitTorrent keep-alive
 *  traffic never quite reaches zero and a flickering "312 B/s" is noise. */
export function formatRate(bps: number): string {
  if (bps < 1024) { return "0 KB/s"; }
  if (bps < 1024 * 1024) { return `${Math.round(bps / 1024)} KB/s`; }
  return `${(bps / (1024 * 1024)).toFixed(1)} MB/s`;
}
