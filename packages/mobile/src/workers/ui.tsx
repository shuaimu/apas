import type { PropsWithChildren } from "react";
import { KeyboardAvoidingView, Platform, ScrollView } from "react-native";
import { ErrorNotice, Screen, SectionTitle } from "@/components/ui";
import { Copy, styles } from "@/jobs/ui";
import { useWorkers } from "./runtime";
import type { WorkerLimits, WorkerBudget } from "@apas/protocol";

export function WorkerPage({ title, children }: PropsWithChildren<{ title: string }>) {
  const state = useWorkers();
  return <Screen><KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === "ios" ? "padding" : "height"} keyboardVerticalOffset={88}><ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={styles.page}><SectionTitle>{title}</SectionTitle>{(!state.authorized || !state.online || !state.foreground) && <ErrorNotice message="Offline or reauthorizing. Cached content is stale and read-only; drafts do not grant authority." />}{!state.supported && <Copy>Recurring workers are unavailable or disabled on this server. Socket connectivity does not enable them.</Copy>}{children}</ScrollView></KeyboardAvoidingView></Screen>;
}
export function RecurringAllowance({ limits, budget }: { limits: WorkerLimits; budget?: WorkerBudget }) {
  return <><SectionTitle>Rolling 24-hour allowance</SectionTitle><Copy>Elapsed {budget?.usage.elapsed_seconds ?? "unknown"} / {limits.elapsed_seconds_24h}s; reserved {budget?.reserved_elapsed_seconds ?? "unknown"}s</Copy><Copy>{budget?.usage.estimated ? "Estimated" : "Reported"} spend {budget?.usage.spend_micro_usd == null ? "unknown" : `$${budget.usage.spend_micro_usd / 1_000_000}`} / ${limits.spend_micro_usd_24h / 1_000_000}; reserved {budget ? `$${budget.reserved_spend_micro_usd / 1_000_000}` : "unknown"}</Copy><Copy>Runs {budget?.runs ?? "unknown"} / {limits.runs_24h}; reserved {budget?.reserved_runs ?? "unknown"}. Attempts per item: {limits.attempts_per_item} (not reset by edits or resume).</Copy>{budget && <Copy>Accounting {budget.accounting_uncertain ? "uncertain — not zero" : "reconciled"}; window from {budget.window_started_at}; eligible {budget.eligible_at ?? "according to current state"}</Copy>}<Copy>This allowance automatically renews as usage leaves the rolling window until paused or stopped. Unknown usage and unresolved reservations remain held. Spend thresholds are not exact provider invoice guarantees.</Copy></>;
}
