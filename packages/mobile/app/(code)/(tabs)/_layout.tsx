import { Stack } from "expo-router";

import { useTheme } from "@/design/tokens";

export default function CodeScreens() {
  const theme = useTheme();
  return (
    <Stack screenOptions={{ headerShown: false, contentStyle: { backgroundColor: theme.background } }}>
      <Stack.Screen name="index" />
      <Stack.Screen name="attention" />
    </Stack>
  );
}
