import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const html = readFileSync(resolve("dist/terminal.html"), "utf8");

describe("local terminal bundle", () => {
  it("ships one self-contained document with a restrictive CSP", () => {
    expect(html).toContain("default-src 'none'");
    expect(html).toContain("connect-src 'none'");
    expect(html).toContain("frame-src 'none'");
    expect(html).not.toMatch(/<script[^>]+src=/i);
    expect(html).not.toMatch(/<link[^>]+href=/i);
  });

  it("contains the narrow native bridge and no credential names", () => {
    expect(html).toContain("ReactNativeWebView");
    expect(html).toContain("__APAS_TERMINAL_RECEIVE__");
    expect(html).not.toMatch(/access_token|refresh_token|authorization:\s*bearer/i);
  });
});
