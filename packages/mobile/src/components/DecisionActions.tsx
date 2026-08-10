import { useMemo, useState } from "react";
import { StyleSheet, Text, View } from "react-native";
import type { CodeEvent } from "@apas/protocol";

import { ErrorNotice, FormField, PrimaryButton, SecondaryButton } from "@/components/ui";
import { useTheme } from "@/design/tokens";

export type DecisionResponse = "approve" | "reject" | "answer";

interface DecisionDescriptor {
  prompt?: string;
  options: string[];
  supportsText: boolean;
}

function descriptor(event: CodeEvent): DecisionDescriptor | null {
  const detail = event.detail as Record<string, unknown> | undefined;
  if (!detail) return null;
  if (event.kind === "plan" && detail.type === "plan_review_request") {
    return { prompt: "Allow this planned tool action?", options: [], supportsText: false };
  }
  if (event.kind === "approval" && detail.type === "output") {
    return { prompt: event.summary, options: [], supportsText: false };
  }
  if (event.kind === "question" && detail.type === "tool_use") {
    const input = detail.input as { questions?: { question?: unknown; options?: { label?: unknown }[] }[] } | undefined;
    const question = input?.questions?.[0];
    return {
      prompt: typeof question?.question === "string" ? question.question : "Answer the agent's question",
      options: question?.options?.flatMap((option) => typeof option.label === "string" ? [option.label] : []) ?? [],
      supportsText: true,
    };
  }
  return null;
}

export function DecisionActions({
  event,
  disabled,
  onRespond,
}: {
  event: CodeEvent;
  disabled: boolean;
  onRespond: (response: DecisionResponse, answer?: string) => Promise<void>;
}) {
  const theme = useTheme();
  const decision = useMemo(() => descriptor(event), [event]);
  const [answer, setAnswer] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!decision || !event.requires_attention) return null;

  const respond = async (response: DecisionResponse, value?: string) => {
    setBusy(true);
    setError(null);
    try {
      await onRespond(response, value);
    } catch (responseError) {
      setError(responseError instanceof Error ? responseError.message : "The decision could not be confirmed.");
    } finally {
      setBusy(false);
    }
  };

  return (
    <View style={[styles.container, { borderColor: theme.border, backgroundColor: theme.surface }]}> 
      {decision.prompt ? <Text style={[styles.prompt, { color: theme.text }]}>{decision.prompt}</Text> : null}
      {error ? <ErrorNotice message={error} /> : null}
      {event.kind === "question" ? (
        <>
          {decision.options.length > 0 ? <View style={styles.actions}>{decision.options.map((option) => <SecondaryButton disabled={disabled || busy} key={option} onPress={() => void respond("answer", option)}>{option}</SecondaryButton>)}</View> : null}
          {decision.supportsText ? <FormField label="Answer" value={answer} onChangeText={setAnswer} placeholder="Type a response" /> : null}
          <PrimaryButton disabled={disabled || busy || !answer.trim()} onPress={() => void respond("answer", answer.trim())}>{busy ? "Sending…" : "Send answer"}</PrimaryButton>
        </>
      ) : (
        <View style={styles.actions}>
          <SecondaryButton disabled={disabled || busy} onPress={() => void respond("reject")}>Reject</SecondaryButton>
          <PrimaryButton disabled={disabled || busy} onPress={() => void respond("approve")}>{busy ? "Sending…" : "Approve"}</PrimaryButton>
        </View>
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  container: { borderWidth: 1, borderRadius: 12, padding: 12, gap: 10, marginTop: -5 },
  prompt: { fontSize: 14, fontWeight: "600", lineHeight: 20 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
});
