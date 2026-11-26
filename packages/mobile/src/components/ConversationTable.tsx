import { ScrollView, StyleSheet, Text, View } from "react-native";
import { marked, type Token, type Tokens } from "marked";
import type { CodeEvent } from "@apas/protocol";

import { useTheme } from "@/design/tokens";

function assistantText(event: CodeEvent): string | undefined {
  if (event.kind !== "agent_status" || !event.detail || typeof event.detail !== "object") return;
  const detail = event.detail;
  if ("role" in detail && detail.role === "assistant" && "content" in detail && typeof detail.content === "string") {
    return detail.content;
  }
  if (!("type" in detail) || detail.type !== "assistant" || !("message" in detail)) return;
  const message = detail.message;
  if (!message || typeof message !== "object" || !("content" in message) || !Array.isArray(message.content)) return;
  return message.content.filter((block) => block?.type === "text" && typeof block.text === "string")
    .map((block) => block.text).join("\n\n");
}

function inlineText(tokens: Token[]): string {
  return tokens.map((token) => {
    if (token.type === "br") return "\n";
    if ("tokens" in token && Array.isArray(token.tokens)) return inlineText(token.tokens);
    return "text" in token && typeof token.text === "string" ? token.text : token.raw;
  }).join("");
}

function Table({ table }: { table: Tokens.Table }) {
  const theme = useTheme();
  const rows = [table.header, ...table.rows].map((row) => row.map((cell) => inlineText(cell.tokens)));
  const widths = table.header.map((_, index) =>
    rows.reduce((width, row) => Math.max(width, Math.min(220, (row[index]?.length ?? 0) * 8 + 20)), 80));
  return (
    <ScrollView horizontal style={{ maxWidth: "100%" }} accessibilityLabel="Conversation table">
      <View>
        {rows.map((row, r) => (
          <View key={r} style={{ flexDirection: "row", backgroundColor: r === 0 ? theme.surfaceMuted : theme.surface }}>
            {row.map((cell, c) => (
              <Text key={c} selectable accessibilityRole={r === 0 ? "header" : undefined}
                style={{ width: widths[c], padding: 8, borderWidth: StyleSheet.hairlineWidth, borderColor: theme.border,
                  color: theme.text, fontWeight: r === 0 ? "700" : "400", textAlign: table.align[c] ?? "left" }}>
                {cell}
              </Text>
            ))}
          </View>
        ))}
      </View>
    </ScrollView>
  );
}

export function conversationTableBlocks(event: CodeEvent) {
  const text = assistantText(event);
  if (!text?.includes("|")) return;
  const blocks = marked.lexer(text);
  if (blocks.some((block) => block.type === "table")) return blocks;
}

export function ConversationTableReply({ blocks }: { blocks: Token[] }) {
  const theme = useTheme();
  return blocks.map((block, index) => block.type === "table"
    ? <Table key={index} table={block as Tokens.Table} />
    : block.raw.trim() ? <Text key={index} selectable style={{ color: theme.text }}>{block.raw.trim()}</Text> : null);
}
