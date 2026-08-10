import { describe, expect, it } from "vitest";
import { splitUnifiedDiff } from "./diff.js";

describe("splitUnifiedDiff", () => {
  it("groups a unified diff by destination file", () => {
    const result = splitUnifiedDiff("diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n+one\ndiff --git a/b.rs b/b.rs\n+two");
    expect(result.files.map((file) => file.path)).toEqual(["a.ts", "b.rs"]);
    expect(result.error).toBeNull();
  });

  it("reports malformed and truncated content", () => {
    expect(splitUnifiedDiff(null).error).toBeTruthy();
    expect(splitUnifiedDiff("diff --git a/a b/a\n+very long", 10).truncated).toBe(true);
  });
});
