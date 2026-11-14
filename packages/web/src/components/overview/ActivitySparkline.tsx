"use client";

import { useMemo } from "react";
import { Message } from "@/lib/store";

const BUCKET_COUNT = 60;
const BUCKET_MS = 60_000;
const WINDOW_MS = BUCKET_COUNT * BUCKET_MS;

interface ActivitySparklineProps {
  messages: Message[];
  width?: number;
  height?: number;
  now?: Date;
}

export function ActivitySparkline({
  messages,
  width = 96,
  height = 20,
  now,
}: ActivitySparklineProps) {
  const { buckets, total, max } = useMemo(
    () => bucketMessages(messages, now ?? new Date()),
    [messages, now],
  );

  const barWidth = width / BUCKET_COUNT;
  const title =
    total === 0
      ? "no activity in last hour"
      : `${total} message${total === 1 ? "" : "s"} in last hour`;

  // A baseline is always drawn, so the strip reads as an hour of time whether
  // or not anything happened in it. The bars are tall and saturated enough to
  // be seen at a glance; the faint grey ones this replaced were not.
  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={title}
      className="shrink-0 text-gray-200 dark:text-gray-700"
    >
      <title>{title}</title>
      <line
        x1={0}
        y1={height - 0.5}
        x2={width}
        y2={height - 0.5}
        stroke="currentColor"
        strokeWidth={1}
      />
      {buckets.map((count, idx) => {
        if (count === 0) return null;
        const h = Math.max(3, (count / max) * (height - 2));
        return (
          <rect
            key={idx}
            x={idx * barWidth}
            y={height - 1 - h}
            width={Math.max(1, barWidth - 0.4)}
            height={h}
            rx={0.5}
            className="fill-blue-500 dark:fill-blue-400"
          />
        );
      })}
    </svg>
  );
}

/**
 * Pure bucketing helper exported for tests. Counts assistant + tool_use
 * messages (agent emissions) per 1-minute bucket across the last hour. The
 * last bucket ends at `now`; bucket[0] is the oldest, bucket[59] the newest.
 */
export function bucketMessages(
  messages: Message[],
  now: Date,
): { buckets: number[]; total: number; max: number } {
  const buckets = new Array<number>(BUCKET_COUNT).fill(0);
  const windowStart = now.getTime() - WINDOW_MS;
  let total = 0;
  for (const m of messages) {
    if (m.role === "user" || m.role === "system") continue;
    const ts = m.timestamp.getTime();
    if (ts < windowStart || ts > now.getTime()) continue;
    const idx = Math.min(
      BUCKET_COUNT - 1,
      Math.floor((ts - windowStart) / BUCKET_MS),
    );
    buckets[idx]++;
    total++;
  }
  const max = buckets.reduce((a, b) => (b > a ? b : a), 0);
  return { buckets, total, max };
}
