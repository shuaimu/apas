import { CanvasAddon } from "@xterm/addon-canvas";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";

import {
  parseInboundBridgeMessage,
  type TerminalBridgeOutbound,
} from "./protocol";

declare global {
  interface Window {
    ReactNativeWebView?: { postMessage: (value: string) => void };
    __APAS_TERMINAL_RECEIVE__?: (value: unknown) => void;
  }
}

const host = document.getElementById("terminal");
if (!host) throw new Error("Terminal host missing");

const post = (message: TerminalBridgeOutbound) =>
  window.ReactNativeWebView?.postMessage(JSON.stringify(message));
const terminal = new Terminal({
  allowProposedApi: false,
  convertEol: false,
  cursorBlink: true,
  disableStdin: false,
  fontFamily: "ui-monospace, SFMono-Regular, Menlo, Consolas, monospace",
  fontSize: 13,
  scrollback: 5000,
  theme: { background: "#08080b", foreground: "#f2f2f4", cursor: "#8b80ff" },
});
const fit = new FitAddon();
terminal.loadAddon(fit);
terminal.loadAddon(new CanvasAddon());
terminal.loadAddon(
  new WebLinksAddon((_event, url) => post({ type: "link_request", url })),
);
terminal.open(host);

const decode = (value: string): Uint8Array => {
  const binary = atob(value);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
};

let renderQueue: Array<() => void> = [];
let writing = false;
const next = () => {
  writing = false;
  const operation = renderQueue.shift();
  if (operation) {
    writing = true;
    operation();
  }
};
const enqueue = (operation: () => void) => {
  renderQueue.push(operation);
  if (!writing) next();
};
const write = (bytes: Uint8Array) => enqueue(() => terminal.write(bytes, next));
const reset = () => {
  queuedOutput = [];
  renderQueue = [];
  enqueue(() => {
    terminal.reset();
    next();
  });
};
let queuedOutput: Uint8Array[] = [];
let outputFrame = 0;
const flushOutput = () => {
  outputFrame = 0;
  const frames = queuedOutput;
  queuedOutput = [];
  for (const frame of frames) write(frame);
};

window.__APAS_TERMINAL_RECEIVE__ = (value) => {
  const message = parseInboundBridgeMessage(value);
  if (!message) return;
  switch (message.type) {
    case "reset":
      reset();
      break;
    case "snapshot":
      reset();
      if (message.cols && message.rows)
        enqueue(() => {
          terminal.resize(message.cols!, message.rows!);
          next();
        });
      write(decode(message.dataBase64));
      break;
    case "output":
      queuedOutput.push(decode(message.dataBase64));
      if (!outputFrame) outputFrame = requestAnimationFrame(flushOutput);
      break;
    case "lifecycle":
      terminal.options.disableStdin = message.lifecycle !== "running";
      break;
    case "theme":
      terminal.options.theme = message.theme;
      break;
    case "focus":
      terminal.focus();
      break;
    case "fit": {
      const size = fit.proposeDimensions();
      if (size) {
        const cols = Math.min(300, Math.max(2, size.cols)),
          rows = Math.min(120, Math.max(1, size.rows));
        enqueue(() => {
          terminal.resize(cols, rows);
          next();
        });
        post({ type: "resize", cols, rows });
      }
      break;
    }
    case "paste":
      terminal.paste(message.text);
      break;
  }
};

terminal.onData((data) => post({ type: "input", data }));
terminal.attachCustomKeyEventHandler((event) => {
  if (
    (event.metaKey || event.ctrlKey) &&
    event.key.toLowerCase() === "v" &&
    event.type === "keydown"
  ) {
    post({ type: "paste_request" });
    return false;
  }
  return true;
});

post({ type: "ready" });
