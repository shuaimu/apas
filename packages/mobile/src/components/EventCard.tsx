import { Pressable, StyleSheet, Text } from "react-native";
import type { CodeEvent } from "@apas/protocol";

import { StatusBadge } from "@/components/ui";
import { useTheme } from "@/design/tokens";

export function EventCard({ event, expanded, onPress }: { event: CodeEvent; expanded: boolean; onPress: () => void }) {
  const theme = useTheme();
  const isUserMessage = event.kind === "instruction";
  const showKind = !isUserMessage && event.kind !== "agent_status";
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{ expanded }}
      onPress={onPress}
      style={[
        styles.card,
        isUserMessage && styles.sentCard,
        {
          borderColor: event.requires_attention ? theme.warning : isUserMessage ? theme.sentBorder : theme.border,
          backgroundColor: isUserMessage ? theme.sentSurface : theme.surface,
        },
      ]}
    >
      {showKind ? <StatusBadge label={event.kind.replaceAll("_", " ")} tone={event.requires_attention ? "warning" : event.kind === "error" ? "danger" : "neutral"} /> : null}
      <Text testID="event-message-line" selectable style={[styles.summary, { color: theme.text }]}>
        {event.summary}
        <Text style={[styles.time, { color: theme.textMuted }]}>  {new Date(event.created_at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}</Text>
      </Text>
      {expanded && !isUserMessage && event.detail !== undefined ? <Text selectable style={[styles.detail, { color: theme.textMuted }]}>{JSON.stringify(event.detail, null, 2)}</Text> : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  card: { borderWidth: 1, borderRadius: 14, padding: 14, gap: 10 },
  sentCard: { alignSelf: "flex-end", marginLeft: 40 },
  time: { fontSize: 12 },
  summary: { fontSize: 15, lineHeight: 21 },
  detail: { fontFamily: "monospace", fontSize: 12, lineHeight: 17 },
});
