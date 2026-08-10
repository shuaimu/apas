import type { PropsWithChildren, ReactNode } from "react";
import {
  ActivityIndicator,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
  type PressableProps,
  type TextInputProps,
  type ViewStyle,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { useTheme } from "@/design/tokens";
import { useMobileStore } from "@/state/store";

export function Screen({ children, style }: PropsWithChildren<{ style?: ViewStyle }>) {
  const theme = useTheme();
  return <SafeAreaView style={[styles.screen, { backgroundColor: theme.background }, style]}>{children}</SafeAreaView>;
}

export function PrimaryButton({
  children,
  loading,
  disabled,
  ...props
}: PropsWithChildren<PressableProps & { loading?: boolean }>) {
  const theme = useTheme();
  const isDisabled = disabled || loading;
  return (
    <Pressable
      accessibilityRole="button"
      disabled={isDisabled}
      style={({ pressed }) => [
        styles.primaryButton,
        { backgroundColor: pressed ? theme.accentPressed : theme.accent, opacity: isDisabled ? 0.45 : 1 },
      ]}
      {...props}
    >
      {loading ? <ActivityIndicator color="#fff" /> : <Text style={styles.primaryButtonText}>{children}</Text>}
    </Pressable>
  );
}

export function SecondaryButton({ children, ...props }: PropsWithChildren<PressableProps>) {
  const theme = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      style={({ pressed }) => [styles.secondaryButton, { borderColor: theme.border, backgroundColor: pressed ? theme.surfaceMuted : theme.surface }]}
      {...props}
    >
      <Text style={[styles.secondaryButtonText, { color: theme.text }]}>{children}</Text>
    </Pressable>
  );
}

export function FormField({ label, error, ...props }: TextInputProps & { label: string; error?: string | null }) {
  const theme = useTheme();
  return (
    <View style={styles.field}>
      <Text style={[styles.label, { color: theme.text }]}>{label}</Text>
      <TextInput
        accessibilityLabel={label}
        placeholderTextColor={theme.textMuted}
        style={[styles.input, { backgroundColor: theme.surface, borderColor: error ? theme.danger : theme.border, color: theme.text }]}
        {...props}
      />
      {error ? <Text accessibilityRole="alert" style={[styles.errorText, { color: theme.danger }]}>{error}</Text> : null}
    </View>
  );
}

export function StatusBadge({ label, tone = "neutral" }: { label: string; tone?: "neutral" | "success" | "warning" | "danger" }) {
  const theme = useTheme();
  const color = tone === "success" ? theme.success : tone === "warning" ? theme.warning : tone === "danger" ? theme.danger : theme.textMuted;
  return <View style={[styles.badge, { borderColor: color }]}><Text style={[styles.badgeText, { color }]}>{label}</Text></View>;
}

export function OfflineBanner() {
  const theme = useTheme();
  const phase = useMobileStore((state) => state.connection);
  const lastUpdated = useMobileStore((state) => state.lastUpdatedAt);
  if (phase === "ready") return null;
  const detail = phase === "offline" && lastUpdated
    ? `Offline · last updated ${new Date(lastUpdated).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`
    : `${phase[0].toUpperCase()}${phase.slice(1)}…`;
  return <View accessibilityRole="alert" style={[styles.offline, { backgroundColor: theme.surfaceMuted }]}><Text style={{ color: theme.textMuted }}>{detail} · actions unavailable</Text></View>;
}

export function EmptyState({ title, body, action }: { title: string; body: string; action?: ReactNode }) {
  const theme = useTheme();
  return (
    <View style={styles.empty}>
      <Text style={[styles.emptyTitle, { color: theme.text }]}>{title}</Text>
      <Text style={[styles.emptyBody, { color: theme.textMuted }]}>{body}</Text>
      {action}
    </View>
  );
}

export function ErrorNotice({ message }: { message: string }) {
  const theme = useTheme();
  return <View accessibilityRole="alert" style={[styles.errorNotice, { borderColor: theme.danger }]}><Text style={{ color: theme.danger }}>{message}</Text></View>;
}

export function SectionTitle({ children }: PropsWithChildren) {
  const theme = useTheme();
  return <Text accessibilityRole="header" style={[styles.sectionTitle, { color: theme.text }]}>{children}</Text>;
}

const styles = StyleSheet.create({
  screen: { flex: 1 },
  primaryButton: { minHeight: 48, borderRadius: 12, alignItems: "center", justifyContent: "center", paddingHorizontal: 18 },
  primaryButtonText: { color: "#fff", fontSize: 16, fontWeight: "700" },
  secondaryButton: { minHeight: 44, borderRadius: 12, borderWidth: 1, alignItems: "center", justifyContent: "center", paddingHorizontal: 16 },
  secondaryButtonText: { fontSize: 15, fontWeight: "600" },
  field: { gap: 6 },
  label: { fontSize: 14, fontWeight: "600" },
  input: { minHeight: 48, borderRadius: 12, borderWidth: 1, paddingHorizontal: 14, fontSize: 16 },
  errorText: { fontSize: 13 },
  badge: { alignSelf: "flex-start", borderWidth: 1, borderRadius: 999, paddingHorizontal: 8, paddingVertical: 3 },
  badgeText: { fontSize: 12, fontWeight: "700" },
  offline: { paddingHorizontal: 16, paddingVertical: 8, alignItems: "center" },
  empty: { flex: 1, alignItems: "center", justifyContent: "center", gap: 10, padding: 32 },
  emptyTitle: { fontSize: 21, fontWeight: "700", textAlign: "center" },
  emptyBody: { maxWidth: 360, lineHeight: 21, textAlign: "center", marginBottom: 8 },
  errorNotice: { borderWidth: 1, borderRadius: 10, padding: 12 },
  sectionTitle: { fontSize: 22, fontWeight: "700" },
});
