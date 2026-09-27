export type TerminalControl =
  | { kind: "key"; data: string }
  | { kind: "paste"; text: string }
  | { kind: "focus" }
  | { kind: "page"; direction: -1 | 1 };
const controls = new Map<number, (control: TerminalControl) => boolean>();
export function registerTerminalControls(
  paneId: number,
  handler: (control: TerminalControl) => boolean,
): () => void {
  controls.set(paneId, handler);
  return () => {
    if (controls.get(paneId) === handler) controls.delete(paneId);
  };
}
export function controlTerminal(
  paneId: number,
  control: TerminalControl,
): boolean {
  const handler = controls.get(paneId);
  if (!handler) return false;
  return handler(control);
}
