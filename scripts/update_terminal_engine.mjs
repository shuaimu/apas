// Regenerate only after reviewing and pinning both xterm packages. Cargo uses
// the checked-in sources; installing or running the CLI does not require Node.
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
const root = fileURLToPath(new URL("../", import.meta.url));
if (
  JSON.parse(
    readFileSync(
      `${root}packages/web/node_modules/@xterm/xterm/package.json`,
      "utf8",
    ),
  ).version !== "6.0.0"
) {
  throw Error("Expected browser xterm@6.0.0 to match the embedded engine");
}
for (const [name, version, source, target] of [
  ["headless", "6.0.0", "lib-headless/xterm-headless.js", "xterm.js"],
  ["addon-serialize", "0.14.0", "lib/addon-serialize.js", "serialize.js"],
]) {
  const path = `${root}packages/web/node_modules/@xterm/${name}`;
  if (
    JSON.parse(readFileSync(`${path}/package.json`, "utf8")).version !== version
  )
    throw Error(`Expected ${name}@${version}`);
  const bytes = readFileSync(`${path}/${source}`, "utf8").replace(
    /\/\/# sourceMappingURL=.*$/m,
    "",
  );
  writeFileSync(
    `${root}crates/client-cli/src/terminal_engine/${target}`,
    bytes,
  );
  writeFileSync(
    `${root}crates/client-cli/src/terminal_engine/${name}.LICENSE`,
    readFileSync(`${root}packages/web/node_modules/@xterm/xterm/LICENSE`),
  );
}
