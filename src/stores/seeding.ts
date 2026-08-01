import { getConfig } from "../api/tauri";
import { isOffline } from "./network";

/** Whether this install still owes an answer about seeding.
 *
 *  Installs made before seeding became opt-in have no `seeding_enabled` key and
 *  used to upload anyway, so their wish is genuinely unknown - guessing either
 *  way is wrong, and `SeedingConsentDialog` asks instead. The backend reads
 *  "unset" as off, so nothing is uploaded while the question is open.
 *
 *  Offline installs are not asked: nothing uploads in that mode either way, so
 *  the question would be noise. It comes up when they first go online. */
export async function needsSeedingConsent(): Promise<boolean> {
  if (isOffline()) { return false; }
  try {
    return (await getConfig("seeding_enabled")) == null;
  } catch (e) {
    // Asking on a failed read would mean asking on every start; the safe state
    // (not seeding) already holds, so stay quiet.
    console.warn("[settings] could not read the seeding preference:", e);
    return false;
  }
}
