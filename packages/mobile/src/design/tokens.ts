import { useColorScheme } from "react-native";

const shared = {
  accent: "#6d5efc",
  accentPressed: "#5547dc",
  success: "#26a269",
  warning: "#d97706",
  danger: "#dc2626",
  radiusSmall: 8,
  radiusMedium: 14,
  radiusLarge: 22,
  space1: 4,
  space2: 8,
  space3: 12,
  space4: 16,
  space5: 24,
} as const;

export const lightTheme = {
  ...shared,
  background: "#f7f7fa",
  surface: "#ffffff",
  surfaceMuted: "#efeff5",
  sentSurface: "#eeecff",
  sentBorder: "#c8c1ff",
  text: "#18181b",
  textMuted: "#686873",
  border: "#dedee7",
  terminal: "#101014",
} as const;

export const darkTheme = {
  ...shared,
  background: "#111115",
  surface: "#1b1b21",
  surfaceMuted: "#25252d",
  sentSurface: "#292452",
  sentBorder: "#665cc7",
  text: "#f7f7fa",
  textMuted: "#aaaab6",
  border: "#383842",
  terminal: "#08080b",
} as const;

export function useTheme() {
  return useColorScheme() === "dark" ? darkTheme : lightTheme;
}
