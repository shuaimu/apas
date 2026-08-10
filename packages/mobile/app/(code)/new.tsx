import { useEffect, useMemo, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { router, useLocalSearchParams } from "expo-router";
import * as Crypto from "expo-crypto";

import { launchTask } from "@/api/client";
import { ErrorNotice, FormField, PrimaryButton, Screen, SecondaryButton, StatusBadge } from "@/components/ui";
import { useTheme } from "@/design/tokens";
import { deleteTaskDraft, loadTaskDraft, saveTaskDraft } from "@/storage/cache";
import { mutationsAllowed, useMobileStore } from "@/state/store";

interface Draft { targetKey: string; profileKey: string; instruction: string; requestId?: string }
const DRAFT_KEY = "new-task";

export default function NewTaskScreen() {
  const { instruction: linkedInstruction } = useLocalSearchParams<{ instruction?: string }>();
  const theme = useTheme();
  const targets = useMobileStore((state) => state.launchTargets);
  const features = useMobileStore((state) => state.features);
  const connection = useMobileStore((state) => state.connection);
  const [draft, setDraft] = useState<Draft>({ targetKey: "", profileKey: "", instruction: "" });
  const [reviewing, setReviewing] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const target = useMemo(() => targets.find((item) => `${item.machine_id}:${item.project_id}` === draft.targetKey), [draft.targetKey, targets]);
  const profile = target?.profiles.find((item) => item.key === draft.profileKey);

  useEffect(() => {
    void loadTaskDraft<Draft>(DRAFT_KEY).then((saved) => {
      const prefill = linkedInstruction?.trim();
      if (prefill && prefill.length <= 4000) setDraft({ ...(saved ?? { targetKey: "", profileKey: "", instruction: "" }), instruction: prefill, requestId: undefined });
      else if (saved) setDraft(saved);
    });
  }, [linkedInstruction]);
  useEffect(() => { const timer = setTimeout(() => void saveTaskDraft(DRAFT_KEY, draft), 250); return () => clearTimeout(timer); }, [draft]);

  const validate = () => {
    if (!target) return "Choose an available project target.";
    if (!profile) return "Choose a supported coding profile.";
    if (!draft.instruction.trim()) return "Describe the coding task.";
    return null;
  };

  const review = () => {
    const message = validate();
    setError(message);
    if (!message) setReviewing(true);
  };

  const submit = async () => {
    const validationError = validate();
    if (validationError || !target || !profile) {
      setError(validationError ?? "The selected target is no longer available.");
      setReviewing(false);
      return;
    }
    if (!features.coding_mutations || !mutationsAllowed()) {
      setError(features.coding_mutations ? "Reconnect and finish synchronizing before submitting." : "Mobile coding actions are disabled by the cluster administrator.");
      return;
    }
    const requestId = draft.requestId ?? Crypto.randomUUID();
    const retainedDraft = { ...draft, requestId };
    setDraft(retainedDraft);
    await saveTaskDraft(DRAFT_KEY, retainedDraft);
    setSubmitting(true);
    setError(null);
    try {
      const response = await launchTask({
        request_id: requestId,
        machine_id: target.machine_id,
        project_id: target.project_id,
        instruction: draft.instruction.trim(),
        profile_key: profile.key,
      });
      await deleteTaskDraft(DRAFT_KEY);
      useMobileStore.getState().markSessionUserInput(response.session_id);
      router.replace({
        pathname: "/(code)/session/[sessionId]",
        params: { sessionId: response.session_id, paneId: String(response.pane_id ?? "") },
      });
    } catch (submissionError) {
      setError(submissionError instanceof Error ? submissionError.message : "Task submission could not be confirmed. Retry to safely reuse the same request identifier.");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Screen>
      <ScrollView contentContainerStyle={styles.content} keyboardShouldPersistTaps="handled">
        <Text accessibilityRole="header" style={[styles.title, { color: theme.text }]}>{reviewing ? "Review task" : "Start coding work"}</Text>
        <Text style={[styles.intro, { color: theme.textMuted }]}>{reviewing ? "Confirm the exact target, terminal profile, and instruction. Nothing is sent until you submit." : "Choose an eligible project and give the agent one clear instruction."}</Text>
        {connection !== "ready" ? <ErrorNotice message="You can edit this encrypted draft offline, but submission requires a fully synchronized connection." /> : null}
        {error ? <ErrorNotice message={error} /> : null}
        {reviewing ? (
          <View style={[styles.review, { backgroundColor: theme.surface, borderColor: theme.border }]}>
            <Text style={[styles.reviewLabel, { color: theme.textMuted }]}>TARGET</Text><Text style={[styles.reviewValue, { color: theme.text }]}>{target?.project_name} · {target?.hostname}</Text>
            <Text style={[styles.reviewLabel, { color: theme.textMuted }]}>PROFILE</Text><Text style={[styles.reviewValue, { color: theme.text }]}>{profile?.label}</Text>
            <Text style={[styles.reviewLabel, { color: theme.textMuted }]}>INSTRUCTION</Text><Text selectable style={[styles.instruction, { color: theme.text }]}>{draft.instruction.trim()}</Text>
          </View>
        ) : (
          <>
            <Text style={[styles.label, { color: theme.text }]}>Project target</Text>
            {targets.length ? targets.map((item) => { const key = `${item.machine_id}:${item.project_id}`; const selected = key === draft.targetKey; return <Pressable accessibilityRole="radio" accessibilityState={{ checked: selected }} key={key} onPress={() => setDraft((current) => ({ ...current, targetKey: key, profileKey: "", requestId: undefined }))} style={[styles.option, { borderColor: selected ? theme.accent : theme.border, backgroundColor: theme.surface }]}><View><Text style={[styles.optionTitle, { color: theme.text }]}>{item.project_name}</Text><Text style={{ color: theme.textMuted }}>{item.hostname} · {item.instance_path}</Text></View><StatusBadge label={item.online ? "Online" : "Offline"} tone={item.online ? "success" : "neutral"} /></Pressable>; }) : <ErrorNotice message="No eligible launch targets are available for this account." />}
            {target ? <><Text style={[styles.label, { color: theme.text }]}>Terminal profile</Text><View style={styles.profileGrid}>{target.profiles.map((item) => { const selected = item.key === draft.profileKey; return <Pressable accessibilityRole="radio" accessibilityState={{ checked: selected }} key={item.key} onPress={() => setDraft((current) => ({ ...current, profileKey: item.key, requestId: undefined }))} style={[styles.profile, { borderColor: selected ? theme.accent : theme.border, backgroundColor: theme.surface }]}><Text style={[styles.optionTitle, { color: theme.text }]}>{item.label}</Text><Text style={{ color: theme.textMuted }}>{item.provider} · {item.mode}</Text></Pressable>; })}</View></> : null}
            <FormField label="Instruction" multiline numberOfLines={7} textAlignVertical="top" placeholder="For example: diagnose the failing login test, implement the fix, and run the relevant suite." value={draft.instruction} onChangeText={(instruction) => setDraft((current) => ({ ...current, instruction, requestId: undefined }))} style={{ minHeight: 140 }} />
          </>
        )}
        {reviewing ? <><PrimaryButton disabled={submitting || !features.coding_mutations || !mutationsAllowed()} onPress={() => void submit()}>{submitting ? "Submitting…" : draft.requestId ? "Retry task safely" : "Submit task"}</PrimaryButton><SecondaryButton disabled={submitting} onPress={() => setReviewing(false)}>Edit task</SecondaryButton></> : <PrimaryButton onPress={review}>Review task</PrimaryButton>}
      </ScrollView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  content: { width: "100%", maxWidth: 680, alignSelf: "center", padding: 20, gap: 14 },
  title: { fontSize: 29, fontWeight: "800" },
  intro: { fontSize: 16, lineHeight: 23, marginBottom: 5 },
  label: { fontSize: 15, fontWeight: "700", marginTop: 6 },
  option: { borderWidth: 1, borderRadius: 14, padding: 14, flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 10 },
  optionTitle: { fontSize: 16, fontWeight: "700" },
  profileGrid: { gap: 9 },
  profile: { borderWidth: 1, borderRadius: 14, padding: 14, gap: 4 },
  review: { borderWidth: 1, borderRadius: 16, padding: 18, gap: 8 },
  reviewLabel: { fontSize: 11, fontWeight: "800", letterSpacing: 1, marginTop: 8 },
  reviewValue: { fontSize: 17, fontWeight: "600" },
  instruction: { fontSize: 16, lineHeight: 23 },
});
