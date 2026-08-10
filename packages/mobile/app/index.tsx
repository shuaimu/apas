import { ActivityIndicator } from "react-native";
import { Redirect } from "expo-router";

import { Screen } from "@/components/ui";
import { useMobileStore } from "@/state/store";

export default function IndexRoute() {
  const { hydrated, signedIn } = useMobileStore();
  if (!hydrated) return <Screen style={{ alignItems: "center", justifyContent: "center" }}><ActivityIndicator /></Screen>;
  return <Redirect href={signedIn ? "/(code)/(tabs)" : "/login"} />;
}
