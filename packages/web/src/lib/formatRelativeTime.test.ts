import { describe, expect, it } from "vitest";
import { formatRelativeTime } from "./formatRelativeTime";

const NOW = Date.parse("2026-11-13T12:00:00Z");
const before = (ms: number) => NOW - ms;

describe("formatRelativeTime", () => {
  it("says just now for anything under a minute", () => {
    expect(formatRelativeTime(NOW, NOW)).toBe("just now");
    expect(formatRelativeTime(before(44_000), NOW)).toBe("just now");
    expect(formatRelativeTime(before(59_999), NOW)).toBe("just now");
  });

  it("never prints a negative time for a moment ahead of this clock", () => {
    expect(formatRelativeTime(NOW + 1_500, NOW)).toBe("just now");
    expect(formatRelativeTime(NOW + 3 * 3_600_000, NOW)).toBe("just now");
  });

  it("counts whole minutes, hours and days", () => {
    expect(formatRelativeTime(before(60_000), NOW)).toBe("1m ago");
    expect(formatRelativeTime(before(59 * 60_000 + 59_000), NOW)).toBe("59m ago");
    expect(formatRelativeTime(before(60 * 60_000), NOW)).toBe("1h ago");
    expect(formatRelativeTime(before(23 * 3_600_000), NOW)).toBe("23h ago");
    expect(formatRelativeTime(before(24 * 3_600_000), NOW)).toBe("1d ago");
    expect(formatRelativeTime(before(120 * 24 * 3_600_000), NOW)).toBe("120d ago");
  });

  it("takes a Date as well as a timestamp", () => {
    expect(formatRelativeTime(new Date(before(5 * 60_000)), NOW)).toBe("5m ago");
  });

  it("shows a dash for a date that is not one", () => {
    expect(formatRelativeTime(new Date("nonsense"), NOW)).toBe("—");
    expect(formatRelativeTime(Number.NaN, NOW)).toBe("—");
  });
});
