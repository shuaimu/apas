// Regression tests for the scroll-stranding bug after hard refresh.
//
// Symptom: user was at the bottom of a long chat, hard-refreshed, ended
// up scrolled to the middle even though no new messages arrived.
//
// Root cause: the auto-scroll effect in MessagePane used
//   messagesEndRef.scrollIntoView({ behavior: "smooth" })
// on every messages.length change. The cache restore on hard reload
// appends ~100 messages in one set; smooth-scroll then has to animate
// across thousands of pixels at ~300ms per ~1000px, taking several
// seconds. Any subsequent layout shift (catchup batch arriving, user
// touch, etc.) moves the bottom further down while the smooth scroll
// is still aimed at the OLD bottom — leaving the chat parked in the
// middle.
//
// The pure decision function `decideAutoScrollMode` picks "instant"
// (snap to scrollHeight after layout) for bulk additions and "smooth"
// for incremental ones, so the bug can't recur.
import { describe, it, expect } from "vitest";
import { decideAutoScrollMode } from "./TabbedView";

describe("decideAutoScrollMode", () => {
  const base = {
    isActive: true,
    shouldAutoScroll: true,
    isRestoringScroll: false,
    prevCount: 5,
    newCount: 6,
  };

  it("returns 'none' when the pane is hidden", () => {
    expect(decideAutoScrollMode({ ...base, isActive: false })).toBe("none");
  });

  it("returns 'none' when the user scrolled away from the bottom", () => {
    expect(decideAutoScrollMode({ ...base, shouldAutoScroll: false })).toBe("none");
  });

  it("returns 'none' while another scroll-restore effect is mid-flight", () => {
    expect(decideAutoScrollMode({ ...base, isRestoringScroll: true })).toBe("none");
  });

  // The actual regression: a cache-restore that jumps from 0 → 100
  // messages must NOT use smooth scroll. The previous bug was the
  // smooth-scroll animation getting stranded mid-chat when subsequent
  // appends moved the target bottom further down.
  it("uses 'instant' on initial cache restore (prev=0 → many)", () => {
    expect(
      decideAutoScrollMode({ ...base, prevCount: 0, newCount: 100 }),
    ).toBe("instant");
  });

  it("uses 'instant' on a catchup batch (many existing + many new)", () => {
    expect(
      decideAutoScrollMode({ ...base, prevCount: 50, newCount: 70 }),
    ).toBe("instant");
  });

  it("uses 'instant' even for a small bulk add when crossing the threshold", () => {
    expect(
      decideAutoScrollMode({ ...base, prevCount: 10, newCount: 14 }),
    ).toBe("instant");
  });

  it("uses 'smooth' for a single live stream-message append", () => {
    expect(
      decideAutoScrollMode({ ...base, prevCount: 25, newCount: 26 }),
    ).toBe("smooth");
  });

  it("uses 'smooth' for two-or-three at-a-time arrivals", () => {
    expect(
      decideAutoScrollMode({ ...base, prevCount: 25, newCount: 28 }),
    ).toBe("smooth");
  });

  // A "no actual change" tick (e.g., effect re-running because isActive
  // changed but messages didn't) must not animate.
  it("returns 'smooth' for a no-op tick when count is unchanged", () => {
    // grew=0 → falls through to the smooth branch but visually is a
    // no-op anyway (scrollIntoView on the same target). The point is
    // that this path doesn't accidentally land in 'instant'.
    expect(
      decideAutoScrollMode({ ...base, prevCount: 25, newCount: 25 }),
    ).toBe("smooth");
  });

  // Edge case: legitimately first messages, but only one of them (a
  // freshly started session). Smooth would be fine, but instant is
  // safer since there's nothing to animate.
  it("uses 'instant' when first message ever arrives (prev=0 → 1)", () => {
    expect(
      decideAutoScrollMode({ ...base, prevCount: 0, newCount: 1 }),
    ).toBe("instant");
  });
});
