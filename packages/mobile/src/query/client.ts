import { focusManager, onlineManager, QueryClient } from "@tanstack/react-query";
import NetInfo from "@react-native-community/netinfo";
import { AppState, type AppStateStatus } from "react-native";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      networkMode: "offlineFirst",
      staleTime: 15_000,
      gcTime: 24 * 60 * 60 * 1000,
      retry: 2,
    },
    mutations: { networkMode: "online", retry: 0 },
  },
});

export function installQueryLifecycle(): () => void {
  const network = NetInfo.addEventListener((state) => {
    onlineManager.setOnline(Boolean(state.isConnected && state.isInternetReachable !== false));
  });
  const appState = AppState.addEventListener("change", (status: AppStateStatus) => {
    focusManager.setFocused(status === "active");
  });
  return () => {
    network();
    appState.remove();
  };
}
