import { useEffect, useId, useRef, type ReactNode } from 'react';

interface ModalDialogProps {
  open: boolean;
  title: string;
  onClose: () => void;
  preventClose?: boolean;
  children: ReactNode;
  footer?: ReactNode;
}

/**
 * Native <dialog closedby="any"|"none"> modal following modern-web-guidance:
 * - Opened via showModal() for top-layer, inert backdrop, Esc & focus return
 * - No redundant role="dialog" / aria-modal
 * - Pointer-coordinate light-dismiss fallback for browsers without closedBy (Safari)
 * - Resets dialog.returnValue on close
 */
export function ModalDialog({
  open,
  title,
  onClose,
  preventClose = false,
  children,
  footer,
}: ModalDialogProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;

    const handleClose = () => {
      dialog.returnValue = '';
      onClose();
    };
    const handleCancel = (e: Event) => {
      if (preventClose) {
        e.preventDefault();
      }
    };
    dialog.addEventListener('close', handleClose);
    dialog.addEventListener('cancel', handleCancel);

    // Fallback light-dismiss when native `closedby="any"` is unsupported (Safari)
    let cleanupFallback = () => {};
    if (!('closedBy' in HTMLDialogElement.prototype)) {
      const outside = (e: PointerEvent) => {
        const r = dialog.getBoundingClientRect();
        return e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom;
      };
      let pressedOutside = false;
      const onDown = (e: PointerEvent) => {
        pressedOutside = outside(e);
      };
      const onUp = (e: PointerEvent) => {
        if (!preventClose && pressedOutside && outside(e)) {
          dialog.close();
        }
      };
      dialog.addEventListener('pointerdown', onDown);
      dialog.addEventListener('pointerup', onUp);
      cleanupFallback = () => {
        dialog.removeEventListener('pointerdown', onDown);
        dialog.removeEventListener('pointerup', onUp);
      };
    }

    return () => {
      dialog.removeEventListener('close', handleClose);
      dialog.removeEventListener('cancel', handleCancel);
      cleanupFallback();
    };
  }, [onClose, preventClose]);

  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      {...({ closedby: preventClose ? 'none' : 'any' } as Record<string, string>)}
    >
      <div className="dialog-head">
        <h2 id={titleId} className="dialog-title">
          {title}
        </h2>
        {!preventClose ? (
          <button type="button" className="btn btn-ghost" onClick={onClose} aria-label="Close dialog">
            Close
          </button>
        ) : null}
      </div>
      <div className="dialog-body">{children}</div>
      {footer ? <div className="dialog-foot">{footer}</div> : null}
    </dialog>
  );
}
