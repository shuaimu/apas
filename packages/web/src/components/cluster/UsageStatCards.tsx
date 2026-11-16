import { formatCompactCount, formatUsd } from "@/lib/formatCount";
import { StatCard } from "@/components/overview/UsagePeriodControl";
import type { UsageCounters } from "./types";

/** Prompts, responses, tokens and cost for one period. Cost is
 *  "Unavailable" when no provider reported one, which is not "$0". */
export function UsageStatCards({ counters, narrow = false }: { counters: UsageCounters; narrow?: boolean }) {
  const tokens = counters.input_tokens + counters.output_tokens;
  return (
    <dl className={`grid grid-cols-2 gap-2 ${narrow ? "" : "sm:grid-cols-4"}`}>
      <StatCard label="Prompts" value={counters.prompts.toLocaleString()} />
      <StatCard label="Responses" value={counters.responses.toLocaleString()} />
      <StatCard label="Tokens" value={formatCompactCount(tokens)} title={tokens.toLocaleString()} />
      <StatCard label="Cost" value={counters.cost_usd_reported ? formatUsd(counters.cost_usd) : "Unavailable"} />
    </dl>
  );
}
