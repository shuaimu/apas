import { Redirect, useLocalSearchParams } from "expo-router";
export default function RetiredJobCreation() {
  const { project } = useLocalSearchParams<{ project?: string }>();
  return <Redirect href={{ pathname: "/(code)/workers/new", params: { project } }} />;
}
