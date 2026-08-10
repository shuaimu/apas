import { useEffect, useMemo, useRef, useState } from "react";
import { ActivityIndicator, FlatList, type NativeScrollEvent, type NativeSyntheticEvent, StyleSheet, Text, View } from "react-native";
import { router, useLocalSearchParams } from "expo-router";
import * as Crypto from "expo-crypto";
import type { CodeEvent, PaneConfig } from "@apas/protocol";

import { DecisionActions, type DecisionResponse } from "@/components/DecisionActions";
import { EventCard } from "@/components/EventCard";
import { EmptyState, ErrorNotice, FormField, OfflineBanner, PrimaryButton, Screen, SecondaryButton, StatusBadge } from "@/components/ui";
import { connectionSupervisor } from "@/connection/runtime";
import { useTheme } from "@/design/tokens";
import { readCachedSnapshot, readConversationPosition, readSelectedConversationPane, saveConversationPosition, saveSelectedConversationPane, type ConversationPosition } from "@/storage/cache";
import { mutationsAllowed, useMobileStore } from "@/state/store";

const EMPTY_EVENTS: CodeEvent[] = [];
const EMPTY_PANES: PaneConfig[] = [];

export default function SessionActivityScreen() {
  const { sessionId, eventId, paneId: requestedPaneId } = useLocalSearchParams<{ sessionId: string; eventId?: string; paneId?: string }>();
  const theme = useTheme();
  const session = useMobileStore((state) => state.sessions.find((item) => item.id === sessionId));
  const events = useMobileStore((state) => state.eventsBySession[sessionId] ?? EMPTY_EVENTS);
  const paneConfigs = useMobileStore((state) => state.panesBySession[sessionId] ?? EMPTY_PANES);
  const paneStatuses = useMobileStore((state) => state.paneStatusesBySession[sessionId]);
  const setEvents = useMobileStore((state) => state.setEvents);
  const connection = useMobileStore((state) => state.connection);
  const terminalEnabled = useMobileStore((state) => Boolean(state.features.terminal));
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [followUp, setFollowUp] = useState("");
  const [followUpRequestId, setFollowUpRequestId] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [savedPositionResult, setSavedPositionResult] = useState<{
    sessionId: string;
    paneId: number;
    position: ConversationPosition | null;
  } | null>(null);
  const initialPaneId = Number(requestedPaneId);
  const [paneSelection, setPaneSelection] = useState<{ sessionId: string; paneId: number | null }>({
    sessionId,
    paneId: Number.isInteger(initialPaneId) && initialPaneId >= 0 ? initialPaneId : null,
  });
  const timeline = useRef<FlatList<CodeEvent>>(null);
  const timelineOffset = useRef(0);
  const followNewest = useRef(true);
  const restoredPosition = useRef(false);
  const requestedSelection = paneSelection.sessionId === sessionId ? paneSelection.paneId : null;
  const panes = useMemo(() => {
    const seen = new Set<number>();
    const options = paneConfigs.map((pane) => {
      seen.add(pane.pane_id);
      return {
        id: pane.pane_id,
        kind: pane.kind ?? "agent",
        label: pane.label?.trim() || `${pane.kind === "terminal" ? "Terminal" : "Pane"} ${pane.pane_id}`,
      };
    });
    for (const event of events) {
      if (event.pane_id === undefined || event.pane_id === null || seen.has(event.pane_id)) continue;
      seen.add(event.pane_id);
      options.push({
        id: event.pane_id,
        kind: event.kind === "terminal" ? "terminal" : "agent",
        label: `Pane ${event.pane_id}`,
      });
    }
    return options;
  }, [events, paneConfigs]);
  const preferredPane = panes.find((pane) => pane.kind !== "terminal") ?? panes[0];
  const deepLinkedPaneId = eventId ? events.find((event) => event.id === eventId)?.pane_id : undefined;
  const desiredPaneId = requestedSelection ?? deepLinkedPaneId ?? null;
  const activePaneId = desiredPaneId !== null && desiredPaneId !== undefined
    && (panes.length === 0 || panes.some((pane) => pane.id === desiredPaneId))
    ? desiredPaneId
    : preferredPane?.id ?? null;
  const savedPosition = savedPositionResult?.sessionId === sessionId
    && savedPositionResult.paneId === activePaneId
    ? savedPositionResult.position
    : undefined;
  const selectedPane = panes.find((pane) => pane.id === activePaneId);
  const selectedIsTerminal = selectedPane?.kind === "terminal";
  const selectedStatus = activePaneId === null ? null : paneStatuses?.[String(activePaneId)] ?? null;
  const conversationEvents = useMemo(
    () => activePaneId === null ? [] : events.filter((event) => event.pane_id === activePaneId),
    [activePaneId, events],
  );

  useEffect(() => {
    restoredPosition.current = false;
    void readCachedSnapshot(sessionId).then((cached) => {
      setEvents(sessionId, cached.events);
      const paneWatermarks = Object.fromEntries(
        Object.entries(cached.watermarks)
          .filter(([key]) => key.startsWith(`${sessionId}:`) && !key.endsWith(":all"))
          .map(([key, value]) => [key.slice(key.lastIndexOf(":") + 1), value]),
      );
      connectionSupervisor()?.send({
        type: "get_session_messages",
        session_id: sessionId,
        limit: 100,
        pane_watermarks: Object.keys(paneWatermarks).length ? paneWatermarks : null,
        after_created_at: Object.keys(paneWatermarks).length
          ? null
          : cached.events.reduce<string | null>((latest, event) => !latest || event.created_at > latest ? event.created_at : latest, null),
      });
    }).catch(() => undefined);
    const supervisor = connectionSupervisor();
    supervisor?.send({ type: "attach_session", session_id: sessionId });
  }, [sessionId, setEvents]);

  useEffect(() => {
    if ((Number.isInteger(initialPaneId) && initialPaneId >= 0) || eventId) return;
    let active = true;
    void readSelectedConversationPane(sessionId)
      .then((rememberedPaneId) => {
        if (!active || rememberedPaneId === null) return;
        setPaneSelection((current) => current.sessionId === sessionId && current.paneId !== null
          ? current
          : { sessionId, paneId: rememberedPaneId });
      })
      .catch(() => undefined);
    return () => {
      active = false;
    };
  }, [eventId, initialPaneId, sessionId]);

  useEffect(() => {
    if (activePaneId === null) return;
    restoredPosition.current = false;
    let active = true;
    void readConversationPosition(sessionId, activePaneId)
      .then((position) => {
        if (active) setSavedPositionResult({ sessionId, paneId: activePaneId, position });
      })
      .catch(() => {
        if (active) setSavedPositionResult({ sessionId, paneId: activePaneId, position: null });
      });
    return () => {
      active = false;
    };
  }, [activePaneId, sessionId]);

  useEffect(() => {
    if (eventId || savedPosition === undefined || conversationEvents.length === 0 || restoredPosition.current) return;
    restoredPosition.current = true;
    followNewest.current = savedPosition?.followNewest ?? true;
    timelineOffset.current = savedPosition?.offset ?? 0;
    const timer = setTimeout(() => {
      if (followNewest.current) timeline.current?.scrollToEnd({ animated: false });
      else timeline.current?.scrollToOffset({ offset: timelineOffset.current, animated: false });
    }, 0);
    return () => clearTimeout(timer);
  }, [conversationEvents.length, eventId, savedPosition]);

  useEffect(() => () => {
    if (activePaneId === null) return;
    void saveConversationPosition(sessionId, activePaneId, {
      offset: timelineOffset.current,
      followNewest: followNewest.current,
    });
  }, [activePaneId, sessionId]);

  useEffect(() => {
    if (!eventId) return;
    const index = conversationEvents.findIndex((event) => event.id === eventId);
    if (index >= 0) setTimeout(() => timeline.current?.scrollToIndex({ index, animated: true, viewPosition: 0.25 }), 50);
  }, [conversationEvents, eventId]);

  if (!session) return <Screen><EmptyState title="Session unavailable" body="It may have been deleted or your project access may have changed." /></Screen>;

  const refreshActivity = () => {
    const latest = conversationEvents.at(-1)?.created_at ?? null;
    connectionSupervisor()?.send({
      type: "get_session_messages",
      session_id: sessionId,
      pane_id: activePaneId,
      limit: 100,
      after_created_at: latest,
    });
  };

  const steer = async () => {
    if (!mutationsAllowed() || !followUp.trim() || activePaneId === null) return;
    const supervisor = connectionSupervisor();
    if (!supervisor) {
      setActionError("Reconnect before sending this message.");
      return;
    }
    if (selectedIsTerminal) {
      if (!terminalEnabled) {
        setActionError("Terminal conversation access is disabled by the cluster administrator.");
        return;
      }
      const requestId = followUpRequestId ?? Crypto.randomUUID();
      setFollowUpRequestId(requestId);
      setActionError(null);
      try {
        await supervisor.sendAcknowledged({
          type: "terminal_conversation_input",
          session_id: sessionId,
          pane_id: activePaneId,
          text: followUp.trim(),
          client_msg_id: requestId,
        }, requestId);
        setFollowUp("");
        setFollowUpRequestId(null);
      } catch (error) {
        setActionError(error instanceof Error ? error.message : "The terminal conversation message was not acknowledged.");
      }
      return;
    }
    const requestId = followUpRequestId ?? Crypto.randomUUID();
    setFollowUpRequestId(requestId);
    setActionError(null);
    try {
      await supervisor.sendAcknowledged({
        type: "input",
        session_id: sessionId,
        pane_id: activePaneId,
        text: followUp.trim(),
        client_msg_id: requestId,
      }, requestId);
      setFollowUp("");
      setFollowUpRequestId(null);
    } catch (error) {
      setActionError(error instanceof Error ? error.message : "The instruction was not acknowledged. Retry safely.");
    }
  };

  const rememberTimelinePosition = (event: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
    timelineOffset.current = Math.max(0, contentOffset.y);
    followNewest.current = contentSize.height - layoutMeasurement.height - contentOffset.y <= 72;
  };

  const persistTimelinePosition = () => {
    if (activePaneId === null) return;
    void saveConversationPosition(sessionId, activePaneId, {
      offset: timelineOffset.current,
      followNewest: followNewest.current,
    });
  };

  const selectConversationPane = (nextPaneId: number) => {
    persistTimelinePosition();
    setPaneSelection({ sessionId, paneId: nextPaneId });
    void saveSelectedConversationPane(sessionId, nextPaneId);
  };

  const respondToDecision = async (event: CodeEvent, response: DecisionResponse, answer?: string) => {
    if (!mutationsAllowed() || event.pane_id === undefined) throw new Error("Reconnect and refresh this decision before responding.");
    const detail = event.detail as Record<string, unknown> | undefined;
    const requestId = Crypto.randomUUID();
    const supervisor = connectionSupervisor();
    if (!supervisor || !detail) throw new Error("This decision is no longer available.");
    if (event.kind === "plan" && detail.type === "plan_review_request" && typeof detail.tool_use_id === "string") {
      await supervisor.sendAcknowledged({
        type: "plan_review_answer",
        session_id: sessionId,
        pane_id: event.pane_id,
        tool_use_id: detail.tool_use_id,
        approve: response === "approve",
        request_id: requestId,
      }, requestId);
    } else if (event.kind === "question" && detail.type === "tool_use" && typeof detail.id === "string" && answer) {
      const input = detail.input as { questions?: { question?: unknown }[] } | undefined;
      const question = input?.questions?.[0]?.question;
      if (typeof question !== "string") throw new Error("The question payload is incomplete; refresh the session.");
      await supervisor.sendAcknowledged({
        type: "answer_question",
        session_id: sessionId,
        pane_id: event.pane_id,
        tool_use_id: detail.id,
        answers: { [question]: answer },
        request_id: requestId,
      }, requestId);
    } else if (event.kind === "approval" && detail.type === "output") {
      const outputType = detail.output_type as { approval_request?: { tool_call_id?: unknown } } | undefined;
      const toolCallId = outputType?.approval_request?.tool_call_id;
      if (typeof toolCallId !== "string") throw new Error("The approval payload is incomplete; refresh the session.");
      await supervisor.sendAcknowledged({
        type: response === "approve" ? "approve" : "reject",
        session_id: sessionId,
        pane_id: event.pane_id,
        tool_call_id: toolCallId,
        request_id: requestId,
      }, requestId);
    } else {
      throw new Error("This decision is no longer current.");
    }
    setEvents(sessionId, events.map((item) => item.id === event.id ? { ...item, requires_attention: false } : item));
    refreshActivity();
  };

  return (
    <Screen>
      <OfflineBanner />
      {actionError ? <View style={styles.error}><ErrorNotice message={actionError} /></View> : null}
      <View style={[styles.summary, { borderColor: theme.border }]}> 
        <View style={styles.row}><View style={{ flex: 1 }}><Text style={[styles.title, { color: theme.text }]}>{session.project_name ?? "Coding session"}</Text><Text style={{ color: theme.textMuted }}>{session.hostname ?? session.working_dir}</Text></View><StatusBadge label={session.is_active ? "Active" : session.status} tone={session.is_active ? "success" : "neutral"} /></View>
        <View style={styles.actions}><SecondaryButton disabled={!selectedIsTerminal || !terminalEnabled} onPress={() => { if (!selectedIsTerminal || activePaneId === null) return; persistTimelinePosition(); router.push({ pathname: "/(code)/session/[sessionId]/terminal", params: { sessionId, paneId: String(activePaneId) } }); }}>Raw terminal</SecondaryButton></View>
      </View>
      {panes.length > 0 ? <View style={styles.panes}><Text style={{ color: theme.textMuted }}>Conversation:</Text>{panes.map((pane) => <SecondaryButton key={pane.id} onPress={() => selectConversationPane(pane.id)}>{`${activePaneId === pane.id ? "✓ " : ""}${pane.label}${paneStatuses?.[String(pane.id)] ? " · working" : ""}`}</SecondaryButton>)}</View> : null}
      <FlatList
        ref={timeline}
        data={conversationEvents}
        keyExtractor={(item) => item.id}
        contentContainerStyle={conversationEvents.length ? styles.timeline : styles.empty}
        initialNumToRender={20}
        maxToRenderPerBatch={15}
        windowSize={9}
        onScroll={rememberTimelinePosition}
        scrollEventThrottle={100}
        onScrollEndDrag={persistTimelinePosition}
        onMomentumScrollEnd={persistTimelinePosition}
        onContentSizeChange={() => {
          if (restoredPosition.current && followNewest.current && !eventId) {
            timeline.current?.scrollToEnd({ animated: false });
          }
        }}
        renderItem={({ item }) => <View style={styles.event}><EventCard event={item} expanded={expanded.has(item.id)} onPress={() => setExpanded((current) => { const next = new Set(current); if (next.has(item.id)) next.delete(item.id); else next.add(item.id); return next; })} /><DecisionActions event={item} disabled={!mutationsAllowed()} onRespond={(response, answer) => respondToDecision(item, response, answer)} /></View>}
        ListHeaderComponent={conversationEvents.length ? <SecondaryButton onPress={() => { const first = conversationEvents[0]?.detail as { id?: unknown } | undefined; connectionSupervisor()?.send({ type: "get_session_messages", session_id: sessionId, pane_id: activePaneId, before_id: typeof first?.id === "string" ? first.id : null, limit: 100 }); }}>Load older activity</SecondaryButton> : null}
        onScrollToIndexFailed={({ index }) => setTimeout(() => timeline.current?.scrollToOffset({ offset: Math.max(0, index * 120), animated: true }), 50)}
        ListEmptyComponent={<EmptyState title={connection === "ready" ? "No activity yet" : "No cached activity"} body={connection === "ready" ? "Agent instructions and tool activity for this pane will appear here." : "Reconnect to retrieve this pane's activity."} />}
      />
      <View style={[styles.composer, { borderColor: theme.border, backgroundColor: theme.background }]}> 
        {connection === "ready" && selectedStatus ? <View testID="pane-working-status" accessibilityLiveRegion="polite" style={[styles.workingStatus, { backgroundColor: theme.surface }]}><ActivityIndicator size="small" color={theme.accent} /><Text numberOfLines={1} style={[styles.workingText, { color: theme.accent }]}>{selectedStatus}</Text></View> : null}
        <FormField label="Message" value={followUp} onChangeText={(value) => { setFollowUp(value); setFollowUpRequestId(null); }} placeholder={selectedIsTerminal ? "Message this terminal conversation" : "Steer this exact session and pane"} multiline />
        <PrimaryButton disabled={!mutationsAllowed() || activePaneId === null || !followUp.trim() || (selectedIsTerminal && !terminalEnabled)} onPress={() => void steer()}>{followUpRequestId ? "Retry message safely" : "Send message"}</PrimaryButton>
      </View>
    </Screen>
  );
}

const styles = StyleSheet.create({
  summary: { padding: 16, borderBottomWidth: 1, gap: 12 },
  row: { flexDirection: "row", gap: 12, alignItems: "center" },
  title: { fontSize: 22, fontWeight: "800" },
  actions: { flexDirection: "row", gap: 8 },
  panes: { flexDirection: "row", alignItems: "center", flexWrap: "wrap", gap: 6, paddingHorizontal: 16, paddingTop: 10 },
  timeline: { padding: 16, gap: 10 },
  event: { gap: 10 },
  empty: { flexGrow: 1 },
  error: { paddingHorizontal: 16, paddingTop: 10 },
  composer: { borderTopWidth: 1, padding: 12, gap: 8 },
  workingStatus: { alignSelf: "flex-start", flexDirection: "row", alignItems: "center", gap: 6, borderRadius: 999, paddingHorizontal: 9, paddingVertical: 4 },
  workingText: { maxWidth: 260, fontSize: 12, fontWeight: "700" },
});
