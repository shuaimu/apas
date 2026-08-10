import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
  validateClientMessage,
  validateCodeEvent,
  validateServerMessage,
} from "../src/validators.js";

const fixturesDirectory = fileURLToPath(new URL("../fixtures", import.meta.url));

describe("Rust protocol golden fixtures", () => {
  for (const name of readdirSync(fixturesDirectory).filter((value) => value.endsWith(".json"))) {
    it(`accepts ${name}`, () => {
      const value: unknown = JSON.parse(readFileSync(`${fixturesDirectory}/${name}`, "utf8"));
      const result = name.startsWith("web-")
        ? validateClientMessage(value)
        : name.startsWith("server-")
          ? validateServerMessage(value)
          : validateCodeEvent(value);
      expect(result.errors).toEqual([]);
      expect(result.valid).toBe(true);
    });
  }

  it("rejects an unknown client mutation", () => {
    expect(validateClientMessage({ type: "invented_mutation" }).valid).toBe(false);
  });
});
