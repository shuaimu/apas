import type { PropsWithChildren } from "react";
import { KeyboardAvoidingView, Platform, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { Screen, ErrorNotice, SectionTitle } from "@/components/ui";
import { useTheme } from "@/design/tokens";
import { useJobs } from "./runtime";
import type { JobLimits, JobUsage } from "@apas/protocol";

export function JobPage({ title, children }: PropsWithChildren<{ title: string }>) {
  const online = useJobs((state) => state.online && state.foreground && state.authorized);
  return <Screen><KeyboardAvoidingView style={{ flex: 1 }} behavior={Platform.OS === "ios" ? "padding" : "height"} keyboardVerticalOffset={88}><ScrollView keyboardShouldPersistTaps="handled" contentContainerStyle={styles.page}><SectionTitle>{title}</SectionTitle>{!online && <ErrorNotice message="Offline or reauthorizing. Cached content is stale; editing drafts is allowed, actions are disabled." />}{children}</ScrollView></KeyboardAvoidingView></Screen>;
}

export function Copy({ children }: PropsWithChildren) {
  const theme = useTheme();
  return <Text selectable style={{ color: theme.text, fontSize: 16, flexShrink: 1 }}>{children}</Text>;
}

export function JobAction({ children, onPress, disabled, selected }: PropsWithChildren<{ onPress: () => void; disabled?: boolean; selected?: boolean }>) {
  const theme = useTheme();
  return <Pressable accessibilityRole="button" accessibilityState={{ disabled: Boolean(disabled), selected }} disabled={disabled} onPress={onPress} style={({ pressed }) => [styles.action, { backgroundColor: pressed ? theme.surfaceMuted : theme.surface, borderColor: selected ? theme.accent : theme.border, opacity: disabled ? 0.5 : 1 }]}><Text style={{ color: theme.text, fontSize: 16, flexShrink: 1 }}>{children}</Text></Pressable>;
}

export function Allowance({ limits, usage }: { limits: JobLimits; usage?: JobUsage }) {
  return <View style={{ gap: 6 }}><Copy>Elapsed: {usage?.elapsed_seconds ?? 0}s / {limits.elapsed_seconds}s · attempts allowed: {limits.attempts}</Copy><Copy>{usage?.estimated ? "Estimated" : "Reported"} spend: {usage?.spend_micro_usd == null ? "unknown" : `$${(usage.spend_micro_usd / 1_000_000).toFixed(4)}`} / ${(limits.spend_micro_usd / 1_000_000).toFixed(2)} threshold</Copy><Copy>In-flight usage can exceed the reported-spend threshold; this is not a billing guarantee.</Copy><Copy>Model concurrency {limits.model_concurrency} · output tokens/call {limits.model_output_tokens} · CPU {limits.cpu_millis}m · memory {limits.memory_bytes} bytes · workspace {limits.workspace_bytes} bytes · output {limits.output_bytes} bytes · PIDs {limits.pids}</Copy></View>;
}

export const styles = StyleSheet.create({
  page: { padding: 16, gap: 14, width: "100%", maxWidth: 720, alignSelf: "center", paddingBottom: 32 },
  action: { minHeight: 48, minWidth: 48, padding: 12, borderWidth: 1, borderRadius: 10, justifyContent: "center" },
  card: { gap: 10, paddingVertical: 12 },
  code: { fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", fontSize: 13 },
});
