import { FlatList, Pressable, StyleSheet, Text, View } from "react-native";
import { router } from "expo-router";
import { deriveAttention, type CodeEvent } from "@apas/protocol";

import { EmptyState, OfflineBanner, Screen, SectionTitle, StatusBadge } from "@/components/ui";
import { useTheme } from "@/design/tokens";
import { useMobileStore } from "@/state/store";

export default function AttentionScreen() {
  const theme = useTheme();
  const sessions = useMobileStore((state) => state.sessions);
  const eventsBySession = useMobileStore((state) => state.eventsBySession);
  const attentionEvents = Object.values(eventsBySession).flatMap(deriveAttention);
  const open = (event: CodeEvent) => router.push({
    pathname: "/(code)/session/[sessionId]",
    params: { sessionId: event.session_id, eventId: event.id, paneId: event.pane_id?.toString() },
  });
  return (
    <Screen>
      <OfflineBanner />
      <View style={styles.header}>
        <Pressable accessibilityRole="button" onPress={() => router.back()} style={styles.backButton}><Text style={{ color: theme.accent, fontWeight: "700" }}>‹ Coding sessions</Text></Pressable>
        <SectionTitle>Needs attention</SectionTitle>
        <Text style={{ color: theme.textMuted }}>Server-authoritative pending decisions and selected failures</Text>
      </View>
      <FlatList
        data={attentionEvents}
        keyExtractor={(item) => item.id}
        contentContainerStyle={attentionEvents.length ? styles.list : styles.empty}
        renderItem={({ item }) => {
          const session = sessions.find((candidate) => candidate.id === item.session_id);
          return <Pressable accessibilityRole="button" onPress={() => open(item)} style={({ pressed }) => [styles.card, { backgroundColor: theme.surface, borderColor: theme.warning, opacity: pressed ? 0.75 : 1 }]}><View style={styles.row}><Text style={[styles.project, { color: theme.text }]}>{session?.project_name ?? "Coding session"}</Text><StatusBadge label={item.kind} tone="warning" /></View><Text style={[styles.summary, { color: theme.text }]}>{item.summary}</Text><Text style={{ color: theme.textMuted }}>Pane {item.pane_id ?? "default"} · {new Date(item.created_at).toLocaleString()}</Text></Pressable>;
        }}
        ListEmptyComponent={<EmptyState title="Nothing needs you" body="Questions, approval requests, and selected failures will appear here." />}
      />
    </Screen>
  );
}

const styles = StyleSheet.create({ header: { padding: 16, gap: 5 }, backButton: { alignSelf: "flex-start", minHeight: 36, justifyContent: "center", marginBottom: 2 }, list: { padding: 16, gap: 10 }, empty: { flexGrow: 1 }, card: { borderWidth: 1, borderRadius: 15, padding: 15, gap: 9 }, row: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 10 }, project: { fontSize: 17, fontWeight: "700" }, summary: { fontSize: 15, lineHeight: 21 } });
