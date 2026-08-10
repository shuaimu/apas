import { ActivityIndicator } from "react-native";
import { Redirect, Stack } from "expo-router";

import { Screen } from "@/components/ui";
import { useTheme } from "@/design/tokens";
import { useMobileStore } from "@/state/store";

export default function AuthenticatedLayout() {
  const theme = useTheme();
  const { hydrated, signedIn } = useMobileStore();
  if (!hydrated) return <Screen style={{ alignItems: "center", justifyContent: "center" }}><ActivityIndicator /></Screen>;
  if (!signedIn) return <Redirect href="/login" />;
  return (
    <Stack screenOptions={{
      contentStyle: { backgroundColor: theme.background },
      headerStyle: { backgroundColor: theme.background },
      headerShadowVisible: false,
      headerTintColor: theme.text,
      headerTitleStyle: { fontWeight: "700" },
    }}>
      <Stack.Screen name="(tabs)" options={{ headerShown: false }} />
      <Stack.Screen name="new" options={{ title: "New task", presentation: "modal" }} />
      <Stack.Screen name="session/[sessionId]/index" options={{ title: "Activity" }} />
      <Stack.Screen name="session/[sessionId]/review" options={{ title: "Review" }} />
      <Stack.Screen name="session/[sessionId]/terminal" options={{ title: "Terminal" }} />
      <Stack.Screen name="settings/account" options={{ title: "Account & devices" }} />
      <Stack.Screen name="settings/notifications" options={{ title: "Notifications" }} />
    </Stack>
  );
}
