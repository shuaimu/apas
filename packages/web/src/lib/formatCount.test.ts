import { describe, expect, it } from "vitest";
import { formatCompactCount, formatUsd } from "./formatCount";

describe("formatCompactCount", () => {
  it("leaves small counts as they are", () => {
    expect(formatCompactCount(0)).toBe("0");
    expect(formatCompactCount(316)).toBe("316");
    expect(formatCompactCount(999)).toBe("999");
  });

  it("picks the unit that keeps the figure under a thousand", () => {
    expect(formatCompactCount(1_234)).toBe("1.23k");
    expect(formatCompactCount(44_800)).toBe("44.8k");
    expect(formatCompactCount(508_600)).toBe("508.6k");
    expect(formatCompactCount(5_500_000)).toBe("5.50M");
    expect(formatCompactCount(512_200_000)).toBe("512.2M");
    // The totals that used to read "25666.1M" and "557002.0M".
    expect(formatCompactCount(25_666_100_000)).toBe("25.7B");
    expect(formatCompactCount(557_002_000_000)).toBe("557.0B");
    expect(formatCompactCount(2_500_000_000_000)).toBe("2.50T");
  });

  it("drops the decimals of an exact multiple", () => {
    expect(formatCompactCount(1_000)).toBe("1k");
    expect(formatCompactCount(15_000)).toBe("15k");
    expect(formatCompactCount(3_000_000)).toBe("3M");
  });

  it("rounds before it settles the unit and the precision", () => {
    expect(formatCompactCount(9_996)).toBe("10.0k");
    expect(formatCompactCount(999_949)).toBe("999.9k");
    expect(formatCompactCount(999_960)).toBe("1.00M");
    expect(formatCompactCount(999_999_999)).toBe("1.00B");
  });

  it("never runs out of units", () => {
    expect(formatCompactCount(1_234_500_000_000_000)).toBe("1234.5T");
    expect(formatCompactCount(5_000_000_000_000_000)).toBe("5000T");
  });

  it("answers something printable for a value that is not a count", () => {
    expect(formatCompactCount(Number.NaN)).toBe("0");
    expect(formatCompactCount(-1_500)).toBe("-1.50k");
  });
});

describe("formatUsd", () => {
  it("shows cents and separates thousands", () => {
    expect(formatUsd(2.5)).toBe("$2.50");
    expect(formatUsd(97.154)).toBe("$97.15");
    expect(formatUsd(3009.01)).toBe("$3,009.01");
    expect(formatUsd(0.8833)).toBe("$0.88");
  });

  it("does not round a small cost down to nothing", () => {
    expect(formatUsd(0.004)).toBe("<$0.01");
  });

  it("shows a dash where no cost was reported", () => {
    expect(formatUsd(0)).toBe("—");
    expect(formatUsd(Number.NaN)).toBe("—");
  });
});
