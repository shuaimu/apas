"use client";

import { useEffect, useState, useCallback, useMemo, useRef } from "react";
import { useRouter } from "next/navigation";
import { createPortal } from "react-dom";
import { TabbedView } from "@/components/tabs/TabbedView";
import { Sidebar } from "@/components/Sidebar";
import { ResizeHandle } from "@/components/ResizeHandle";
import { MobileCodeHome } from "@/components/mobile/MobileCodeHome";
import { MobileSessionActivity } from "@/components/mobile/MobileSessionActivity";
import { useStore } from "@/lib/store";
import { reloadWindow } from "@/lib/browserActions";
import { clearAllSnapshots } from "@/lib/sessionCacheDb";
import { Settings, Wifi, WifiOff, LogOut, Menu, X, RefreshCw, Trash2 } from "lucide-react";

const MIN_SIDEBAR_WIDTH = 180;
const MAX_SIDEBAR_WIDTH = 400;
const DEFAULT_SIDEBAR_WIDTH = 256;
const REPO_URL = "https://github.com/shuaimu/apas";
const API_URL = process.env.NEXT_PUBLIC_API_URL || "https://apas.mpaxos.com";
const WEB_UI_VERSION = process.env.NEXT_PUBLIC_WEB_UI_VERSION || "00.00.0";

// Helper to get/set per-project layout preferences
function getProjectLayoutKey(cliClientId: string | null | undefined, key: string): string {
  return cliClientId ? `apas_layout_${cliClientId}_${key}` : `apas_layout_global_${key}`;
}

function getProjectLayout(cliClientId: string | null | undefined, key: string, defaultValue: string): string {
  if (typeof window === 'undefined') return defaultValue;
  // Try project-specific first
  if (cliClientId) {
    const projectValue = localStorage.getItem(getProjectLayoutKey(cliClientId, key));
    if (projectValue !== null) return projectValue;
  }
  // Fall back to global default
  const globalValue = localStorage.getItem(`apas_layout_global_${key}`);
  return globalValue !== null ? globalValue : defaultValue;
}

function setProjectLayout(cliClientId: string | null | undefined, key: string, value: string): void {
  if (typeof window === 'undefined') return;
  localStorage.setItem(getProjectLayoutKey(cliClientId, key), value);
}

function useMobileViewport(): boolean {
  const [mobile, setMobile] = useState(false);

  useEffect(() => {
    const query = typeof window.matchMedia === "function"
      ? window.matchMedia("(max-width: 767px)")
      : null;
    const update = () => setMobile(query ? query.matches : window.innerWidth < 768);
    update();
    if (query && typeof query.addEventListener === "function") {
      query.addEventListener("change", update);
      return () => query.removeEventListener("change", update);
    }
    window.addEventListener("resize", update);
    return () => window.removeEventListener("resize", update);
  }, []);

  return mobile;
}

export default function Home() {
  const router = useRouter();
  const { connected, connect, disconnect, sessionId, isAuthenticated, logout, token, userId, userEmail, serverVersion, cliClientId, cliClients, sessions, attachSession, listSessions, rebootDaemon, setUserEmail, setClusterIdentity } = useStore();
  const isMobileViewport = useMobileViewport();
  const [isCheckingAuth, setIsCheckingAuth] = useState(true);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [isReconnecting, setIsReconnecting] = useState(false);
  const [clearCacheState, setClearCacheState] = useState<"idle" | "confirm" | "clearing">("idle");
  const [mobileScreen, setMobileScreen] = useState<"home" | "session">("home");
  const reconnectConnectAttemptedRef = useRef(false);

  // Sidebar width state - per-project
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR_WIDTH);

  // Sidebar collapsed state - per-project
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);

  // Load layout when cliClientId changes
  useEffect(() => {
    const savedWidth = getProjectLayout(cliClientId, "sidebar_width", DEFAULT_SIDEBAR_WIDTH.toString());
    const width = parseInt(savedWidth, 10);
    if (!isNaN(width) && width >= MIN_SIDEBAR_WIDTH && width <= MAX_SIDEBAR_WIDTH) {
      setSidebarWidth(width);
    }
    const savedCollapsed = getProjectLayout(cliClientId, "sidebar_collapsed", "false");
    setSidebarCollapsed(savedCollapsed === "true");
  }, [cliClientId]);

  const handleSidebarResize = useCallback((delta: number) => {
    setSidebarWidth(prev => {
      const newWidth = Math.min(MAX_SIDEBAR_WIDTH, Math.max(MIN_SIDEBAR_WIDTH, prev + delta));
      return newWidth;
    });
  }, []);

  const handleSidebarResizeEnd = useCallback(() => {
    setProjectLayout(cliClientId, "sidebar_width", sidebarWidth.toString());
  }, [sidebarWidth, cliClientId]);

  const toggleSidebarCollapsed = useCallback(() => {
    setSidebarCollapsed(prev => {
      const newValue = !prev;
      setProjectLayout(cliClientId, "sidebar_collapsed", newValue.toString());
      return newValue;
    });
  }, [cliClientId]);

  const currentProjectCliVersion = useMemo(() => {
    const byClientId = cliClientId
      ? cliClients.find((client) => client.id === cliClientId)
      : undefined;
    if (byClientId?.version) return byClientId.version;

    const bySessionId = sessionId
      ? cliClients.find((client) => client.activeSession === sessionId)
      : undefined;
    return bySessionId?.version ?? null;
  }, [cliClientId, cliClients, sessionId]);

  useEffect(() => {
    // Check for token in localStorage
    const storedToken = localStorage.getItem("apas_token");
    if (!storedToken) {
      router.push("/login");
      return;
    }
    setIsCheckingAuth(false);
    // Connect with token
    connect();
  }, [connect, router]);

  // Refresh mutable identity from the server. Account status is deliberately
  // not trusted from JWT claims or stale local storage.
  useEffect(() => {
    if (userEmail) return;
    const authToken = token || (typeof window !== "undefined" ? localStorage.getItem("apas_token") : null);
    if (!authToken) return;
    const controller = new AbortController();
    Promise.resolve(fetch(`${API_URL}/auth/me`, {
      headers: { Authorization: `Bearer ${authToken}` },
      signal: controller.signal,
    }))
      .then(async (res) => {
        if (!res?.ok) return;
        const data = await res.json();
        if (data?.user_email && data?.account_status && setClusterIdentity) {
          setClusterIdentity(data.user_email, data.account_status);
        } else if (data?.user_email) {
          setUserEmail(data.user_email);
        }
      })
      .catch(() => {
        // Ignore fetch errors - keep best-effort behavior
      });
    return () => controller.abort();
  }, [token, userEmail, setUserEmail, setClusterIdentity]);

  // Handle auth failure - redirect to login
  useEffect(() => {
    if (!isCheckingAuth && !isAuthenticated && !connected && !token) {
      // Token was invalid or expired
      const storedToken = localStorage.getItem("apas_token");
      if (!storedToken) {
        router.push("/login");
      }
    }
  }, [isAuthenticated, connected, token, isCheckingAuth, router]);

  const handleLogout = () => {
    logout();
    router.push("/login");
  };

  const handleReconnect = () => {
    if (isReconnecting) return; // Prevent double-clicks
    reconnectConnectAttemptedRef.current = false;
    setIsReconnecting(true);
    // Use setTimeout(0) to let React update the UI first (show spinner)
    // before we disconnect (which also triggers a re-render)
    setTimeout(() => {
      disconnect();
      // Then reconnect after a delay to let the old connection close
      setTimeout(() => {
        reconnectConnectAttemptedRef.current = true;
        connect();
      }, 500);
    }, 50);
  };

  // Clear reconnecting state when connection is established
  useEffect(() => {
    if (isReconnecting && connected && reconnectConnectAttemptedRef.current) {
      reconnectConnectAttemptedRef.current = false;
      setIsReconnecting(false);
    }
  }, [connected, isReconnecting]);

  // Also clear reconnecting after a timeout in case connection fails
  useEffect(() => {
    if (isReconnecting) {
      const timeout = setTimeout(() => {
        reconnectConnectAttemptedRef.current = false;
        setIsReconnecting(false);
      }, 5000); // 5 second timeout
      return () => clearTimeout(timeout);
    }
  }, [isReconnecting]);

  // Show loading while checking auth
  if (isCheckingAuth) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900">
        <div className="text-gray-500">Loading...</div>
      </div>
    );
  }

  return (
    <div className="app-container flex overflow-hidden bg-background">
      {isMobileViewport && (
        <div className="flex h-full min-h-0 w-full flex-col md:hidden">
          {mobileScreen === "home" ? (
            <MobileCodeHome
              active
              connected={connected}
              legacySessions={sessions}
              token={token}
              onAccount={() => setSettingsOpen(true)}
              onManageMachines={() => router.push("/machines")}
              onOpenSession={(targetSessionId) => {
                attachSession(targetSessionId);
                setMobileScreen("session");
              }}
              onRebootDaemon={(machineId) => rebootDaemon(machineId)}
              serverVersion={serverVersion}
            />
          ) : (
            <MobileSessionActivity
              connected={connected}
              onAccount={() => setSettingsOpen(true)}
              onBack={() => {
                setMobileScreen("home");
                listSessions();
              }}
              onReconnect={handleReconnect}
            />
          )}
        </div>
      )}

      {!isMobileViewport && (
      <div className="hidden min-w-0 flex-1 overflow-hidden md:flex">
      {/* Mobile sidebar overlay */}
      {sidebarOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-40 md:hidden"
          onClick={() => setSidebarOpen(false)}
        />
      )}

      {/* Sidebar - hidden on mobile, shown on md+ */}
      {!sidebarCollapsed && (
        <>
          <div className={`
            fixed inset-y-0 left-0 z-50 transform transition-transform duration-200 ease-in-out md:relative md:translate-x-0 md:flex md:flex-shrink-0
            ${sidebarOpen ? 'translate-x-0' : '-translate-x-full'}
          `}
            style={{ width: typeof window !== 'undefined' && window.innerWidth >= 768 ? sidebarWidth : 256 }}
          >
            <Sidebar onClose={() => setSidebarOpen(false)} onCollapse={toggleSidebarCollapsed} width={typeof window !== 'undefined' && window.innerWidth >= 768 ? sidebarWidth : undefined} />
          </div>

          {/* Sidebar resize handle - only on desktop */}
          <div className="hidden md:flex h-full">
            <ResizeHandle
              direction="horizontal"
              onResize={handleSidebarResize}
              onResizeEnd={handleSidebarResizeEnd}
              className="h-full"
            />
          </div>
        </>
      )}

      {/* Collapsed sidebar expand button - only on desktop */}
      {sidebarCollapsed && (
        <div className="hidden md:flex flex-col items-center py-2 px-1 border-r border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800">
          <button
            onClick={toggleSidebarCollapsed}
            className="p-2 hover:bg-gray-200 dark:hover:bg-gray-700 rounded-lg"
            title="Expand sidebar"
          >
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 5l7 7-7 7M5 5l7 7-7 7" />
            </svg>
          </button>
        </div>
      )}

      {/* Main content */}
      <div className="flex-1 flex flex-col min-w-0 overflow-hidden">
        {/* Header */}
        <header className="hidden md:flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-800 flex-shrink-0">
          <div className="flex items-center gap-2">
            {/* Mobile menu button */}
            <button
              onClick={() => setSidebarOpen(!sidebarOpen)}
              className="p-2 hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg md:hidden"
              title="Toggle sidebar"
            >
              <Menu className="w-5 h-5" />
            </button>
            <h1 className="text-xl font-semibold">APAS</h1>
            <span className="text-sm text-gray-500 hidden sm:inline">
              Claude Code Remote · Web v{WEB_UI_VERSION} · Server v{serverVersion ?? "unknown"} · CLI v{currentProjectCliVersion ?? "unknown"}
            </span>
          </div>
          <div className="flex items-center gap-1 sm:gap-2">
            {/* Connection status with reconnect */}
            <button
              onClick={handleReconnect}
              disabled={isReconnecting}
              className={`flex items-center gap-1 text-sm rounded-lg px-2 py-1 transition-all duration-200 ${
                isReconnecting
                  ? "bg-blue-100 dark:bg-blue-900/30 cursor-wait"
                  : "hover:bg-gray-100 dark:hover:bg-gray-800"
              }`}
              title={connected ? "Click to reconnect" : "Click to connect"}
            >
              {isReconnecting ? (
                <>
                  <RefreshCw className="w-4 h-4 text-blue-500 animate-spin" />
                  <span className="text-blue-500 sm:inline">Reconnecting...</span>
                </>
              ) : connected ? (
                <>
                  <Wifi className="w-4 h-4 text-green-500" />
                  <span className="text-green-500 hidden sm:inline">Connected</span>
                </>
              ) : (
                <>
                  <WifiOff className="w-4 h-4 text-gray-400" />
                  <span className="text-gray-400 hidden sm:inline">Disconnected</span>
                </>
              )}
            </button>
            <a
              href={REPO_URL}
              target="_blank"
              rel="noopener noreferrer"
              className="hidden sm:block p-2 hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg"
              title="Open GitHub repository"
            >
              <svg
                className="w-5 h-5"
                viewBox="0 0 16 16"
                aria-hidden="true"
                focusable="false"
              >
                <path
                  fill="currentColor"
                  d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0 0 16 8c0-4.42-3.58-8-8-8Z"
                />
              </svg>
            </a>
            <button
              onClick={() => setSettingsOpen(true)}
              className="p-2 hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg"
              title="Settings"
            >
              <Settings className="w-5 h-5" />
            </button>
            <button
              onClick={handleLogout}
              className="p-2 hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg text-gray-500 hover:text-red-500"
              title="Logout"
            >
              <LogOut className="w-5 h-5" />
            </button>
          </div>
        </header>

        {/* Chat area. On phones the top header is hidden; its essential
            controls are merged into the tab-bar row via mobileLeading /
            mobileTrailing (leading menu, trailing actions). */}
        <main className="flex-1 overflow-hidden flex flex-col">
          <TabbedView
            mobileLeading={
              <button
                onClick={() => setSidebarOpen(!sidebarOpen)}
                className="md:hidden flex items-center self-stretch px-2.5 text-gray-600 dark:text-gray-300 hover:bg-gray-200 dark:hover:bg-gray-700/50"
                title="Projects / menu"
                aria-label="Toggle sidebar"
              >
                <Menu className="w-5 h-5" />
              </button>
            }
            mobileTrailing={
              <div className="md:hidden flex items-center self-stretch gap-0.5 pl-1 pr-1">
                <button
                  onClick={handleReconnect}
                  disabled={isReconnecting}
                  className="p-2 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-700/50"
                  title={connected ? "Connected — tap to reconnect" : "Disconnected — tap to connect"}
                  aria-label="Connection status"
                >
                  {isReconnecting ? (
                    <RefreshCw className="w-5 h-5 text-blue-500 animate-spin" />
                  ) : connected ? (
                    <Wifi className="w-5 h-5 text-green-500" />
                  ) : (
                    <WifiOff className="w-5 h-5 text-gray-400" />
                  )}
                </button>
                <button
                  onClick={() => setSettingsOpen(true)}
                  className="p-2 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-700/50 text-gray-600 dark:text-gray-300"
                  title="Settings"
                  aria-label="Settings"
                >
                  <Settings className="w-5 h-5" />
                </button>
                <button
                  onClick={handleLogout}
                  className="p-2 rounded-lg hover:bg-gray-200 dark:hover:bg-gray-700/50 text-gray-500 hover:text-red-500"
                  title="Logout"
                  aria-label="Logout"
                >
                  <LogOut className="w-5 h-5" />
                </button>
              </div>
            }
          />
        </main>
      </div>
      </div>
      )}

      {/* Settings Modal */}
      {settingsOpen && typeof document !== 'undefined' && createPortal(
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-[100]" onClick={() => { setSettingsOpen(false); setClearCacheState("idle"); }}>
          <div className="bg-white dark:bg-gray-800 rounded-lg max-w-md w-full mx-4 shadow-xl" onClick={(e) => e.stopPropagation()}>
            {/* Header */}
            <div className="flex items-center justify-between p-4 border-b border-gray-200 dark:border-gray-700">
              <h3 className="text-lg font-semibold">Settings</h3>
              <button
                onClick={() => { setSettingsOpen(false); setClearCacheState("idle"); }}
                className="p-1 hover:bg-gray-200 dark:hover:bg-gray-700 rounded"
              >
                <X className="w-5 h-5" />
              </button>
            </div>

            {/* Content */}
            <div className="p-4 space-y-4">
              {/* User Info */}
              <div>
                <h4 className="text-sm font-medium text-gray-500 dark:text-gray-400 mb-2">Account</h4>
                <div className="bg-gray-50 dark:bg-gray-700 rounded-lg p-3">
                  <p className="text-sm">
                    <span className="text-gray-500 dark:text-gray-400">User ID: </span>
                    <span className="font-mono text-xs">{userId || 'Not logged in'}</span>
                  </p>
                  {userEmail && (
                    <p className="text-sm mt-2">
                      <span className="text-gray-500 dark:text-gray-400">Email: </span>
                      <span className="font-mono text-xs">{userEmail}</span>
                    </p>
                  )}
                </div>
              </div>

              {/* Connection Info */}
              <div>
                <h4 className="text-sm font-medium text-gray-500 dark:text-gray-400 mb-2">Connection</h4>
                <div className="bg-gray-50 dark:bg-gray-700 rounded-lg p-3 space-y-2">
                  <p className="text-sm flex items-center gap-2">
                    <span className="text-gray-500 dark:text-gray-400">Status:</span>
                    {connected ? (
                      <span className="text-green-500 flex items-center gap-1">
                        <Wifi className="w-4 h-4" /> Connected
                      </span>
                    ) : (
                      <span className="text-gray-400 flex items-center gap-1">
                        <WifiOff className="w-4 h-4" /> Disconnected
                      </span>
                    )}
                  </p>
                  <p className="text-sm">
                    <span className="text-gray-500 dark:text-gray-400">Server: </span>
                    <span className="font-mono text-xs">{process.env.NEXT_PUBLIC_WS_URL || 'wss://apas.mpaxos.com'}</span>
                  </p>
                </div>
              </div>

              {/* Local cache */}
              <div>
                <h4 className="text-sm font-medium text-gray-500 dark:text-gray-400 mb-2">Local cache</h4>
                <div className="bg-gray-50 dark:bg-gray-700 rounded-lg p-3">
                  <p className="text-xs text-gray-500 dark:text-gray-400 mb-2">
                    Drops every cached session snapshot from this browser, then reloads. Use this if a pane appears to be missing recent messages — the page will refetch from the server on reload.
                  </p>
                  {clearCacheState === "idle" && (
                    <button
                      onClick={() => setClearCacheState("confirm")}
                      className="w-full px-3 py-2 bg-amber-500 hover:bg-amber-600 text-white rounded text-sm font-medium transition-colors flex items-center justify-center gap-2"
                    >
                      <Trash2 className="w-4 h-4" /> Clear local cache
                    </button>
                  )}
                  {clearCacheState === "confirm" && (
                    <div className="flex gap-2">
                      <button
                        onClick={async () => {
                          setClearCacheState("clearing");
                          await clearAllSnapshots();
                          reloadWindow();
                        }}
                        className="flex-1 px-3 py-2 bg-red-500 hover:bg-red-600 text-white rounded text-sm font-medium transition-colors"
                      >
                        Confirm clear & reload
                      </button>
                      <button
                        onClick={() => setClearCacheState("idle")}
                        className="flex-1 px-3 py-2 bg-gray-200 dark:bg-gray-600 hover:bg-gray-300 dark:hover:bg-gray-500 text-gray-700 dark:text-gray-100 rounded text-sm font-medium transition-colors"
                      >
                        Cancel
                      </button>
                    </div>
                  )}
                  {clearCacheState === "clearing" && (
                    <p className="text-xs text-gray-500 dark:text-gray-400 text-center py-2">
                      Clearing cache and reloading…
                    </p>
                  )}
                </div>
              </div>

              {/* Actions */}
              <div className="flex gap-2 pt-2">
                <button
                  onClick={() => {
                    handleReconnect();
                    setSettingsOpen(false);
                    setClearCacheState("idle");
                  }}
                  className="flex-1 px-4 py-2 bg-blue-500 hover:bg-blue-600 text-white rounded-lg text-sm font-medium transition-colors"
                >
                  Reconnect
                </button>
                <button
                  onClick={() => {
                    handleLogout();
                    setSettingsOpen(false);
                    setClearCacheState("idle");
                  }}
                  className="flex-1 px-4 py-2 bg-red-500 hover:bg-red-600 text-white rounded-lg text-sm font-medium transition-colors"
                >
                  Logout
                </button>
              </div>

              {/* About */}
              <div className="pt-2 border-t border-gray-200 dark:border-gray-700">
                <p className="text-xs text-gray-400 text-center">
                  APAS - Claude Code Remote Interface
                </p>
              </div>
            </div>
          </div>
        </div>,
        document.body
      )}
    </div>
  );
}
