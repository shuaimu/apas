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
  width = 80,
  height = 18,
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

  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={title}
      className="text-gray-300 dark:text-gray-600"
    >
      <title>{title}</title>
      {max === 0 ? (
        <line
          x1={0}
          y1={height - 0.5}
          x2={width}
          y2={height - 0.5}
          stroke="currentColor"
          strokeWidth={1}
          strokeDasharray="2 2"
        />
      ) : (
        buckets.map((count, idx) => {
          if (count === 0) return null;
          const h = Math.max(1, (count / max) * (height - 2));
          const isLatest = idx === BUCKET_COUNT - 1;
          return (
            <rect
              key={idx}
              x={idx * barWidth}
              y={height - h}
              width={Math.max(0.8, barWidth - 0.3)}
              height={h}
              className={
                isLatest
                  ? "fill-indigo-500 dark:fill-indigo-400"
                  : "fill-gray-400 dark:fill-gray-500"
              }
            />
          );
        })
      )}
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
