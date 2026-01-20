"use client";

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { MessageList } from "@/components/chat/MessageList";
import { DualPaneView } from "@/components/chat/DualPaneView";
import { Sidebar } from "@/components/Sidebar";
import { useStore } from "@/lib/store";
import { Settings, Wifi, WifiOff, LogOut } from "lucide-react";

export default function Home() {
  const router = useRouter();
  const { connected, connect, sessionId, isDualPane, isAuthenticated, logout, token } = useStore();
  const [isCheckingAuth, setIsCheckingAuth] = useState(true);

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

  // Show loading while checking auth
  if (isCheckingAuth) {
    return (
      <div className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-gray-900">
        <div className="text-gray-500">Loading...</div>
      </div>
    );
  }

  return (
    <div className="flex h-screen bg-background">
      {/* Sidebar */}
      <Sidebar />

      {/* Main content */}
      <div className="flex-1 flex flex-col">
        {/* Header */}
        <header className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-800">
          <div className="flex items-center gap-2">
            <h1 className="text-xl font-semibold">APAS</h1>
            <span className="text-sm text-gray-500">Claude Code Remote</span>
          </div>
          <div className="flex items-center gap-2">
            {/* Connection status */}
            <div className="flex items-center gap-1 text-sm">
              {connected ? (
                <>
                  <Wifi className="w-4 h-4 text-green-500" />
                  <span className="text-green-500">Connected</span>
                </>
              ) : (
                <>
                  <WifiOff className="w-4 h-4 text-gray-400" />
                  <span className="text-gray-400">Disconnected</span>
                </>
              )}
            </div>
            <button
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
    </div>
  );
}
