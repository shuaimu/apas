import { Pressable, StyleSheet, Text, View } from "react-native";
import type { MobileSessionSummary } from "@apas/protocol";

import { StatusBadge } from "@/components/ui";
import { useTheme } from "@/design/tokens";

export type MobileSessionActivityStatus = "working" | "idle" | "offline";

export function sessionActivityStatus(
  session: MobileSessionSummary,
  paneStatuses?: Record<string, string>,
): MobileSessionActivityStatus {
  if (!session.is_active) return "offline";
  if (Object.values(paneStatuses ?? {}).some(Boolean) || session.is_working) return "working";
  return "idle";
}

export function SessionCard({
  session,
  paneStatuses,
  onPress,
}: {
  session: MobileSessionSummary;
  paneStatuses?: Record<string, string>;
  onPress: () => void;
}) {
  const theme = useTheme();
  const activity = sessionActivityStatus(session, paneStatuses);
  const tone = activity === "working" ? "success" : activity === "idle" ? "neutral" : "danger";
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`Open ${session.project_name ?? "coding session"}`}
      onPress={onPress}
      style={({ pressed }) => [styles.card, { backgroundColor: theme.surface, borderColor: theme.border, opacity: pressed ? 0.75 : 1 }]}
    >
      <View style={styles.row}>
        <Text numberOfLines={1} style={[styles.title, { color: theme.text }]}>{session.project_name ?? "Coding session"}</Text>
        <StatusBadge label={activity[0].toUpperCase() + activity.slice(1)} tone={tone} />
      </View>
      <Text numberOfLines={1} style={{ color: theme.textMuted }}>{session.hostname ?? session.working_dir ?? "Unknown target"}</Text>
      {session.latest_summary ? <Text numberOfLines={2} style={[styles.summary, { color: theme.text }]}>{session.latest_summary}</Text> : null}
      <View style={styles.row}>
        <Text style={[styles.meta, { color: theme.textMuted }]}>{session.latest_update_at ? new Date(session.latest_update_at).toLocaleString() : "No recent activity"}</Text>
        {(session.attention_count ?? 0) > 0 ? <StatusBadge label={`${session.attention_count} attention`} tone="warning" /> : null}
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  card: { borderWidth: 1, borderRadius: 16, padding: 15, gap: 9 },
  row: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 10 },
  title: { flex: 1, fontSize: 17, fontWeight: "700" },
  summary: { lineHeight: 20 },
  meta: { flex: 1, fontSize: 12 },
});
