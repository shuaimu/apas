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
      <Stack.Screen name="workers/index" options={{ title: "Workers & Attention" }} />
      <Stack.Screen name="workers/new" options={{ title: "Approve recurring worker" }} />
      <Stack.Screen name="workers/[workerId]/index" options={{ title: "Worker responsibility" }} />
      <Stack.Screen name="workers/[workerId]/runs/[runId]/index" options={{ title: "Exact execution" }} />
      <Stack.Screen name="workers/[workerId]/runs/[runId]/questions/[questionId]" options={{ title: "Execution question" }} />
      <Stack.Screen name="workers/[workerId]/runs/[runId]/review" options={{ title: "Historical review" }} />
      <Stack.Screen name="jobs/index" options={{ title: "Earlier job results" }} />
      <Stack.Screen name="jobs/new" options={{ title: "Unapproved worker draft" }} />
      <Stack.Screen name="jobs/[jobId]/index" options={{ title: "Job detail" }} />
      <Stack.Screen name="jobs/[jobId]/questions/[questionId]" options={{ title: "Job questions" }} />
      <Stack.Screen name="jobs/[jobId]/review" options={{ title: "Job review" }} />
      <Stack.Screen name="session/[sessionId]/index" options={{ title: "Activity" }} />
      <Stack.Screen name="session/[sessionId]/review" options={{ title: "Review" }} />
      <Stack.Screen name="session/[sessionId]/terminal" options={{ title: "Terminal" }} />
      <Stack.Screen name="settings/account" options={{ title: "Account & devices" }} />
      <Stack.Screen name="settings/notifications" options={{ title: "Notifications" }} />
    </Stack>
  );
}
