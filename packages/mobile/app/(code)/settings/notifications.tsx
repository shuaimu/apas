import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Pressable, StyleSheet, Switch, Text, View } from "react-native";
import type { MobileNotificationPreferences } from "@apas/protocol";

import { getNotificationPreferences, updateNotificationPreferences } from "@/api/client";
import { ErrorNotice, PrimaryButton, Screen } from "@/components/ui";
import { useTheme } from "@/design/tokens";
import { requestAndRegisterNotifications } from "@/notifications";
import { useMobileStore } from "@/state/store";

type NotificationKey = "decisions" | "failures" | "pull_requests" | "completions";
const DEFAULTS: Record<NotificationKey, boolean> = { decisions: true, failures: true, pull_requests: true, completions: false };
const ROWS: { key: NotificationKey; title: string; body: string }[] = [
  { key: "decisions", title: "Decisions", body: "Questions and approvals that need your response" },
  { key: "failures", title: "Selected failures", body: "Important task or test failures" },
  { key: "pull_requests", title: "Pull requests", body: "A pull request is ready or creation failed" },
  { key: "completions", title: "Completions", body: "A coding task reached a final state" },
];

export default function NotificationSettingsScreen() {
  const theme = useTheme();
  const queryClient = useQueryClient();
  const enabled = useMobileStore((state) => Boolean(state.features.notifications));
  const preferences = useQuery({ queryKey: ["notification-preferences"], queryFn: getNotificationPreferences, enabled });
  const mutation = useMutation({
    mutationFn: updateNotificationPreferences,
    onSuccess: (value) => queryClient.setQueryData(["notification-preferences"], value),
  });
  const current = { ...DEFAULTS, ...(preferences.data ?? {}) };
  const toggle = (key: NotificationKey, value: boolean) => {
    if (!enabled || mutation.isPending) return;
    mutation.mutate({
      decisions: current.decisions,
      failures: current.failures,
      pull_requests: current.pull_requests,
      completions: current.completions,
      [key]: value,
    } satisfies MobileNotificationPreferences);
  };
  return (
    <Screen style={styles.screen}>
      <Text accessibilityRole="header" style={[styles.title, { color: theme.text }]}>Coding updates</Text>
      <Text style={[styles.body, { color: theme.textMuted }]}>Notifications are generic by default and never contain prompts, code, diffs, terminal output, secrets, or filesystem paths.</Text>
      {!enabled ? <ErrorNotice message="Mobile notifications are currently disabled by the cluster administrator." /> : null}
      {preferences.error || mutation.error ? <ErrorNotice message={(preferences.error ?? mutation.error) instanceof Error ? (preferences.error ?? mutation.error as Error).message : "Notification settings could not be updated"} /> : null}
      <PrimaryButton disabled={!enabled} onPress={() => void requestAndRegisterNotifications().then(() => preferences.refetch()).catch((error: unknown) => mutation.reset())}>Enable notifications on this device</PrimaryButton>
      <View style={[styles.group, { borderColor: theme.border, backgroundColor: theme.surface }]}>{ROWS.map((row, index) => <Pressable accessibilityRole="switch" accessibilityState={{ checked: current[row.key], disabled: !enabled }} key={row.key} onPress={() => toggle(row.key, !current[row.key])} style={[styles.row, index > 0 && { borderTopWidth: 1, borderTopColor: theme.border }]}><View style={{ flex: 1, gap: 3 }}><Text style={[styles.rowTitle, { color: theme.text }]}>{row.title}</Text><Text style={{ color: theme.textMuted }}>{row.body}</Text></View><Switch disabled={!enabled || mutation.isPending} value={current[row.key]} onValueChange={(value) => toggle(row.key, value)} /></Pressable>)}</View>
    </Screen>
  );
}

const styles = StyleSheet.create({ screen: { padding: 20, gap: 14 }, title: { fontSize: 27, fontWeight: "800" }, body: { fontSize: 16, lineHeight: 23 }, group: { borderWidth: 1, borderRadius: 15, overflow: "hidden" }, row: { flexDirection: "row", alignItems: "center", gap: 12, padding: 15 }, rowTitle: { fontSize: 16, fontWeight: "700" } });
