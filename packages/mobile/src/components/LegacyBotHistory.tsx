import { useState } from "react";
import { ScrollView, Text, View } from "react-native";
import { router } from "expo-router";
import type { MobilePaneSummary, PaneConfig } from "@apas/protocol";
import { useTheme } from "@/design/tokens";
import { JobAction } from "@/jobs/ui";

export function LegacyBotHistory({ pane, project }: { pane: PaneConfig | MobilePaneSummary; project: string }) {
  const theme = useTheme(); const [expanded, setExpanded] = useState(false);
  const prompt = typeof pane.prompt === "string" ? pane.prompt : null;
  const cadence = typeof pane.min_iteration_interval_minutes === "number" ? pane.min_iteration_interval_minutes : null;
  return <View style={{ paddingHorizontal: 16, gap: 8 }}>
    <Text selectable style={{ color: theme.text }}>Earlier bot · original provider {pane.provider}{pane.model ? ` · ${pane.model}` : ""}{cadence === null ? "" : ` · original interval ${cadence} minutes`}. Instructions and conversation history are read-only; no conversion or provider switch is performed.</Text>
    <JobAction onPress={() => setExpanded(!expanded)}>{expanded ? "Hide original bot instructions" : "Read original bot instructions"}</JobAction>
    {expanded && <ScrollView nestedScrollEnabled style={{ maxHeight: 220 }} accessibilityLabel="Read-only original bot instructions"><Text selectable style={{ color: theme.text, fontSize: 16 }}>{prompt ?? "The original prompt is not in this cached summary. Reconnect to load its retained pane metadata; no replacement prompt is invented."}</Text></ScrollView>}
    <JobAction onPress={() => router.push({ pathname: "/(code)/workers/new", params: { project } })}>Approve a separate fresh worker</JobAction>
  </View>;
}
