import { useMemo, useState } from "react";
import { FlatList, Pressable, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { router } from "expo-router";
import type { MobileSessionSummary } from "@apas/protocol";

import { EmptyState, OfflineBanner, PrimaryButton, Screen, SectionTitle } from "@/components/ui";
import { SessionCard, sessionActivityStatus } from "@/components/SessionCard";
import { useTheme } from "@/design/tokens";
import { useMobileStore } from "@/state/store";

type Filter = "all" | "idle";
const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "All projects" },
  { key: "idle", label: "Idle projects" },
];

function matches(
  session: MobileSessionSummary,
  paneStatuses: Record<string, string> | undefined,
  filter: Filter,
): boolean {
  const activity = sessionActivityStatus(session, paneStatuses);
  return filter === "all" || activity === "idle" || activity === "pending";
}

function timestamp(value?: string | null): number {
  const parsed = value ? Date.parse(value) : Number.NaN;
  return Number.isNaN(parsed) ? Number.NEGATIVE_INFINITY : parsed;
}

function compareSessionRecency(left: MobileSessionSummary, right: MobileSessionSummary): number {
  const leftHasUserInput = Boolean(left.last_user_input_at);
  const rightHasUserInput = Boolean(right.last_user_input_at);
  if (leftHasUserInput !== rightHasUserInput) return rightHasUserInput ? 1 : -1;
  return timestamp(right.last_user_input_at) - timestamp(left.last_user_input_at)
    || timestamp(right.latest_update_at) - timestamp(left.latest_update_at)
    || left.id.localeCompare(right.id);
}

export default function CodeHomeScreen() {
  const theme = useTheme();
  const { width } = useWindowDimensions();
  const tablet = width >= 768;
  const sessions = useMobileStore((state) => state.sessions);
  const paneStatusesBySession = useMobileStore((state) => state.paneStatusesBySession);
  const [filter, setFilter] = useState<Filter>("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const filtered = useMemo(() => sessions
    .filter((session) => matches(session, paneStatusesBySession[session.id], filter))
    .sort(compareSessionRecency), [filter, paneStatusesBySession, sessions]);
  const selected = sessions.find((session) => session.id === selectedId) ?? filtered[0];

  const open = (session: MobileSessionSummary) => {
    useMobileStore.getState().setActiveSession(session.id);
    if (tablet) setSelectedId(session.id);
    else router.push({ pathname: "/(code)/session/[sessionId]", params: { sessionId: session.id } });
  };

  return (
    <Screen>
      <OfflineBanner />
      <View style={styles.header}>
        <View style={styles.headerCopy}><SectionTitle>Coding sessions</SectionTitle><Text style={{ color: theme.textMuted }}>Active work and recent outcomes</Text></View>
        <View style={styles.headerActions}>
          <Pressable accessibilityRole="button" onPress={() => router.push("/(code)/settings/account")} style={styles.accountButton}><Text style={{ color: theme.accent, fontWeight: "700" }}>Account</Text></Pressable>
          <PrimaryButton onPress={() => router.push("/(code)/new")}>New task</PrimaryButton>
        </View>
      </View>
      <View style={styles.filters}>
        {FILTERS.map((item) => { const selectedFilter = filter === item.key; return <Pressable accessibilityRole="button" accessibilityState={{ selected: selectedFilter }} key={item.key} onPress={() => setFilter(item.key)} style={[styles.chip, { backgroundColor: selectedFilter ? theme.accent : theme.surfaceMuted }]}><Text style={{ color: selectedFilter ? "#fff" : theme.text, fontWeight: "600" }}>{item.label}</Text></Pressable>; })}
      </View>
      <View style={[styles.content, tablet && styles.tabletContent]}>
        <FlatList
          style={tablet ? styles.tabletList : undefined}
          contentContainerStyle={filtered.length ? styles.list : styles.emptyList}
          data={filtered}
          keyExtractor={(item) => item.id}
          renderItem={({ item }) => <SessionCard session={item} paneStatuses={paneStatusesBySession[item.id]} onPress={() => open(item)} />}
          ListEmptyComponent={<EmptyState title={sessions.length ? "No idle projects" : "No coding sessions yet"} body={sessions.length ? "All connected projects are currently working." : "Start the first task from an eligible project and follow its activity here."} action={<PrimaryButton onPress={() => router.push("/(code)/new")}>Start a task</PrimaryButton>} />}
        />
        {tablet ? <View style={[styles.detail, { borderColor: theme.border }]}>{selected ? <><Text style={[styles.detailTitle, { color: theme.text }]}>{selected.project_name ?? "Coding session"}</Text><Text style={{ color: theme.textMuted }}>{selected.latest_summary ?? "Open the full activity timeline to inspect this session."}</Text><PrimaryButton onPress={() => router.push({ pathname: "/(code)/session/[sessionId]", params: { sessionId: selected.id } })}>Open activity</PrimaryButton></> : <EmptyState title="Choose a session" body="Session details will appear here." />}</View> : null}
      </View>
    </Screen>
  );
}

const styles = StyleSheet.create({
  header: { flexDirection: "row", alignItems: "flex-start", justifyContent: "space-between", paddingHorizontal: 16, paddingTop: 16, gap: 12 },
  headerCopy: { flex: 1, gap: 2 },
  headerActions: { alignItems: "flex-end", gap: 4 },
  accountButton: { minHeight: 36, justifyContent: "center", paddingHorizontal: 4 },
  filters: { flexDirection: "row", gap: 8, paddingHorizontal: 16, paddingTop: 14 },
  chip: { borderRadius: 999, paddingHorizontal: 13, paddingVertical: 8 },
  content: { flex: 1 },
  tabletContent: { flexDirection: "row", gap: 16, paddingRight: 16 },
  tabletList: { flex: 0.48 },
  list: { gap: 10, padding: 16 },
  emptyList: { flexGrow: 1 },
  detail: { flex: 0.52, borderLeftWidth: 1, padding: 24, gap: 16 },
  detailTitle: { fontSize: 25, fontWeight: "800" },
});
