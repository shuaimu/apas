"use client";

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import {
  overviewButton,
  overviewDangerButton,
  overviewPrimaryButton,
} from "@/components/overview/OverviewElements";

export interface ConfirmRequest {
  /** The dialog's accessible name, when it should differ from the title. */
  ariaLabel?: string;
  title: string;
  body: ReactNode;
  confirmLabel: string;
  tone: "primary" | "danger";
  onConfirm: () => void;
}

/**
 * One confirmation dialog for the whole page. Render `confirmDialog` once,
 * after everything that can open it, so the opener comes first in the
 * document.
 *
 * The panel is the Overview's card, opaque so the page does not show through
 * it in the dark theme.
 *
 * Focus starts on Cancel, stays inside the dialog, and goes back to the
 * opener when it closes. Escape and a press on the backdrop cancel. Confirm
 * closes first and then runs the action, so whatever the action shows next is
 * not hidden behind the dialog.
 */
export function useConfirmDialog(): { requestConfirm: (request: ConfirmRequest) => void; confirmDialog: ReactNode } {
  const [request, setRequest] = useState<ConfirmRequest | null>(null);
  const opener = useRef<HTMLElement | null>(null);
  const panel = useRef<HTMLDivElement | null>(null);
  const cancelButton = useRef<HTMLButtonElement | null>(null);

  const requestConfirm = useCallback((next: ConfirmRequest) => {
    opener.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setRequest(next);
  }, []);

  const close = useCallback(() => {
    setRequest(null);
    const target = opener.current;
    opener.current = null;
    target?.focus?.();
  }, []);

  useEffect(() => {
    if (request) cancelButton.current?.focus();
  }, [request]);

  const confirmDialog = request ? (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onMouseDown={(event) => { if (event.target === event.currentTarget) close(); }}
    >
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-label={request.ariaLabel ?? request.title}
        // Focusable, so a click on the dialog's text keeps focus inside it
        // and Escape still reaches this handler.
        tabIndex={-1}
        className="w-full max-w-md rounded-lg border border-gray-200 bg-white p-4 shadow-xl outline-none dark:border-gray-700 dark:bg-gray-800"
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            close();
            return;
          }
          if (event.key !== "Tab") return;
          const buttons = Array.from(panel.current?.querySelectorAll<HTMLButtonElement>("button:not([disabled])") ?? []);
          if (buttons.length === 0) return;
          const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
          const next = event.shiftKey
            ? (index <= 0 ? buttons.length - 1 : index - 1)
            : (index === -1 || index === buttons.length - 1 ? 0 : index + 1);
          event.preventDefault();
          buttons[next].focus();
        }}
      >
        <h2 className="text-sm font-semibold text-gray-900 dark:text-gray-100">{request.title}</h2>
        <div className="mt-2 space-y-2 text-sm text-gray-600 dark:text-gray-300">{request.body}</div>
        <div className="mt-4 flex justify-end gap-2">
          <button ref={cancelButton} type="button" className={overviewButton} onClick={close}>Cancel</button>
          <button
            type="button"
            className={request.tone === "danger" ? overviewDangerButton : overviewPrimaryButton}
            onClick={() => {
              const action = request.onConfirm;
              close();
              action();
            }}
          >
            {request.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  ) : null;

  return { requestConfirm, confirmDialog };
}
