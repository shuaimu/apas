import { readFile, writeFile } from "node:fs/promises";
import { compile } from "json-schema-to-typescript";

const schemaPath = new URL("../schema/mobile-protocol.schema.json", import.meta.url);
const outputPath = new URL("../src/generated.ts", import.meta.url);
const schema = JSON.parse(await readFile(schemaPath, "utf8"));
const generated = await compile(schema, "MobileProtocolContract", {
  bannerComment: "/* Generated from Rust JSON Schema. Do not edit by hand. */",
  format: true,
  style: { singleQuote: false, semi: true, tabWidth: 2, trailingComma: "all" },
});
await writeFile(outputPath, generated);
