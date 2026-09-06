import type {
  ClipboardSelectionType,
  IClipboardProvider,
} from "@xterm/addon-clipboard";

// ClipboardSelectionType is a declaration-only const enum in addon 0.2.0, so
// importing it as a runtime value produces undefined in Vite's test transform.
export const SYSTEM_CLIPBOARD_SELECTION = "c" as ClipboardSelectionType;
export const PRIMARY_CLIPBOARD_SELECTION = "p" as ClipboardSelectionType;

type TerminalClipboardKeyEvent = Pick<
  KeyboardEvent,
  "altKey" | "ctrlKey" | "key" | "metaKey" | "type"
>;

/**
 * Keep browser clipboard shortcuts out of the hosted TUI.
 *
 * xterm normally turns Ctrl+V into byte 0x16 and prevents the browser paste
 * event. Copy is different: Ctrl+C must remain SIGINT unless xterm owns a
 * selection. Claude's fullscreen selection is not an xterm selection and
 * reaches the clipboard through OSC 52 instead.
 */
export function handleTerminalClipboardKey(
  event: TerminalClipboardKeyEvent,
  hasSelection: boolean,
): boolean {
  if (event.type !== "keydown" || event.altKey || (!event.ctrlKey && !event.metaKey)) {
    return true;
  }

  const key = event.key.toLowerCase();
  if (key === "v") return false;
  if (key === "c" && hasSelection) return false;
  return true;
}

function copyWithHiddenTextarea(text: string): void {
  if (typeof document === "undefined" || typeof document.execCommand !== "function") return;

  const active = document.activeElement;
  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.readOnly = true;
  textarea.style.position = "fixed";
  textarea.style.left = "-9999px";
  document.body.appendChild(textarea);
  textarea.select();
  try {
    document.execCommand("copy");
  } catch {
    // The browser can reject clipboard writes after transient activation ends.
  } finally {
    textarea.remove();
    if (active instanceof HTMLElement) active.focus({ preventScroll: true });
  }
}

/**
 * OSC 52 provider for hosted TUIs.
 *
 * Terminal processes may write the browser clipboard. Reads always return
 * empty so an agent cannot inspect clipboard contents. The textarea path
 * covers browsers that lack the async clipboard API or reject a write.
 */
export class WriteOnlyTerminalClipboardProvider implements IClipboardProvider {
  readText(_selection: ClipboardSelectionType): string {
    return "";
  }

  async writeText(selection: ClipboardSelectionType, text: string): Promise<void> {
    if (selection !== SYSTEM_CLIPBOARD_SELECTION) return;

    try {
      if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
        await navigator.clipboard.writeText(text);
        return;
      }
    } catch {
      // Fall through to the selection-based browser API.
    }

    copyWithHiddenTextarea(text);
  }
}
