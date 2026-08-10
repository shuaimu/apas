import { useCallback, useEffect, useRef, useState } from "react";
import { Alert, Keyboard, Linking, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useLocalSearchParams } from "expo-router";
import * as Clipboard from "expo-clipboard";
import { WebView, type WebViewMessageEvent } from "react-native-webview";
import { terminalHtml, parseOutboundBridgeMessage, type TerminalBridgeInbound } from "@apas/terminal-web";
import {
  initialTerminalState,
  markTerminalTransportDisconnected,
  reconcileTerminal,
  terminalTheme,
  type TerminalReconciliationState,
} from "@apas/protocol";

import { ErrorNotice, Screen, StatusBadge } from "@/components/ui";
import { connectionSupervisor } from "@/connection/runtime";
import { useTheme } from "@/design/tokens";
import { mutationsAllowed, useMobileStore } from "@/state/store";
import { subscribeTerminalMessages } from "@/terminal/events";
import { encodeTerminalInput } from "@/terminal/input";

const LINK_HOSTS = new Set(["github.com", "gitlab.com", "docs.rs", "docs.expo.dev"]);

export default function TerminalScreen() {
  const { sessionId, paneId: rawPaneId } = useLocalSearchParams<{ sessionId: string; paneId?: string }>();
  const paneId = Number(rawPaneId);
  const validPane = Number.isInteger(paneId) && paneId >= 0;
  const theme = useTheme();
  const webView = useRef<WebView>(null);
  const reconciliation = useRef<TerminalReconciliationState>(initialTerminalState());
  const resizeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const connection = useMobileStore((state) => state.connection);
  const terminalEnabled = useMobileStore((state) => Boolean(state.features.terminal));
  const sessionAccessible = useMobileStore((state) => state.sessions.some((session) => session.id === sessionId));
  const [ready, setReady] = useState(false);
  const [lifecycle, setLifecycle] = useState<TerminalReconciliationState["lifecycle"]>("unknown");
  const [status, setStatus] = useState<string | null>(null);

  const reportBridgeHealth = useCallback((event: "terminal_bridge_ready" | "terminal_bridge_rejected_message" | "terminal_bridge_crash") => {
    if (connection === "ready") connectionSupervisor()?.send({ type: "mobile_telemetry", event });
  }, [connection]);

  const inject = useCallback((message: TerminalBridgeInbound) => {
    webView.current?.injectJavaScript(`window.__APAS_TERMINAL_RECEIVE__?.(${JSON.stringify(message)});true;`);
  }, []);

  const attach = useCallback(() => {
    if (terminalEnabled && validPane && connection === "ready" && sessionAccessible) {
      connectionSupervisor()?.send({ type: "terminal_attach", session_id: sessionId, pane_id: paneId });
    }
  }, [connection, paneId, sessionAccessible, sessionId, terminalEnabled, validPane]);

  useEffect(() => {
    if (!validPane) return;
    const unsubscribe = subscribeTerminalMessages((message) => {
      if (message.session_id !== sessionId || message.pane_id !== paneId) return;
      const result = reconcileTerminal(reconciliation.current, message);
      reconciliation.current = result.state;
      setLifecycle(result.state.lifecycle);
      setStatus(result.state.status);
      if (result.action === "reset") inject({ type: "reset", reason: "process_restarted" });
      if (result.action === "snapshot" && result.dataBase64 !== undefined) inject({ type: "snapshot", dataBase64: result.dataBase64, sequence: result.state.sequence, instanceId: result.state.instanceId, truncated: result.truncated ?? false });
      if (result.action === "output" && result.dataBase64 !== undefined) inject({ type: "output", dataBase64: result.dataBase64, sequence: result.state.sequence, instanceId: result.state.instanceId });
      inject({ type: "lifecycle", lifecycle: result.state.lifecycle, status: result.state.status });
      if (result.state.needsSnapshot && connection === "ready") attach();
    });
    return unsubscribe;
  }, [attach, connection, inject, paneId, sessionId, validPane]);

  useEffect(() => {
    if ((!sessionAccessible || connection !== "ready") && resizeTimer.current) {
      clearTimeout(resizeTimer.current);
      resizeTimer.current = null;
    }
    if (!sessionAccessible) {
      reconciliation.current = initialTerminalState();
      inject({ type: "reset", reason: "access_lost" });
      inject({ type: "lifecycle", lifecycle: "unknown", status: null });
      return;
    }
    if (connection !== "ready") {
      reconciliation.current = markTerminalTransportDisconnected(reconciliation.current);
      setLifecycle(reconciliation.current.lifecycle);
      inject({ type: "lifecycle", lifecycle: reconciliation.current.lifecycle, status: reconciliation.current.status });
      return;
    }
    attach();
  }, [attach, connection, inject, sessionAccessible]);

  useEffect(() => () => {
    if (resizeTimer.current) clearTimeout(resizeTimer.current);
  }, []);

  const sendInput = (value: string) => {
    if (!ready || lifecycle !== "running" || !mutationsAllowed() || !validPane) return;
    connectionSupervisor()?.send({ type: "terminal_input", session_id: sessionId, pane_id: paneId, data_b64: encodeTerminalInput(value) });
  };

  const requestPaste = async () => {
    const text = await Clipboard.getStringAsync();
    if (!text) return;
    Alert.alert("Paste into terminal?", `Send ${text.length} character${text.length === 1 ? "" : "s"} to pane ${paneId}?`, [
      { text: "Cancel", style: "cancel" },
      { text: "Paste", onPress: () => inject({ type: "paste", text }) },
    ]);
  };

  const openLink = (urlValue: string) => {
    const url = new URL(urlValue);
    if (!LINK_HOSTS.has(url.hostname)) {
      Alert.alert("Link blocked", `${url.hostname} is not in the terminal link allowlist.`);
      return;
    }
    Alert.alert("Open external link?", url.toString(), [
      { text: "Cancel", style: "cancel" },
      { text: "Open", onPress: () => void Linking.openURL(url.toString()) },
    ]);
  };

  const onBridgeMessage = (event: WebViewMessageEvent) => {
    let raw: unknown;
    try { raw = JSON.parse(event.nativeEvent.data); } catch {
      reportBridgeHealth("terminal_bridge_rejected_message");
      return;
    }
    const message = parseOutboundBridgeMessage(raw);
    if (!message) {
      reportBridgeHealth("terminal_bridge_rejected_message");
      return;
    }
    if (message.type === "ready") {
      setReady(true);
      reportBridgeHealth("terminal_bridge_ready");
      inject({ type: "theme", theme: terminalTheme });
      inject({ type: "lifecycle", lifecycle, status });
      attach();
    } else if (message.type === "input") sendInput(message.data);
    else if (message.type === "resize") {
      if (!ready || lifecycle !== "running" || !mutationsAllowed() || !sessionAccessible) return;
      if (resizeTimer.current) clearTimeout(resizeTimer.current);
      resizeTimer.current = setTimeout(() => {
        resizeTimer.current = null;
        if (mutationsAllowed() && validPane && sessionAccessible) connectionSupervisor()?.send({ type: "terminal_resize", session_id: sessionId, pane_id: paneId, cols: message.cols, rows: message.rows });
      }, 150);
    } else if (message.type === "paste_request") void requestPaste();
    else if (message.type === "link_request") openLink(message.url);
  };

  if (!validPane) return <Screen><ErrorNotice message="Choose an exact terminal pane from the activity screen before attaching." /></Screen>;
  const visibleLifecycle = sessionAccessible ? lifecycle : "unknown";
  const visibleStatus = sessionAccessible ? status : null;
  const inputReady = ready && visibleLifecycle === "running" && mutationsAllowed() && sessionAccessible;
  return (
    <Screen>
      <View style={[styles.statusBar, { backgroundColor: theme.terminal }]}><Text style={styles.title}>Pane {paneId}</Text><StatusBadge label={visibleLifecycle} tone={visibleLifecycle === "running" ? "success" : visibleLifecycle === "exited" ? "danger" : "warning"} />{visibleStatus ? <Text numberOfLines={1} style={styles.status}>{visibleStatus}</Text> : null}</View>
      {!terminalEnabled ? <ErrorNotice message="Mobile terminal access is disabled by the cluster administrator." /> : null}
      <WebView
        ref={webView}
        source={{ html: terminalHtml, baseUrl: "about:blank" }}
        originWhitelist={["about:blank"]}
        javaScriptEnabled
        domStorageEnabled={false}
        allowFileAccess={false}
        allowUniversalAccessFromFileURLs={false}
        setSupportMultipleWindows={false}
        mixedContentMode="never"
        onMessage={onBridgeMessage}
        onError={() => reportBridgeHealth("terminal_bridge_crash")}
        onContentProcessDidTerminate={() => {
          setReady(false);
          reportBridgeHealth("terminal_bridge_crash");
        }}
        onRenderProcessGone={() => {
          setReady(false);
          reportBridgeHealth("terminal_bridge_crash");
        }}
        onShouldStartLoadWithRequest={(request) => request.url === "about:blank"}
        onOpenWindow={() => undefined}
        style={{ flex: 1, backgroundColor: theme.terminal }}
      />
      <ScrollView horizontal keyboardShouldPersistTaps="always" contentContainerStyle={[styles.accessories, { backgroundColor: theme.surfaceMuted }]}>
        <KeyButton label="Esc" disabled={!inputReady} onPress={() => sendInput("\u001b")} />
        <KeyButton label="Tab" disabled={!inputReady} onPress={() => sendInput("\t")} />
        <KeyButton label="Ctrl-C" disabled={!inputReady} onPress={() => sendInput("\u0003")} />
        <KeyButton label="Ctrl-D" disabled={!inputReady} onPress={() => sendInput("\u0004")} />
        <KeyButton label="←" disabled={!inputReady} onPress={() => sendInput("\u001b[D")} />
        <KeyButton label="↑" disabled={!inputReady} onPress={() => sendInput("\u001b[A")} />
        <KeyButton label="↓" disabled={!inputReady} onPress={() => sendInput("\u001b[B")} />
        <KeyButton label="→" disabled={!inputReady} onPress={() => sendInput("\u001b[C")} />
        <KeyButton label="Paste" disabled={!inputReady} onPress={() => void requestPaste()} />
        <KeyButton label="Hide keyboard" onPress={() => Keyboard.dismiss()} />
        <KeyButton label="Focus" onPress={() => inject({ type: "focus" })} />
      </ScrollView>
    </Screen>
  );
}

function KeyButton({ label, disabled, onPress }: { label: string; disabled?: boolean; onPress: () => void }) {
  return <Pressable accessibilityRole="button" disabled={disabled} onPress={onPress} style={[styles.key, disabled && { opacity: 0.4 }]}><Text style={styles.keyText}>{label}</Text></Pressable>;
}

const styles = StyleSheet.create({
  statusBar: { minHeight: 46, flexDirection: "row", alignItems: "center", gap: 10, paddingHorizontal: 12 },
  title: { color: "#f2f2f4", fontSize: 16, fontWeight: "800" },
  status: { color: "#aaaab6", flex: 1 },
  accessories: { gap: 6, padding: 7 },
  key: { minHeight: 38, minWidth: 44, borderRadius: 8, backgroundColor: "#34343e", paddingHorizontal: 10, alignItems: "center", justifyContent: "center" },
  keyText: { color: "#f2f2f4", fontWeight: "700" },
});
