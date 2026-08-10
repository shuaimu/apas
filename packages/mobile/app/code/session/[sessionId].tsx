import { ActivityIndicator } from "react-native";
import { Redirect, useLocalSearchParams } from "expo-router";
import { Screen } from "@/components/ui";
import { useMobileStore } from "@/state/store";

export default function SessionAppLink() {
  const { sessionId } = useLocalSearchParams<{ sessionId: string }>();
  const { hydrated, signedIn, connection, sessions } = useMobileStore();
  if (!hydrated || (signedIn && connection !== "ready")) return <Screen style={{ alignItems: "center", justifyContent: "center" }}><ActivityIndicator /></Screen>;
  if (!signedIn) return <Redirect href="/login" />;
  if (!sessions.some((session) => session.id === sessionId)) return <Redirect href="/(code)/(tabs)" />;
  return <Redirect href={{ pathname: "/(code)/session/[sessionId]", params: { sessionId } }} />;
}
