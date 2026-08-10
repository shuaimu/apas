import type { ExpoConfig, ConfigContext } from "expo/config";

const easProjectId = "6fe5495e-c7cc-453d-87be-eab02c44ffa3";

function requireReleaseEndpoint(rawUrl: string, protocol: "https:" | "wss:", profile: string) {
  const parsed = new URL(rawUrl);
  if (profile === "production" && parsed.protocol !== protocol) {
    throw new Error(`Production mobile builds require ${protocol.replace(":", "")} endpoints`);
  }
  return rawUrl;
}

export default ({ config }: ConfigContext): ExpoConfig => {
  const buildProfile = process.env.EAS_BUILD_PROFILE ?? "development";
  const apiUrl = requireReleaseEndpoint(
    process.env.EXPO_PUBLIC_API_URL ?? "https://apas.mpaxos.com",
    "https:",
    buildProfile,
  );
  const wsUrl = requireReleaseEndpoint(
    process.env.EXPO_PUBLIC_WS_URL ?? "wss://apas.mpaxos.com",
    "wss:",
    buildProfile,
  );

  return {
    ...config,
    name: "APAS Code",
    slug: "apas",
    version: "0.1.0",
    scheme: "apas",
    orientation: "default",
    userInterfaceStyle: "automatic",
    runtimeVersion: { policy: "appVersion" },
    updates: {
      url: `https://u.expo.dev/${easProjectId}`,
    },
    ios: {
      supportsTablet: true,
      bundleIdentifier: "com.mpaxos.apas.code",
      associatedDomains: ["applinks:apas.mpaxos.com"],
      infoPlist: {
        ITSAppUsesNonExemptEncryption: false,
      },
    },
    android: {
      package: "com.mpaxos.apas.code",
      intentFilters: [
        {
          action: "VIEW",
          autoVerify: true,
          data: [{ scheme: "https", host: "apas.mpaxos.com", pathPrefix: "/code" }],
          category: ["BROWSABLE", "DEFAULT"],
        },
      ],
    },
    plugins: [
      "expo-router",
      ["expo-secure-store", { configureAndroidBackup: false }],
      ["expo-sqlite", { useSQLCipher: true, enableFTS: false }],
      [
        "expo-notifications",
        {
          color: "#5b5bd6",
          defaultChannel: "coding-updates",
          enableBackgroundRemoteNotifications: false,
        },
      ],
    ],
    experiments: { typedRoutes: true },
    extra: {
      apiUrl,
      wsUrl,
      buildProfile,
      pushProjectId: process.env.EXPO_PUBLIC_EAS_PROJECT_ID ?? easProjectId,
      eas: { projectId: easProjectId },
    },
  };
};
