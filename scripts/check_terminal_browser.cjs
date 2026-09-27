// Isolated browser smoke test. No APAS server or provider receives input.
// Set APAS_PLAYWRIGHT_MODULE / APAS_CHROMIUM_PATH if using an external install.
const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");
const root = path.resolve(__dirname, "..");
const web = path.join(root, "packages/web");
const { chromium } = require(
  process.env.APAS_PLAYWRIGHT_MODULE || "playwright",
);
const ts = require(path.join(web, "node_modules/typescript"));
(async () => {
  const browser = await chromium.launch({
    headless: true,
    executablePath: process.env.APAS_CHROMIUM_PATH,
    args: ["--no-sandbox"],
  });
  try {
    const context = await browser.newContext({
      viewport: { width: 390, height: 700 },
      isMobile: true,
      hasTouch: true,
    });
    const page = await context.newPage();
    await page.setContent(
      '<meta name="viewport" content="width=device-width,initial-scale=1"><div id="terminal" style="width:380px;height:500px"></div>',
    );
    await page.addStyleTag({
      path: path.join(web, "node_modules/@xterm/xterm/css/xterm.css"),
    });
    await page.addScriptTag({
      path: path.join(web, "node_modules/@xterm/xterm/lib/xterm.js"),
    });
    const touch = ts.transpileModule(
      fs.readFileSync(path.join(web, "src/lib/terminalTouch.ts"), "utf8"),
      {
        compilerOptions: {
          target: ts.ScriptTarget.ES2020,
          module: ts.ModuleKind.ESNext,
        },
      },
    ).outputText;
    await page.addScriptTag({
      type: "module",
      content: touch + "\nwindow.installTerminalTouch = installTerminalTouch;",
    });
    await page.evaluate(async () => {
      window.input = [];
      window.terminal = new window.Terminal({
        cols: 40,
        rows: 20,
        scrollback: 1000,
        allowProposedApi: true,
      });
      terminal.open(document.getElementById("terminal"));
      terminal.onData((data) => window.input.push(data));
      installTerminalTouch(terminal);
      await new Promise((resolve) =>
        terminal.write(
          Array.from({ length: 100 }, (_, i) => `line ${i}\r\n`).join(""),
          resolve,
        ),
      );
    });
    const before = await page.evaluate(() => terminal.buffer.active.viewportY);
    const client = await context.newCDPSession(page);
    const swipe = async () => {
      await client.send("Input.dispatchTouchEvent", {
        type: "touchStart",
        touchPoints: [{ x: 140, y: 110 }],
      });
      for (let y = 125; y <= 230; y += 15)
        await client.send("Input.dispatchTouchEvent", {
          type: "touchMove",
          touchPoints: [{ x: 140, y }],
        });
      await client.send("Input.dispatchTouchEvent", {
        type: "touchEnd",
        touchPoints: [],
      });
    };
    await swipe();
    const after = await page.evaluate(() => terminal.buffer.active.viewportY);
    assert(after < before, "touch swipe must scroll normal-buffer history");
    assert.equal(
      await page.evaluate(() => input.length),
      0,
      "local scrolling must not type into the provider",
    );
    const fixture = JSON.parse(
      fs.readFileSync(
        path.join(web, "src/lib/terminalCheckpoints.fixture.json"),
        "utf8",
      ),
    ).find((item) => item.name === "alternate-mouse");
    await page.evaluate(async (checkpoint) => {
      terminal.reset();
      terminal.resize(checkpoint.screen.cols, checkpoint.screen.rows);
      await new Promise((resolve) =>
        terminal.write(
          Uint8Array.from(atob(checkpoint.data_b64), (ch) => ch.charCodeAt(0)),
          resolve,
        ),
      );
      input.length = 0;
    }, fixture.checkpoint);
    await swipe();
    const mouse = await page.evaluate(() => input.slice());
    assert(
      mouse.some((data) => /^\x1b\[<64;/.test(data)),
      "restored application scrolling must emit SGR wheel-up events",
    );
    await page.evaluate(() => {
      input.length = 0;
      terminal.paste("hello\nworld");
    });
    const paste = await page.evaluate(() => input.join(""));
    assert(
      paste.startsWith("\x1b[200~") && paste.endsWith("\x1b[201~"),
      "restored bracketed paste mode must frame multiline text",
    );
    console.log(
      JSON.stringify({
        normalTouchScroll: { before, after },
        restoredMouseScroll: true,
        bracketedPaste: true,
      }),
    );
  } finally {
    await browser.close();
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
