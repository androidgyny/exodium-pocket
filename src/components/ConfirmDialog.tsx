import { createSignal, Show } from "solid-js";
import { Portal } from "solid-js/web";
import { Dialog } from "@ark-ui/solid/dialog";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  message: string;
  confirmLabel: string;
  /** Styles the confirm button red for destructive actions. */
  danger?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}

/** Must match the CSS animation duration on .closing. */
const EXIT_MS = 180;

/** Small confirm modal sharing the ark-dialog look and the delayed-unmount
 *  exit animation used by PlaylistNameDialog. */
export function ConfirmDialog(props: ConfirmDialogProps) {
  const [closing, setClosing] = createSignal(false);

  const close = (confirmed: boolean) => {
    if (closing()) { return; }
    setClosing(true);
    setTimeout(() => {
      setClosing(false);
      if (confirmed) { props.onConfirm(); }
      props.onClose();
    }, EXIT_MS);
  };

  return (
    <Show when={props.open}>
      <Dialog.Root open onOpenChange={(e) => { if (!e.open) { close(false); } }}>
        <Portal>
          <Dialog.Backdrop class={`ark-dialog-backdrop${closing() ? " closing" : ""}`} />
          <Dialog.Positioner class="ark-dialog-positioner">
            <Dialog.Content class={`ark-dialog-content playlist-dialog${closing() ? " closing" : ""}`}>
              <Dialog.Title class="ark-dialog-title">{props.title}</Dialog.Title>
              <Dialog.Description class="ark-dialog-desc">{props.message}</Dialog.Description>
              <div class="ark-dialog-actions">
                <button class="btn-secondary" onClick={() => close(false)}>Cancel</button>
                <button
                  class={props.danger ? "btn-danger" : "btn-primary"}
                  onClick={() => close(true)}
                >
                  {props.confirmLabel}
                </button>
              </div>
            </Dialog.Content>
          </Dialog.Positioner>
        </Portal>
      </Dialog.Root>
    </Show>
  );
}
