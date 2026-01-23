"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { createPortal } from "react-dom";
import { MessageList } from "@/components/chat/MessageList";
import { DualPaneView } from "@/components/chat/DualPaneView";
import { Sidebar } from "@/components/Sidebar";
import { useStore } from "@/lib/store";
import { Settings, Wifi, WifiOff, LogOut, Menu, X, RefreshCw } from "lucide-react";

export default function Home() {
  const router = useRouter();
  const { connected, connect, disconnect, sessionId, isDualPane, isAuthenticated, logout, token, userId } = useStore();
  const [isCheckingAuth, setIsCheckingAuth] = useState(true);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [isReconnecting, setIsReconnecting] = useState(false);

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
    setIsReconnecting(true);
    // Use setTimeout(0) to let React update the UI first (show spinner)
    // before we disconnect (which also triggers a re-render)
    setTimeout(() => {
      disconnect();
      // Then reconnect after a delay to let the old connection close
      setTimeout(() => {
        connect();
      }, 500);
    }, 50);
  };

  // Clear reconnecting state when connection is established
  useEffect(() => {
    if (isReconnecting && connected) {
      setIsReconnecting(false);
    }
  }, [connected, isReconnecting]);

  // Also clear reconnecting after a timeout in case connection fails
  useEffect(() => {
    if (isReconnecting) {
      const timeout = setTimeout(() => {
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
    <div className="flex h-screen overflow-hidden bg-background">
      {/* Mobile sidebar overlay */}
      {sidebarOpen && (
        <div
          className="fixed inset-0 bg-black/50 z-40 md:hidden"
          onClick={() => setSidebarOpen(false)}
        />
      )}

      {/* Sidebar - hidden on mobile, shown on md+ */}
      <div className={`
        fixed inset-y-0 left-0 z-50 w-64 transform transition-transform duration-200 ease-in-out md:relative md:translate-x-0
        ${sidebarOpen ? 'translate-x-0' : '-translate-x-full'}
      `}>
        <Sidebar onClose={() => setSidebarOpen(false)} />
      </div>

      {/* Main content */}
      <div className="flex-1 flex flex-col min-w-0 overflow-hidden">
        {/* Header */}
        <header className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-800 flex-shrink-0">
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
            <span className="text-sm text-gray-500 hidden sm:inline">Claude Code Remote</span>
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

        {/* Chat area or placeholder */}
        {sessionId ? (
          <main className="flex-1 overflow-hidden flex flex-col">
            {isDualPane ? <DualPaneView /> : <MessageList />}
          </main>
        ) : (
          <div className="flex-1 flex items-center justify-center text-gray-400">
            <div className="text-center">
              <p className="text-lg">No session selected</p>
              <p className="text-sm mt-1">Select a CLI client from the sidebar to start a session</p>
            </div>
          </div>
        )}
      </div>

      {/* Settings Modal */}
      {settingsOpen && typeof document !== 'undefined' && createPortal(
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-[100]" onClick={() => setSettingsOpen(false)}>
          <div className="bg-white dark:bg-gray-800 rounded-lg max-w-md w-full mx-4 shadow-xl" onClick={(e) => e.stopPropagation()}>
            {/* Header */}
            <div className="flex items-center justify-between p-4 border-b border-gray-200 dark:border-gray-700">
              <h3 className="text-lg font-semibold">Settings</h3>
              <button
                onClick={() => setSettingsOpen(false)}
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
                    <span className="font-mono text-xs">{process.env.NEXT_PUBLIC_WS_URL || 'ws://apas.mpaxos.com:8080'}</span>
                  </p>
                </div>
              </div>

              {/* Actions */}
              <div className="flex gap-2 pt-2">
                <button
                  onClick={() => {
                    handleReconnect();
                    setSettingsOpen(false);
                  }}
                  className="flex-1 px-4 py-2 bg-blue-500 hover:bg-blue-600 text-white rounded-lg text-sm font-medium transition-colors"
                >
                  Reconnect
                </button>
                <button
                  onClick={() => {
                    handleLogout();
                    setSettingsOpen(false);
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
