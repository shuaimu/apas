import { ActivityIndicator } from "react-native";
import { Redirect, useLocalSearchParams } from "expo-router";
import { Screen } from "@/components/ui";
import { useMobileStore } from "@/state/store";

export default function NewTaskAppLink() {
  const { instruction } = useLocalSearchParams<{ instruction?: string }>();
  const { hydrated, signedIn } = useMobileStore();
  if (!hydrated) return <Screen style={{ alignItems: "center", justifyContent: "center" }}><ActivityIndicator /></Screen>;
  if (!signedIn) return <Redirect href="/login" />;
  return <Redirect href={{ pathname: "/(code)/new", params: { instruction } }} />;
}
