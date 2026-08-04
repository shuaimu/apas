import { describe, expect, it } from "vitest";
import { redirectParam, safeRedirect } from "./safeRedirect";

describe("redirectParam", () => {
  it("encodes a target that contains its own query string", () => {
    // The original bug: `/login?redirect=/share?code=ABC` parses as
    // `redirect=/share` plus a stray top-level `code=ABC`, so the invite code
    // was dropped and the recipient landed on an empty form.
    const qs = redirectParam("/share?code=ABC123");
    expect(qs).toBe("redirect=%2Fshare%3Fcode%3DABC123");

    // Round-trip through the parser the browser actually uses.
    const parsed = new URLSearchParams(qs);
    expect(parsed.get("redirect")).toBe("/share?code=ABC123");
    expect(parsed.get("code")).toBeNull();
  });

  it("survives a full URL round-trip with the code intact", () => {
    const url = new URL(`http://x.test/login?${redirectParam("/share?code=ABC123")}`);
    const target = safeRedirect(url.searchParams.get("redirect"));
    expect(target).toBe("/share?code=ABC123");
    expect(new URL(target!, "http://x.test").searchParams.get("code")).toBe("ABC123");
  });
});

describe("safeRedirect", () => {
  it("accepts same-origin relative paths", () => {
    expect(safeRedirect("/share?code=ABC")).toBe("/share?code=ABC");
    expect(safeRedirect("/machines")).toBe("/machines");
    expect(safeRedirect("/")).toBe("/");
  });

  it.each([
    ["https://evil.example", "absolute, different origin"],
    ["http://evil.example/x", "absolute, different origin"],
    ["//evil.example", "protocol-relative — absolute despite the leading slash"],
    ["//evil.example/path", "protocol-relative"],
    ["/\\evil.example", "backslash folds to / in some browsers"],
    ["/\\/evil.example", "backslash folds to /"],
    ["machines", "relative to the current path, not the origin"],
    ["", "empty"],
  ])("rejects %j (%s)", (input) => {
    expect(safeRedirect(input)).toBeNull();
  });

  it("rejects nullish input", () => {
    expect(safeRedirect(null)).toBeNull();
    expect(safeRedirect(undefined)).toBeNull();
  });

  it("rejects a target that only becomes protocol-relative once decoded", () => {
    // `%2F%2Fevil.example` is `//evil.example` by the time a browser navigates,
    // so checking only the raw form would let it through.
    expect(safeRedirect("/%2F%2Fevil.example")).toBeNull();
    expect(safeRedirect("%2F%2Fevil.example")).toBeNull();
  });

  it("rejects malformed percent-encoding rather than guessing", () => {
    // We cannot know what this becomes, so it does not get the benefit of the
    // doubt.
    expect(safeRedirect("/%E0%A4%A")).toBeNull();
  });

  it("rejects control characters embedded in the path", () => {
    // Header-injection shaped input: the newline sits in the middle, so it
    // cannot be trimmed away and must be refused outright.
    expect(safeRedirect("/share\nHost: evil.example")).toBeNull();
    expect(safeRedirect("/sh\u0000are")).toBeNull();
  });

  it("trims trailing control characters rather than rejecting the path", () => {
    // trim() runs first and the *trimmed* value is what gets returned, so a
    // stray CRLF is stripped and nothing downstream ever sees it. Safe, and
    // worth pinning: an earlier draft of this test assumed a rejection.
    expect(safeRedirect("/share\r\n")).toBe("/share");
    expect(safeRedirect("/share  ")).toBe("/share");
  });

  it("does not treat a leading-slash lookalike as safe after trimming", () => {
    // Leading whitespace is trimmed first, so ` //evil` must still be caught.
    expect(safeRedirect("  //evil.example")).toBeNull();
    expect(safeRedirect("  /machines")).toBe("/machines");
  });
});
