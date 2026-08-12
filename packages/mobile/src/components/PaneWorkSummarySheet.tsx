import { useEffect } from "react";
import {
  ActivityIndicator,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import type { PaneWorkSummary } from "@apas/protocol";

import { SecondaryButton, StatusBadge } from "@/components/ui";
import { connectionSupervisor } from "@/connection/runtime";
import { useTheme } from "@/design/tokens";
import { readPaneWorkSummarySnapshot } from "@/storage/cache";
import {
  mutationsAllowed,
  paneWorkSummaryKey,
  useMobileStore,
  type PaneWorkSummaryAvailability,
} from "@/state/store";

export interface SummaryPaneOption {
  id: number;
  label: string;
}

export function formatSummaryWindow(start: string, end: string): string {
  const formatter = new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
  return `${formatter.format(new Date(start))} – ${formatter.format(new Date(end))}`;
}

export function summaryStatusLabel(summary: PaneWorkSummary): string {
  switch (summary.status) {
    case "complete": return "Complete";
    case "partial": return "In progress";
    case "queued": return "Queued";
    case "generating": return "Generating";
    case "stale": return "Updating";
    case "failed": return "Failed";
    case "source_expired": return "Source expired";
    default: return "Unknown";
  }
}

export function summaryAvailabilityMessage(
  availability: PaneWorkSummaryAvailability,
): string | null {
  switch (availability) {
    case "cli_update_required":
      return "The project CLI needs an update before it can generate new summaries. Cached summaries remain available.";
    case "summarizer_disabled":
      return "Summary generation is disabled on the project host.";
    case "summarizer_unavailable":
      return "The isolated summarizer is currently unavailable. Normal agent work is unaffected.";
    case "unknown":
      return "Summary generation support has not been confirmed yet.";
    default:
      return null;
  }
}

export function canRetrySummary(summary: PaneWorkSummary): boolean {
  return summary.status === "failed";
}

function SummaryCard({
  summary,
  retryDisabled,
  onRetry,
}: {
  summary: PaneWorkSummary;
  retryDisabled: boolean;
  onRetry: (windowStart: string) => void;
}) {
  const theme = useTheme();
  const pending = summary.status === "queued" || summary.status === "generating";
  const tone = summary.status === "complete"
    ? "success"
    : summary.status === "failed" || summary.status === "source_expired"
      ? "danger"
      : "warning";
  return (
    <View
      testID={`summary-card-${summary.window_start}`}
      style={[styles.card, { backgroundColor: theme.surface, borderColor: theme.border }]}
    >
      <View style={styles.cardHeader}>
        <Text style={[styles.window, { color: theme.text }]}>
          {formatSummaryWindow(summary.window_start, summary.window_end)}
        </Text>
        <StatusBadge label={summaryStatusLabel(summary)} tone={tone} />
      </View>
      {summary.summary ? <Text style={[styles.body, { color: theme.text }]}>{summary.summary}</Text> : null}
      {pending && !summary.summary ? (
        <View accessibilityRole="progressbar" style={styles.pending}>
          <ActivityIndicator size="small" color={theme.accent} />
          <Text style={{ color: theme.textMuted }}>
            {summary.status === "queued" ? "Waiting for the project summarizer…" : "Summarizing retained activity…"}
          </Text>
        </View>
      ) : null}
      {summary.status === "stale" ? (
        <Text style={[styles.detail, { color: theme.warning }]}>Later activity changed this window; a fresh summary is being prepared.</Text>
      ) : null}
      {summary.status === "source_expired" ? (
        <Text style={[styles.detail, { color: theme.textMuted }]}>The retained conversation source expired, so this window cannot be reconstructed.</Text>
      ) : null}
      {summary.status === "failed" ? (
        <View style={styles.failure}>
          <Text style={[styles.detail, { color: theme.danger }]}>{summary.error || "Summary generation failed."}</Text>
          <SecondaryButton disabled={retryDisabled} onPress={() => onRetry(summary.window_start)}>Retry</SecondaryButton>
        </View>
      ) : null}
      <View style={styles.metadata}>
        {summary.status === "partial" && summary.source_through ? (
          <Text style={[styles.metadataText, { color: theme.textMuted }]}>Through {new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(new Date(summary.source_through))}</Text>
        ) : null}
        {typeof summary.source_message_count === "number" ? (
          <Text style={[styles.metadataText, { color: theme.textMuted }]}>{summary.source_message_count} source {summary.source_message_count === 1 ? "event" : "events"}</Text>
        ) : null}
        {summary.provider ? (
          <Text style={[styles.metadataText, { color: theme.textMuted }]}>via {summary.provider}{summary.model ? ` · ${summary.model}` : ""}</Text>
        ) : null}
      </View>
    </View>
  );
}

export function PaneWorkSummarySheet({
  visible,
  sessionId,
  paneId,
  paneLabel,
  panes,
  onSelectPane,
  onClose,
}: {
  visible: boolean;
  sessionId: string;
  paneId: number | null;
  paneLabel: string | null;
  panes: SummaryPaneOption[];
  onSelectPane: (paneId: number) => void;
  onClose: () => void;
}) {
  const theme = useTheme();
  const connection = useMobileStore((state) => state.connection);
  const supported = useMobileStore((state) => state.negotiatedCapabilities.includes("pane_work_summary_v1"));
  const key = paneId === null ? null : paneWorkSummaryKey(sessionId, paneId);
  const cache = useMobileStore((state) => key ? state.paneWorkSummaries[key] : undefined);
  const beginRequest = useMobileStore((state) => state.beginPaneWorkSummaryRequest);
  const setError = useMobileStore((state) => state.setPaneWorkSummaryError);
  const hydrate = useMobileStore((state) => state.hydratePaneWorkSummaries);
  const setVisiblePane = useMobileStore((state) => state.setVisibleSummaryPane);
  const online = connection === "ready";
  const controlsDisabled = !online || !supported || Boolean(cache?.loading);

  useEffect(() => {
    if (!visible || paneId === null || !supported) return;
    const target = { sessionId, paneId };
    let active = true;
    setVisiblePane(target);
    void readPaneWorkSummarySnapshot(sessionId, paneId)
      .then((snapshot) => {
        if (!active) return;
        if (snapshot) {
          hydrate(
            snapshot.sessionId,
            snapshot.paneId,
            snapshot.summaries,
            snapshot.availability,
            snapshot.updatedAt,
          );
        }
        if (useMobileStore.getState().connection !== "ready") return;
        beginRequest(sessionId, paneId);
        if (!connectionSupervisor()?.send({
          type: "list_pane_work_summaries",
          session_id: sessionId,
          pane_id: paneId,
          include_current: true,
        })) {
          setError(sessionId, paneId, "Reconnect to load current summaries.");
        }
      })
      .catch(() => {
        if (!active || useMobileStore.getState().connection !== "ready") return;
        beginRequest(sessionId, paneId);
        connectionSupervisor()?.send({
          type: "list_pane_work_summaries",
          session_id: sessionId,
          pane_id: paneId,
          include_current: true,
        });
      });
    return () => {
      active = false;
      const current = useMobileStore.getState().visibleSummaryPane;
      if (current?.sessionId === sessionId && current.paneId === paneId) setVisiblePane(null);
    };
  }, [beginRequest, hydrate, paneId, sessionId, setError, setVisiblePane, supported, visible]);

  const refresh = (windowStart?: string) => {
    if (paneId === null || controlsDisabled || !mutationsAllowed()) return;
    beginRequest(sessionId, paneId);
    if (!connectionSupervisor()?.send({
      type: "refresh_pane_work_summary",
      session_id: sessionId,
      pane_id: paneId,
      ...(windowStart ? { window_start: windowStart } : {}),
    })) {
      setError(sessionId, paneId, "Reconnect before refreshing summaries.");
    }
  };

  const availabilityMessage = summaryAvailabilityMessage(cache?.availability ?? "unknown");
  return (
    <Modal visible={visible} transparent animationType="slide" onRequestClose={onClose}>
      <View style={styles.modalRoot}>
        <Pressable accessibilityLabel="Close work summary" style={styles.backdrop} onPress={onClose} />
        <View style={[styles.sheet, { backgroundColor: theme.background, borderColor: theme.border }]}>
          <View style={styles.handleRow}><View style={[styles.handle, { backgroundColor: theme.border }]} /></View>
          <View style={styles.headingRow}>
            <View style={styles.headingText}>
              <Text accessibilityRole="header" style={[styles.title, { color: theme.text }]}>Work summary</Text>
              <Text style={{ color: theme.textMuted }}>{paneLabel ?? "No pane selected"}</Text>
            </View>
            <SecondaryButton onPress={onClose}>Close</SecondaryButton>
          </View>
          {panes.length > 1 ? (
            <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.panes}>
              {panes.map((pane) => (
                <SecondaryButton key={pane.id} onPress={() => onSelectPane(pane.id)}>
                  {`${pane.id === paneId ? "✓ " : ""}${pane.label}`}
                </SecondaryButton>
              ))}
            </ScrollView>
          ) : null}
          <View style={styles.toolbar}>
            <SecondaryButton disabled={controlsDisabled || !mutationsAllowed()} onPress={() => refresh()}>
              Refresh current window
            </SecondaryButton>
            {!online ? <Text style={[styles.freshness, { color: theme.textMuted }]}>Offline cached view{cache?.updatedAt ? ` · updated ${new Date(cache.updatedAt).toLocaleString()}` : ""}</Text> : null}
          </View>
          <ScrollView testID="summary-scroll" contentContainerStyle={styles.summaryList}>
            {availabilityMessage ? (
              <View style={[styles.notice, { backgroundColor: theme.surfaceMuted, borderColor: theme.warning }]}>
                <Text style={{ color: theme.text }}>{availabilityMessage}</Text>
              </View>
            ) : null}
            {cache?.error ? <Text accessibilityRole="alert" style={{ color: theme.danger }}>{cache.error}</Text> : null}
            {cache?.loading && cache.summaries.length === 0 ? (
              <View style={styles.centerState}><ActivityIndicator color={theme.accent} /><Text style={{ color: theme.textMuted }}>Loading summaries…</Text></View>
            ) : null}
            {!cache?.loading && (cache?.summaries.length ?? 0) === 0 ? (
              <View style={styles.centerState}><Text style={{ color: theme.textMuted, textAlign: "center" }}>No meaningful retained activity has been summarized for this pane yet.</Text></View>
            ) : null}
            {cache?.summaries.map((summary) => (
              <SummaryCard
                key={`${summary.window_start}-${summary.source_digest ?? "source"}`}
                summary={summary}
                retryDisabled={controlsDisabled || !mutationsAllowed() || !canRetrySummary(summary)}
                onRetry={refresh}
              />
            ))}
          </ScrollView>
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  modalRoot: { flex: 1, justifyContent: "flex-end" },
  backdrop: { position: "absolute", top: 0, right: 0, bottom: 0, left: 0, backgroundColor: "rgba(0, 0, 0, 0.4)" },
  sheet: { maxHeight: "88%", minHeight: "48%", borderTopLeftRadius: 22, borderTopRightRadius: 22, borderWidth: 1, paddingBottom: 12 },
  handleRow: { alignItems: "center", paddingVertical: 8 },
  handle: { width: 42, height: 4, borderRadius: 999 },
  headingRow: { flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 16 },
  headingText: { flex: 1 },
  title: { fontSize: 20, fontWeight: "800" },
  panes: { gap: 6, paddingHorizontal: 16, paddingTop: 10 },
  toolbar: { gap: 6, paddingHorizontal: 16, paddingTop: 10 },
  freshness: { fontSize: 12 },
  summaryList: { gap: 10, padding: 16, paddingBottom: 30 },
  card: { borderWidth: 1, borderRadius: 14, padding: 12, gap: 8 },
  cardHeader: { flexDirection: "row", alignItems: "flex-start", gap: 8 },
  window: { flex: 1, fontSize: 12, fontWeight: "700" },
  body: { fontSize: 14, lineHeight: 20 },
  pending: { flexDirection: "row", alignItems: "center", gap: 8 },
  detail: { fontSize: 12, lineHeight: 17 },
  failure: { gap: 8 },
  metadata: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  metadataText: { fontSize: 11 },
  notice: { borderWidth: 1, borderRadius: 10, padding: 10 },
  centerState: { minHeight: 120, alignItems: "center", justifyContent: "center", gap: 8, padding: 16 },
});
