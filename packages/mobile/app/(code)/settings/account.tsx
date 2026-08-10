import { useQuery } from "@tanstack/react-query";
import { Alert, FlatList, StyleSheet, Text, View } from "react-native";
import { router } from "expo-router";

import { listDevices, revokeDevice } from "@/api/client";
import { ErrorNotice, PrimaryButton, Screen, SecondaryButton, StatusBadge } from "@/components/ui";
import { useTheme } from "@/design/tokens";
import { logoutAndWipe } from "@/security/logout";
import { useMobileStore } from "@/state/store";

export default function AccountScreen() {
  const theme = useTheme();
  const userEmail = useMobileStore((state) => state.userEmail);
  const query = useQuery({ queryKey: ["mobile-devices"], queryFn: listDevices });

  const revoke = (id: string) => Alert.alert("Revoke this device?", "Its mobile credentials and associated notifications will stop working.", [
    { text: "Cancel", style: "cancel" },
    { text: "Revoke", style: "destructive", onPress: () => void revokeDevice(id).then(() => query.refetch()) },
  ]);

  const signOut = () => Alert.alert("Sign out?", "Protected credentials and the encrypted offline cache will be removed from this device.", [
    { text: "Cancel", style: "cancel" },
    { text: "Sign out", style: "destructive", onPress: () => void (async () => {
      await logoutAndWipe().catch(() => undefined);
      router.replace("/login");
    })() },
  ]);

  return (
    <Screen>
      <FlatList
        data={query.data ?? []}
        keyExtractor={(item) => item.id}
        contentContainerStyle={styles.list}
        ListHeaderComponent={<View style={styles.header}><Text style={[styles.email, { color: theme.text }]}>{userEmail ?? "Signed-in account"}</Text><Text style={{ color: theme.textMuted }}>Device sessions are revocable independently. Refresh tokens never leave secure storage.</Text><SecondaryButton onPress={() => router.push("/(code)/settings/notifications")}>Notification settings</SecondaryButton>{query.error ? <ErrorNotice message={query.error instanceof Error ? query.error.message : "Could not load devices"} /> : null}<Text style={[styles.heading, { color: theme.text }]}>Mobile devices</Text></View>}
        renderItem={({ item }) => <View style={[styles.device, { backgroundColor: theme.surface, borderColor: theme.border }]}><View style={{ flex: 1, gap: 4 }}><Text style={[styles.deviceName, { color: theme.text }]}>{item.device_name ?? item.platform}</Text><Text style={{ color: theme.textMuted }}>APAS Code {item.app_version} · last used {new Date(item.last_used_at).toLocaleString()}</Text></View><View style={styles.deviceActions}>{item.revoked_at ? <StatusBadge label="Revoked" tone="danger" /> : <SecondaryButton onPress={() => revoke(item.id)}>Revoke</SecondaryButton>}</View></View>}
        ListFooterComponent={<View style={styles.footer}><PrimaryButton onPress={signOut}>Sign out and wipe this device</PrimaryButton></View>}
      />
    </Screen>
  );
}

const styles = StyleSheet.create({ list: { padding: 16, gap: 10 }, header: { gap: 12, marginBottom: 10 }, email: { fontSize: 24, fontWeight: "800" }, heading: { fontSize: 18, fontWeight: "700", marginTop: 10 }, device: { borderWidth: 1, borderRadius: 14, padding: 14, flexDirection: "row", alignItems: "center", gap: 10 }, deviceName: { fontSize: 16, fontWeight: "700" }, deviceActions: { alignItems: "flex-end" }, footer: { marginTop: 18 } });
