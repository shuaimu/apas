import { useStore } from "@/lib/store";
import type { PaneKind } from "@/lib/store";

/**
 * A "tab type" is a pane kind plus a provider — the unit the add-tab menu
 * offers and a project owner restricts. Claude *agent* and Claude *terminal*
 * are different capabilities (the terminal runs the real TUI with permission
 * prompts bypassed), so neither half alone identifies a type.
 *
 * Keys must match `shared::tab_type_key` exactly: `<kind>:<provider>`, with
 * the providers spelled as their serde names.
 */
export function tabTypeKey(kind: PaneKind, provider: string): string {
  return `${kind}:${provider}`;
}

export interface TabTypeOption {
  key: string;
  kind: PaneKind;
  provider: string;
  label: string;
}

/**
 * Every tab type, in menu order. Must stay in step with `shared::all_tab_types`
 * — a Rust test reads this file and asserts the two agree.
 *
 * Deliberately not every provider: MiniMax, GLM and DeepSeek are the claude
 * binary against a different backend, so `PROVIDER_MODEL_GROUPS` offers them
 * as claude *models*. Listing them here would give an admin a checkbox that
 * does nothing, since those tabs arrive as `provider: claude`.
 *
 * Terminal exists only for claude and codex, mirroring
 * `terminal_pane::terminal_binary_for` in the CLI.
 */
export const ALL_TAB_TYPES: TabTypeOption[] = (
  [
    { kind: "agent", provider: "claude", label: "Claude" },
    { kind: "agent", provider: "codex", label: "Codex" },
    { kind: "agent", provider: "opencode", label: "OpenCode" },
    { kind: "agent", provider: "cursor-agent", label: "Cursor" },
    { kind: "terminal", provider: "claude", label: "Claude terminal" },
    { kind: "terminal", provider: "codex", label: "Codex terminal" },
  ] as Omit<TabTypeOption, "key">[]
).map((t) => ({ ...t, key: tabTypeKey(t.kind, t.provider) }));

/**
 * The current project's deny list. Empty when unknown, so a project whose CLI
 * has not reported flags yet is permissive rather than locked down — the CLI
 * re-checks `.apas` on every `AddPane` and is what actually enforces this.
 */
export function useDisallowedTabTypes(): string[] {
  const sessionId = useStore((s) => s.sessionId);
  const projectFlags = useStore((s) => s.projectFlags);
  if (!sessionId) return [];
  return projectFlags[sessionId]?.disallowedTabTypes ?? [];
}

/** Normalized deny list from a possibly-stale flags object. */
export function disallowedFrom(flags: { disallowedTabTypes?: string[] }): string[] {
  return flags.disallowedTabTypes ?? [];
}

/**
 * Case-insensitive membership, matching the CLI's comparison.
 *
 * Tolerates a nullish list: a flags object cached before this field existed —
 * or produced by an optimistic update that spread an older one — must read as
 * "no restrictions", not throw. Getting this wrong took the whole Overview
 * down, because the card renders above everything else on the page.
 */
export function isTabTypeAllowed(
  disallowed: string[] | undefined | null,
  kind: PaneKind,
  provider: string,
): boolean {
  if (!disallowed || disallowed.length === 0) return true;
  const key = tabTypeKey(kind, provider).toLowerCase();
  return !disallowed.some((d) => d.trim().toLowerCase() === key);
}

/** Hook form for the current project. */
export function useIsTabTypeAllowed(): (kind: PaneKind, provider: string) => boolean {
  const disallowed = useDisallowedTabTypes();
  return (kind, provider) => isTabTypeAllowed(disallowed, kind, provider);
}
