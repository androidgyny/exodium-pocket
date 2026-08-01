import { Show } from "solid-js";
import { Portal } from "solid-js/web";
import { Tooltip } from "@ark-ui/solid/tooltip";
import { isOffline } from "../stores/network";
import { transferStats, formatRate } from "../stores/transfer";
import { seedingOn } from "../stores/seeding";
import { formatBytes } from "../util";

interface Props {
  /** Opens Settings - the badge is the shortcut to the setting it reports. */
  onOpenSettings: () => void;
}

const IconDown = () => (
  <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3">
    <path stroke-linecap="round" stroke-linejoin="round" d="M12 4v15m0 0l-6-6m6 6l6-6" />
  </svg>
);

const IconUp = () => (
  <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3">
    <path stroke-linecap="round" stroke-linejoin="round" d="M12 20V5m0 0l-6 6m6-6l6 6" />
  </svg>
);

/** Network state in the top bar: offline, or online with live transfer rates.
 *
 *  One component for both because they occupy the same slot and answer the same
 *  question - a separate "online" badge would have drifted from the offline one
 *  in placement and wording. */
export function NetworkBadge(props: Props) {
  const s = () => transferStats();
  const moving = () => {
    const v = s();
    return !!v && (v.download_bps >= 1024 || v.upload_bps >= 1024);
  };

  /** What "Online" actually means right now.
   *
   *  A rate of zero is ambiguous - sharing switched off and nobody requesting
   *  look identical - so the sharing state comes first, and the peer count
   *  carries the rest: connections are a standing state, transfer is
   *  event-driven, so peers can prove liveness when the rates cannot. */
  const peers = (n: number) => `${n} peer${n === 1 ? "" : "s"}`;

  const statusLine = () => {
    const v = s();
    if (!v?.active) { return "Online - no torrent running."; }
    const shared = `${formatBytes(v.uploaded_bytes)} shared this session.`;
    if (!seedingOn()) { return "Sharing is off - downloading only."; }
    if (moving()) { return `${peers(v.peers)} - ${shared}`; }
    if (v.peers === 0) { return "Online, looking for peers."; }
    return `Sharing with ${peers(v.peers)} - nothing requested right now. ${shared}`;
  };

  return (
    <Show
      when={!isOffline()}
      fallback={
        <Tooltip.Root openDelay={300}>
          <Tooltip.Trigger asChild={(triggerProps) =>
            <button {...triggerProps()} class="net-badge net-badge--offline" onClick={props.onOpenSettings}>
              <span class="net-badge-dot" /> Offline
            </button>
          } />
          <Portal><Tooltip.Positioner><Tooltip.Content class="ark-tooltip">
            Torrent client is off - no downloads or previews. Click to change.
          </Tooltip.Content></Tooltip.Positioner></Portal>
        </Tooltip.Root>
      }
    >
      <Tooltip.Root openDelay={300}>
        <Tooltip.Trigger asChild={(triggerProps) =>
          <button {...triggerProps()} class="net-badge net-badge--online" onClick={props.onOpenSettings}>
            <span class="net-badge-dot" classList={{ "is-active": moving() }} />
            <Show when={moving()} fallback={<>Online</>}>
              <span class="net-badge-rate"><IconDown />{formatRate(s()!.download_bps)}</span>
              <span class="net-badge-rate"><IconUp />{formatRate(s()!.upload_bps)}</span>
            </Show>
          </button>
        } />
        <Portal><Tooltip.Positioner><Tooltip.Content class="ark-tooltip">
          {statusLine()} Click for network settings.
        </Tooltip.Content></Tooltip.Positioner></Portal>
      </Tooltip.Root>
    </Show>
  );
}
