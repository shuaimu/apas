import { useMemo } from "react";
import { FlatList, Linking, ScrollView, StyleSheet, Text, View } from "react-native";
import { useLocalSearchParams } from "expo-router";
import { splitUnifiedDiff, type CodeEvent } from "@apas/protocol";

import { EmptyState, ErrorNotice, Screen, SecondaryButton, StatusBadge } from "@/components/ui";
import { useTheme } from "@/design/tokens";
import { useMobileStore } from "@/state/store";

const REVIEW_KINDS = new Set(["plan", "diff", "test", "error", "pull_request"]);

function externalUrl(event: CodeEvent): string | null {
  const detail = event.detail as { url?: unknown } | undefined;
  if (typeof detail?.url !== "string") return null;
  try {
    const url = new URL(detail.url);
    return url.protocol === "https:" && url.hostname === "github.com" ? url.toString() : null;
  } catch { return null; }
}

export default function ReviewScreen() {
  const { sessionId } = useLocalSearchParams<{ sessionId: string }>();
  const timeline = useMobileStore((state) => state.eventsBySession[sessionId]);
  const events = useMemo(() => (timeline ?? []).filter((event) => REVIEW_KINDS.has(event.kind)), [timeline]);
  return (
    <Screen>
      <FlatList
        data={events}
        keyExtractor={(item) => item.id}
        contentContainerStyle={events.length ? styles.list : styles.empty}
        renderItem={({ item }) => <ReviewCard event={item} />}
        ListEmptyComponent={<EmptyState title="Nothing to review" body="Plans, diffs, tests, errors, and pull-request results will appear here." />}
      />
    </Screen>
  );
}

export function ReviewCard({ event }: { event: CodeEvent }) {
  const theme = useTheme();
  const url = externalUrl(event);
  const rawDetail = event.detail as { diff?: unknown; error?: unknown } | undefined;
  const diff = event.kind === "diff" ? splitUnifiedDiff(rawDetail?.diff) : null;
  return (
    <View style={[styles.card, { backgroundColor: theme.surface, borderColor: event.kind === "error" ? theme.danger : theme.border }]}>
      <View style={styles.row}><StatusBadge label={event.kind.replaceAll("_", " ")} tone={event.kind === "error" ? "danger" : event.requires_attention ? "warning" : "neutral"} /><Text style={{ color: theme.textMuted }}>{new Date(event.created_at).toLocaleString()}</Text></View>
      <Text selectable style={[styles.summary, { color: theme.text }]}>{event.summary}</Text>
      {rawDetail?.error ? <ErrorNotice message={String(rawDetail.error)} /> : null}
      {diff ? <View style={styles.files}>{diff.error ? <ErrorNotice message={diff.error} /> : null}{diff.files.map((file) => <View key={file.path} style={[styles.file, { borderColor: theme.border }]}><Text style={[styles.fileName, { color: theme.text }]}>{file.path}</Text><ScrollView horizontal showsHorizontalScrollIndicator><Text selectable style={[styles.detail, styles.source, { color: theme.textMuted }]}>{file.content}</Text></ScrollView></View>)}{diff.truncated ? <ErrorNotice message="This diff was truncated for mobile rendering. Open the web app for the complete patch." /> : null}</View> : event.detail !== undefined ? <Text selectable style={[styles.detail, { color: theme.textMuted }]}>{JSON.stringify(event.detail, null, 2)}</Text> : null}
      {event.kind === "pull_request" && !url ? <ErrorNotice message="The pull-request link is missing or is not on the allowed github.com host." /> : null}
      {url ? <SecondaryButton onPress={() => void Linking.openURL(url)}>Open pull request</SecondaryButton> : null}
    </View>
  );
}

const styles = StyleSheet.create({
  list: { padding: 16, gap: 12 },
  empty: { flexGrow: 1 },
  card: { borderWidth: 1, borderRadius: 15, padding: 15, gap: 12 },
  row: { flexDirection: "row", justifyContent: "space-between", alignItems: "center" },
  summary: { fontSize: 17, lineHeight: 23, fontWeight: "600" },
  detail: { fontFamily: "monospace", fontSize: 12, lineHeight: 17 },
  source: { padding: 10, minWidth: "100%" },
  files: { gap: 10 },
  file: { borderWidth: 1, borderRadius: 10, overflow: "hidden" },
  fileName: { fontFamily: "monospace", fontWeight: "700", padding: 10 },
});
