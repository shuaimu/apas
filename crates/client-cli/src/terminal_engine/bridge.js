// xterm 6.0.0 + serialize 0.14.0. The few internal fields below fill gaps in
// the upstream serializer. Cross-engine fixtures gate changes to these pins.
let timerId = 0;
// xterm uses this single property to select its headless scheduling path.
// This object exposes no process, filesystem, module loader, or network APIs.
globalThis.process = { title: "apas-terminal" };
globalThis.navigator = {
  userAgent: "APAS terminal",
  platform: "Linux",
  language: "en-US",
};
globalThis.performance = { now: () => Date.now() };
const timers = new Map();
globalThis.setTimeout = (callback, delay = 0) => {
  const id = ++timerId;
  if (!delay) timers.set(id, callback);
  return id;
};
globalThis.clearTimeout = (id) => timers.delete(id);
globalThis.console = { log() {}, warn() {}, error() {}, debug() {}, info() {} };
function drain() {
  while (timers.size) {
    const [id, callback] = timers.entries().next().value;
    timers.delete(id);
    callback();
  }
}
let terminal, serializer;
const charsetCodes = new Map();
const charsetCode = (charset) => charsetCodes.get(charset) ?? "B";
globalThis.createTerminal = (cols, rows) => {
  terminal = new exports.Terminal({
    cols,
    rows,
    scrollback: 1000,
    allowProposedApi: true,
  });
  serializer = new module.exports.SerializeAddon();
  terminal.loadAddon(serializer);
  for (const code of [
    "B",
    "0",
    "A",
    "4",
    "C",
    "5",
    "R",
    "Q",
    "K",
    "Y",
    "E",
    "6",
    "Z",
    "H",
    "7",
    "=",
  ]) {
    terminal.write(`\x1b(${code}`);
    drain();
    charsetCodes.set(terminal._core._charsetService.charset, code);
  }
  terminal.reset();
};
globalThis.processOutput = (data) => {
  terminal.write(data);
  drain();
};
globalThis.resizeTerminal = (cols, rows) => {
  terminal.resize(cols, rows);
  drain();
};
const cup = (x, y) =>
  `\x1b[${Math.max(0, y) + 1};${Math.max(0, Math.min(terminal.cols - 1, x)) + 1}H`;
function sgr(attr) {
  const params = [0];
  for (const [method, code] of [
    ["isBold", 1],
    ["isDim", 2],
    ["isItalic", 3],
    ["isUnderline", 4],
    ["isBlink", 5],
    ["isInverse", 7],
    ["isInvisible", 8],
    ["isStrikethrough", 9],
    ["isOverline", 53],
  ]) {
    if (attr[method]()) params.push(code);
  }
  for (const [side, code] of [
    ["Fg", 38],
    ["Bg", 48],
  ]) {
    const color = attr[`get${side}Color`]();
    if (attr[`is${side}RGB`]())
      params.push(
        code,
        2,
        (color >> 16) & 255,
        (color >> 8) & 255,
        color & 255,
      );
    else if (attr[`is${side}Palette`]()) params.push(code, 5, color);
  }
  return `\x1b[${params.join(";")}m`;
}
function position(buffer, origin = false) {
  let result = cup(buffer.x, buffer.y - (origin ? buffer.scrollTop : 0));
  // CUP clamps at the right margin. Reprint the last cell to restore wrap pending.
  if (buffer.x >= terminal.cols) {
    const publicBuffer =
      buffer === terminal._core.buffers.normal
        ? terminal.buffer.normal
        : terminal.buffer.alternate;
    const line = publicBuffer.getLine(buffer.ybase + buffer.y);
    let column = terminal.cols - 1;
    if (line.getCell(column).getWidth() === 0) column--;
    const cell = line.getCell(column);
    result =
      cup(column, buffer.y - (origin ? buffer.scrollTop : 0)) +
      sgr(cell) +
      (cell.getChars() || " ");
  }
  return result;
}
function bufferState(buffer) {
  let result = `\x1b[?6l\x1b[${buffer.scrollTop + 1};${buffer.scrollBottom + 1}r\x1b[3g`;
  for (const [column, enabled] of Object.entries(buffer.tabs))
    if (enabled && +column < terminal.cols) result += cup(+column, 0) + "\x1bH";
  result +=
    `\x1b(${charsetCode(buffer.savedCharset)}\x0f` +
    cup(buffer.savedX, buffer.savedY - buffer.ybase) +
    sgr(buffer.savedCurAttrData) +
    "\x1b7\x1b(B\x0f";
  return (
    result + position(buffer) + sgr(terminal._core._inputHandler._curAttrData)
  );
}
globalThis.serializeTerminal = () => {
  const core = terminal._core;
  const normal = serializer.serialize({
    excludeAltBuffer: true,
    excludeModes: true,
  });
  let data = "\x1bc" + normal + bufferState(core.buffers.normal);
  if (terminal.buffer.active.type === "alternate") {
    const both = serializer.serialize({ excludeModes: true });
    // 47 switches buffers without overwriting the saved normal cursor we restored.
    data +=
      both.slice(normal.length).replace(/^\x1b\[\?1049h/, "\x1b[?47h") +
      bufferState(core.buffers.alt);
  }
  data += serializer._serializeModes(terminal);
  if (core.coreMouseService.activeEncoding === "SGR") data += "\x1b[?1006h";
  if (core.coreMouseService.activeEncoding === "SGR_PIXELS")
    data += "\x1b[?1016h";
  if (core.coreService.isCursorHidden) data += "\x1b[?25l";
  if (terminal.modes.originMode) data += position(core.buffers.active, true);
  data += sgr(core._inputHandler._curAttrData);
  const charsets = core._charsetService;
  for (let i = 0; i < 4; i++)
    data += `\x1b${"()*+"[i]}${charsetCode(charsets._charsets[i])}`;
  data += ["\x0f", "\x0e", "\x1bn", "\x1bo"][charsets.glevel];
  return data;
};
globalThis.inspectTerminal = () =>
  JSON.stringify({
    text: Array.from({ length: terminal.buffer.active.length }, (_, i) =>
      terminal.buffer.active.getLine(i).translateToString(true),
    ),
    x: terminal.buffer.active.cursorX,
    y: terminal.buffer.active.cursorY,
    modes: terminal.modes,
  });

globalThis.hasPendingSequence = () =>
  terminal._core._inputHandler._parser.currentState !== 0 ||
  terminal._core._inputHandler._utf8Decoder.interim.some((byte) => byte !== 0);
