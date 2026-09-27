import { CanvasAddon } from "@xterm/addon-canvas";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { Terminal } from "@xterm/xterm";

import { parseInboundBridgeMessage, type TerminalBridgeOutbound } from "./protocol";

declare global {
  interface Window {
    ReactNativeWebView?: { postMessage: (value: string) => void };
    __APAS_TERMINAL_RECEIVE__?: (value: unknown) => void;
  }
}

const host = document.getElementById("terminal");
if (!host) throw new Error("Terminal host missing");

const post = (message: TerminalBridgeOutbound) => window.ReactNativeWebView?.postMessage(JSON.stringify(message));
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
terminal.loadAddon(new WebLinksAddon((_event, url) => post({ type: "link_request", url })));
terminal.open(host);
fit.fit();

const decode = (value: string): Uint8Array => {
  const binary = atob(value);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
};

let queuedOutput: Uint8Array[] = [];
let outputFrame = 0;
const flushOutput = () => {
  outputFrame = 0;
  const frames = queuedOutput;
  queuedOutput = [];
  for (const frame of frames) terminal.write(frame);
};

window.__APAS_TERMINAL_RECEIVE__ = (value) => {
  const message = parseInboundBridgeMessage(value);
  if (!message) return;
  switch (message.type) {
    case "reset": terminal.reset(); break;
    case "snapshot":
      terminal.reset();
      terminal.write(decode(message.dataBase64));
      if (message.truncated) terminal.writeln("\r\n[Earlier scrollback truncated by server]");
      break;
    case "output":
      queuedOutput.push(decode(message.dataBase64));
      if (!outputFrame) outputFrame = requestAnimationFrame(flushOutput);
      break;
    case "lifecycle": terminal.options.disableStdin = message.lifecycle !== "running"; break;
    case "theme": terminal.options.theme = message.theme; break;
    case "focus": terminal.focus(); break;
    case "paste": terminal.paste(message.text); break;
  }
};

terminal.onData((data) => post({ type: "input", data }));
terminal.onResize(({ cols, rows }) => post({ type: "resize", cols, rows }));
terminal.attachCustomKeyEventHandler((event) => {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "v" && event.type === "keydown") {
    post({ type: "paste_request" });
    return false;
  }
  return true;
});

const resize = new ResizeObserver(() => {
  fit.fit();
  post({ type: "resize", cols: terminal.cols, rows: terminal.rows });
});
resize.observe(host);
post({ type: "ready" });
