import { useMemo, useState } from "react";
import { FlatList, Pressable, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { router } from "expo-router";
import type { MobileSessionSummary } from "@apas/protocol";

import { EmptyState, OfflineBanner, PrimaryButton, Screen, SectionTitle } from "@/components/ui";
import { SessionCard } from "@/components/SessionCard";
import { useTheme } from "@/design/tokens";
import { useMobileStore } from "@/state/store";

type Filter = "active" | "attention" | "completed" | "recent";
const FILTERS: { key: Filter; label: string }[] = [
  { key: "active", label: "Active" },
  { key: "attention", label: "Attention" },
  { key: "completed", label: "Completed" },
  { key: "recent", label: "Recent" },
];

function matches(session: MobileSessionSummary, filter: Filter): boolean {
  if (filter === "active") return Boolean(session.is_active);
  if (filter === "attention") return (session.attention_count ?? 0) > 0;
  if (filter === "completed") return !session.is_active && ["ended", "completed"].includes(session.status);
  return true;
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
  const [filter, setFilter] = useState<Filter>("active");
  const [project, setProject] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const projects = useMemo(() => [...new Set(sessions.map((session) => session.project_name).filter(Boolean) as string[])].sort(), [sessions]);
  const filtered = useMemo(() => sessions
    .filter((session) => matches(session, filter) && (!project || session.project_name === project))
    .sort(compareSessionRecency), [filter, project, sessions]);
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
      {projects.length > 1 ? <FlatList horizontal style={styles.projectFilterList} showsHorizontalScrollIndicator={false} contentContainerStyle={styles.projectFilters} data={["All projects", ...projects]} keyExtractor={(item) => item} renderItem={({ item }) => { const value = item === "All projects" ? null : item; const selectedProject = project === value; return <Pressable onPress={() => setProject(value)} style={[styles.projectChip, { borderColor: selectedProject ? theme.accent : theme.border }]}><Text style={{ color: selectedProject ? theme.accent : theme.textMuted }}>{item}</Text></Pressable>; }} /> : null}
      <View style={[styles.content, tablet && styles.tabletContent]}>
        <FlatList
          style={tablet ? styles.tabletList : undefined}
          contentContainerStyle={filtered.length ? styles.list : styles.emptyList}
          data={filtered}
          keyExtractor={(item) => item.id}
          renderItem={({ item }) => <SessionCard session={item} onPress={() => open(item)} />}
          ListEmptyComponent={<EmptyState title={sessions.length ? `No ${filter} sessions` : "No coding sessions yet"} body={sessions.length ? "Try another status or project filter." : "Start the first task from an eligible project and follow its activity here."} action={<PrimaryButton onPress={() => router.push("/(code)/new")}>Start a task</PrimaryButton>} />}
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
  projectFilterList: { flexGrow: 0, flexShrink: 0 },
  projectFilters: { alignItems: "center", paddingHorizontal: 16, paddingVertical: 10, gap: 8 },
  projectChip: { borderWidth: 1, borderRadius: 999, paddingHorizontal: 12, paddingVertical: 6 },
  content: { flex: 1 },
  tabletContent: { flexDirection: "row", gap: 16, paddingRight: 16 },
  tabletList: { flex: 0.48 },
  list: { gap: 10, padding: 16 },
  emptyList: { flexGrow: 1 },
  detail: { flex: 0.52, borderLeftWidth: 1, padding: 24, gap: 16 },
  detailTitle: { fontSize: 25, fontWeight: "800" },
});
