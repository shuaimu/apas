import { describe, expect, it } from "vitest";
import { projectHue, projectInitials } from "./projectList";

describe("projectInitials", () => {
  it("takes the first letter of the first two words", () => {
    expect(projectInitials("my-project")).toBe("MP");
    expect(projectInitials("apas_web")).toBe("AW");
    expect(projectInitials("foo.bar.baz")).toBe("FB");
    expect(projectInitials("hello world")).toBe("HW");
  });

  it("takes the first two letters of a one-word name", () => {
    expect(projectInitials("apas")).toBe("AP");
    expect(projectInitials("x")).toBe("X");
  });

  it("handles non-ASCII names without dropping them", () => {
    expect(projectInitials("项目")).toBe("项目");
    expect(projectInitials("ünïcode-test")).toBe("ÜT");
  });

  it("falls back to a placeholder when the name has no letters", () => {
    expect(projectInitials("---")).toBe("?");
    expect(projectInitials("")).toBe("?");
  });
});

describe("projectHue", () => {
  it("is stable for the same id and within the hue circle", () => {
    const hue = projectHue("project-a");
    expect(hue).toBe(projectHue("project-a"));
    expect(hue).toBeGreaterThanOrEqual(0);
    expect(hue).toBeLessThan(360);
  });

  it("separates ids that differ only slightly", () => {
    expect(projectHue("project-a")).not.toBe(projectHue("project-b"));
  });
});
