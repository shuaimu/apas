import { describe, it, expect } from "vitest";
import { bucketMessages } from "./ActivitySparkline";
import type { Message } from "@/lib/store";

function mkMsg(role: Message["role"], minutesAgo: number, now: Date): Message {
  return {
    id: `${role}-${minutesAgo}`,
    role,
    content: "",
    timestamp: new Date(now.getTime() - minutesAgo * 60_000),
  };
}

describe("bucketMessages", () => {
  const now = new Date("2026-05-22T12:00:00Z");

  it("returns all-zero buckets when there are no messages", () => {
    const { buckets, total, max } = bucketMessages([], now);
    expect(buckets).toHaveLength(60);
    expect(buckets.every((b) => b === 0)).toBe(true);
    expect(total).toBe(0);
    expect(max).toBe(0);
  });

  it("counts assistant messages and ignores user / system", () => {
    const msgs: Message[] = [
      mkMsg("assistant", 1, now),
      mkMsg("user", 1, now),
      mkMsg("system", 1, now),
      mkMsg("assistant", 2, now),
    ];
    const { total, max } = bucketMessages(msgs, now);
    expect(total).toBe(2);
    expect(max).toBe(1);
  });

  it("places a recent message in the last bucket", () => {
    const msgs: Message[] = [mkMsg("assistant", 0, now)];
    const { buckets, total } = bucketMessages(msgs, now);
    expect(total).toBe(1);
    expect(buckets[59]).toBe(1);
    expect(buckets[58]).toBe(0);
  });

  it("places an old message at the beginning of the window", () => {
    const msgs: Message[] = [mkMsg("assistant", 59.5, now)];
    const { buckets, total } = bucketMessages(msgs, now);
    expect(total).toBe(1);
    expect(buckets[0]).toBe(1);
    expect(buckets[59]).toBe(0);
  });

  it("drops messages outside the 60-minute window", () => {
    const msgs: Message[] = [
      mkMsg("assistant", 61, now),
      mkMsg("assistant", 120, now),
    ];
    const { total } = bucketMessages(msgs, now);
    expect(total).toBe(0);
  });

  it("ignores future-dated messages defensively", () => {
    const msgs: Message[] = [
      {
        id: "future",
        role: "assistant",
        content: "",
        timestamp: new Date(now.getTime() + 30_000),
      },
    ];
    const { total } = bucketMessages(msgs, now);
    expect(total).toBe(0);
  });

  it("reports the maximum bucket count", () => {
    const msgs: Message[] = [
      mkMsg("assistant", 5, now),
      mkMsg("assistant", 5, now),
      mkMsg("assistant", 5, now),
      mkMsg("assistant", 30, now),
    ];
    const { max, total } = bucketMessages(msgs, now);
    expect(total).toBe(4);
    expect(max).toBe(3);
  });
});
