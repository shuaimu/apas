import { describe, expect, it } from "vitest";
import {
  compareReleaseVersions,
  daemonVersionLabel,
  isMachineBehind,
  latestSeenVersion,
  parseReleaseVersion,
  rebootLabelFor,
} from "./daemonVersion";

const latest = (...candidates: Array<string | null | undefined>) =>
  latestSeenVersion(candidates);

describe("parsing release versions", () => {
  it("reads YY.MM.COMMIT", () => {
    expect(parseReleaseVersion("26.08.74")).toEqual({
      year: 26,
      month: 8,
      build: 74,
    });
  });

  it("refuses anything that is not three numeric components", () => {
    for (const bad of [
      "",
      "26.08",
      "26.08.74.1",
      "v26.08.74",
      "26.08.x",
      "26.08.7e1",
      "nightly",
      null,
      undefined,
    ]) {
      expect(parseReleaseVersion(bad), String(bad)).toBeNull();
    }
  });
});

describe("ordering", () => {
  it("is by release, not by text", () => {
    // The case a string comparison gets wrong: "26.08.9" > "26.08.10".
    const nine = parseReleaseVersion("26.08.9")!;
    const ten = parseReleaseVersion("26.08.10")!;
    expect(compareReleaseVersions(nine, ten)).toBeLessThan(0);
    expect(isMachineBehind("26.08.9", ten)).toBe(true);
    expect(isMachineBehind("26.08.10", nine)).toBe(false);
  });

  it("orders across months and years", () => {
    expect(isMachineBehind("26.08.99", latest("26.09.1"))).toBe(true);
    expect(isMachineBehind("25.12.99", latest("26.01.1"))).toBe(true);
  });
});

describe("what counts as behind", () => {
  it("a machine older than a peer is behind", () => {
    const seen = latest("26.08.70", "26.08.74");
    expect(isMachineBehind("26.08.70", seen)).toBe(true);
  });

  it("a machine older than the server is behind, even with no newer peer", () => {
    // Every machine level with each other, all of them behind the deployment.
    const seen = latest("26.09.3", "26.08.74", "26.08.74");
    expect(isMachineBehind("26.08.74", seen)).toBe(true);
  });

  it("the newest machine is not behind", () => {
    const seen = latest("26.08.70", "26.08.74");
    expect(isMachineBehind("26.08.74", seen)).toBe(false);
  });

  it("a machine newer than the server is not behind", () => {
    // The normal state here: hosts install the CLI from source, so they run
    // ahead of the deployed server rather than behind it.
    const seen = latest("26.08.60", "26.08.74");
    expect(isMachineBehind("26.08.74", seen)).toBe(false);
  });

  it("an unknown version is never behind", () => {
    const seen = latest("26.08.74");
    expect(isMachineBehind(undefined, seen)).toBe(false);
    expect(isMachineBehind(null, seen)).toBe(false);
    expect(isMachineBehind("", seen)).toBe(false);
    expect(isMachineBehind("nightly", seen)).toBe(false);
  });

  it("nothing is behind when no version can be read at all", () => {
    expect(isMachineBehind("26.08.70", latest("bogus", undefined))).toBe(false);
  });
});

describe("an unreadable version cannot distort the maximum", () => {
  it("is excluded rather than parsed leniently", () => {
    // A garbled report must not be able to mark an entire fleet as behind.
    const seen = latest("26.08.74", "99.99.banana", "26.08.74");
    expect(seen).toEqual({ year: 26, month: 8, build: 74 });
    expect(isMachineBehind("26.08.74", seen)).toBe(false);
  });

  it("leaves the real maximum intact", () => {
    expect(latest("nope", "26.08.70", "26.08.74")).toEqual({
      year: 26,
      month: 8,
      build: 74,
    });
  });

  it("does not depend on the order it sees them", () => {
    const forward = latest("26.08.70", "26.08.74", "26.09.1");
    const backward = latest("26.09.1", "26.08.74", "26.08.70");
    expect(forward).toEqual(backward);
  });
});

describe("wording", () => {
  it("says a restart updates only when the machine is known to be behind", () => {
    expect(rebootLabelFor(true)).toBe("Reboot to update");
    expect(rebootLabelFor(false)).toBe("Reboot");
  });

  it("never claims no update is available", () => {
    // A restart tries to update regardless, so the plain label must not promise
    // the machine comes back unchanged.
    expect(rebootLabelFor(false).toLowerCase()).not.toContain("no update");
    expect(rebootLabelFor(false).toLowerCase()).not.toContain("current");
  });

  it("says so plainly when a machine reports no version", () => {
    expect(daemonVersionLabel("26.08.74")).toBe("26.08.74");
    expect(daemonVersionLabel("  26.08.74  ")).toBe("26.08.74");
    expect(daemonVersionLabel(undefined)).toBe("version unknown");
    expect(daemonVersionLabel("")).toBe("version unknown");
  });
});
