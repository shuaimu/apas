import { useEffect } from "react";
import { Stack } from "expo-router";
import { StatusBar } from "expo-status-bar";
import { QueryClientProvider } from "@tanstack/react-query";
import { SafeAreaProvider } from "react-native-safe-area-context";

import { startConnectionRuntime, stopConnectionRuntime } from "@/connection/runtime";
import { useTheme } from "@/design/tokens";
import { installQueryLifecycle, queryClient } from "@/query/client";
import { loadCredentials } from "@/security/credentials";
import { readCachedSnapshot } from "@/storage/cache";
import { useMobileStore } from "@/state/store";

export default function RootLayout() {
  const theme = useTheme();
  const setHydrated = useMobileStore((state) => state.setHydrated);
  const setCachedSessions = useMobileStore((state) => state.setCachedSessions);

  useEffect(() => {
    let active = true;
    const removeQueryLifecycle = installQueryLifecycle();
    void (async () => {
      const credentials = await loadCredentials();
      if (!active) return;
      if (credentials) {
        try {
          const cached = await readCachedSnapshot();
          if (active) setCachedSessions(cached.sessions, cached.updatedAt);
        } catch {
          // A missing/corrupt cache is recoverable; bootstrap will replace it.
        }
      }
      if (!active) return;
      setHydrated(Boolean(credentials));
      if (credentials) startConnectionRuntime();
    })();
    return () => {
      active = false;
      removeQueryLifecycle();
      stopConnectionRuntime();
    };
  }, [setCachedSessions, setHydrated]);

  return (
    <SafeAreaProvider>
      <QueryClientProvider client={queryClient}>
        <StatusBar style="auto" />
        <Stack screenOptions={{ headerBackTitle: "Back", contentStyle: { backgroundColor: theme.background } }}>
          <Stack.Screen name="index" options={{ headerShown: false }} />
          <Stack.Screen name="login" options={{ headerShown: false }} />
          <Stack.Screen name="(code)" options={{ headerShown: false }} />
        </Stack>
      </QueryClientProvider>
    </SafeAreaProvider>
  );
}
