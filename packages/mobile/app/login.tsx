import { useState } from "react";
import { KeyboardAvoidingView, Platform, StyleSheet, Text, View } from "react-native";
import { router } from "expo-router";

import { login } from "@/api/client";
import { ErrorNotice, FormField, PrimaryButton, Screen } from "@/components/ui";
import { startConnectionRuntime } from "@/connection/runtime";
import { useTheme } from "@/design/tokens";
import { useMobileStore } from "@/state/store";

export default function LoginScreen() {
  const theme = useTheme();
  const setHydrated = useMobileStore((state) => state.setHydrated);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const submit = async () => {
    if (!email.trim() || !password) {
      setError("Enter your email and password.");
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      await login(email, password);
      setHydrated(true);
      startConnectionRuntime();
      router.replace("/(code)/(tabs)");
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Sign in failed");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Screen>
      <KeyboardAvoidingView behavior={Platform.OS === "ios" ? "padding" : undefined} style={styles.center}>
        <View style={styles.form}>
          <View style={styles.heading}>
            <Text style={[styles.eyebrow, { color: theme.accent }]}>APAS CODE</Text>
            <Text accessibilityRole="header" style={[styles.title, { color: theme.text }]}>Your coding work, in reach.</Text>
            <Text style={[styles.subtitle, { color: theme.textMuted }]}>Follow active agents, handle decisions, and steer work securely from this device.</Text>
          </View>
          {error ? <ErrorNotice message={error} /> : null}
          <FormField label="Email" autoCapitalize="none" autoComplete="email" keyboardType="email-address" value={email} onChangeText={setEmail} />
          <FormField label="Password" autoCapitalize="none" autoComplete="current-password" secureTextEntry value={password} onChangeText={setPassword} onSubmitEditing={submit} />
          <PrimaryButton loading={submitting} onPress={submit}>Sign in</PrimaryButton>
          <Text style={[styles.privacy, { color: theme.textMuted }]}>Credentials are kept in secure storage on this device. Project code is not executed on your phone.</Text>
        </View>
      </KeyboardAvoidingView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  center: { flex: 1, justifyContent: "center", padding: 24 },
  form: { width: "100%", maxWidth: 430, alignSelf: "center", gap: 16 },
  heading: { gap: 8, marginBottom: 10 },
  eyebrow: { fontSize: 13, fontWeight: "800", letterSpacing: 1.4 },
  title: { fontSize: 34, lineHeight: 39, fontWeight: "800" },
  subtitle: { fontSize: 16, lineHeight: 23 },
  privacy: { fontSize: 12, lineHeight: 17, textAlign: "center" },
});
