import { Alert } from "react-native";
import { act, cleanup, render } from "@testing-library/react-native";
import type { MobileSessionSummary } from "@apas/protocol";

import TerminalScreen from "@/../app/(code)/session/[sessionId]/terminal";
import { useMobileStore } from "@/state/store";
import { publishTerminalMessage } from "@/terminal/events";

const sessionId = "58dbd62d-c40f-4b5b-a1d4-96aca52ea595";
const instanceId = "14a1094b-6862-43ec-81ae-d38069927aa3";
const mockSend = jest.fn(() => true);
const mockInjectJavaScript = jest.fn();
const mockClipboardRead = jest.fn<Promise<string>, []>();
let mockWebViewProps: {
  onMessage?: (event: { nativeEvent: { data: string } }) => void;
  onShouldStartLoadWithRequest?: (request: { url: string }) => boolean;
} = {};

jest.mock("expo-router", () => ({
  useLocalSearchParams: () => ({ sessionId, paneId: "3" }),
}));
jest.mock("expo-clipboard", () => ({
  getStringAsync: () => mockClipboardRead(),
}));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: jest.requireActual<typeof import("react-native")>("react-native").View,
}));
jest.mock("@/connection/runtime", () => ({
  connectionSupervisor: () => ({ send: mockSend }),
}));
jest.mock("react-native-webview", () => ({
  WebView: jest.requireActual<typeof import("react")>("react").forwardRef(function MockWebView(props: typeof mockWebViewProps, ref) {
    const React = jest.requireActual<typeof import("react")>("react");
    const View = jest.requireActual<typeof import("react-native")>("react-native").View;
    mockWebViewProps = props;
    React.useImperativeHandle(ref, () => ({ injectJavaScript: mockInjectJavaScript }));
    return React.createElement(View, { testID: "terminal-webview" });
  }),
}));

const session: MobileSessionSummary = {
  id: sessionId,
  project_id: "9cd95c53-90d1-472a-89c9-a3a008fc15a4",
  project_name: "terminal-test",
  hostname: "builder",
  status: "active",
  is_active: true,
};

function bridge(message: unknown): void {
  act(() => {
    mockWebViewProps.onMessage?.({ nativeEvent: { data: JSON.stringify(message) } });
  });
}

function makeRunning(): void {
  act(() => {
    publishTerminalMessage({
      type: "terminal_snapshot",
      session_id: sessionId,
      pane_id: 3,
      instance_id: instanceId,
      seq: 1,
      data_b64: "",
      lifecycle: "running",
    });
  });
  bridge({ type: "ready" });
}

describe("native terminal containment and input", () => {
  beforeEach(() => {
    jest.useFakeTimers();
    mockSend.mockClear();
    mockInjectJavaScript.mockClear();
    mockClipboardRead.mockReset();
    mockWebViewProps = {};
    useMobileStore.setState({
      hydrated: true,
      signedIn: true,
      connection: "ready",
      serverMutationsAllowed: true,
      sessions: [session],
      features: { terminal: true, coding_mutations: true },
    });
  });

  afterEach(() => {
    cleanup();
    jest.useRealTimers();
    jest.restoreAllMocks();
  });

  it("routes composed Unicode input as UTF-8 to the exact pane", () => {
    render(<TerminalScreen />);
    makeRunning();
    mockSend.mockClear();

    bridge({ type: "input", data: "你好🙂" });

    expect(mockSend).toHaveBeenCalledWith({
      type: "terminal_input",
      session_id: sessionId,
      pane_id: 3,
      data_b64: Buffer.from("你好🙂", "utf8").toString("base64"),
    });
  });

  it("debounces rotation resize bursts and sends only the final dimensions", () => {
    render(<TerminalScreen />);
    makeRunning();
    mockSend.mockClear();

    bridge({ type: "resize", cols: 80, rows: 24 });
    bridge({ type: "resize", cols: 120, rows: 38 });
    act(() => jest.advanceTimersByTime(149));
    expect(mockSend).not.toHaveBeenCalled();
    act(() => jest.advanceTimersByTime(1));
    expect(mockSend).toHaveBeenCalledTimes(1);
    expect(mockSend).toHaveBeenCalledWith(expect.objectContaining({
      type: "terminal_resize",
      cols: 120,
      rows: 38,
    }));
  });

  it("requires explicit confirmation before clipboard text enters the WebView", async () => {
    mockClipboardRead.mockResolvedValue("echo safe");
    const alert = jest.spyOn(Alert, "alert");
    render(<TerminalScreen />);
    makeRunning();
    mockInjectJavaScript.mockClear();

    await act(async () => {
      mockWebViewProps.onMessage?.({ nativeEvent: { data: JSON.stringify({ type: "paste_request" }) } });
      await Promise.resolve();
    });
    expect(mockInjectJavaScript).not.toHaveBeenCalled();

    const actions = alert.mock.calls.at(-1)?.[2];
    const paste = actions?.find((action) => action.text === "Paste");
    act(() => paste?.onPress?.());
    expect(mockInjectJavaScript).toHaveBeenCalledWith(expect.stringContaining('"type":"paste","text":"echo safe"'));
  });

  it("clears presentation and cancels pending resize when access is revoked", () => {
    render(<TerminalScreen />);
    makeRunning();
    bridge({ type: "resize", cols: 100, rows: 30 });
    mockSend.mockClear();
    mockInjectJavaScript.mockClear();

    act(() => useMobileStore.setState({ sessions: [] }));
    act(() => jest.advanceTimersByTime(200));

    expect(mockSend).not.toHaveBeenCalled();
    expect(mockInjectJavaScript).toHaveBeenCalledWith(expect.stringContaining('"type":"reset","reason":"access_lost"'));
    expect(mockWebViewProps.onShouldStartLoadWithRequest?.({ url: "https://evil.example" })).toBe(false);
  });
});
