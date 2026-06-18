export const MINIMAX_DEFAULT_MODEL = "MiniMax-M2.7";
export const GLM_DEFAULT_MODEL = "glm-5.1";
// Keep in sync with crates/client-cli/src/mode/dual_pane.rs; the apas
// cargo test `deepseek_default_model_matches_web_provider_options` guards drift.
export const DEEPSEEK_DEFAULT_MODEL = "deepseek-v4-pro";
export const CLAUDE_FABLE_MODEL = "claude-fable-5";

export interface ProviderModelOption {
  value: string;
  label: string;
  provider: string;
  model?: string;
}

export interface ProviderModelGroup {
  id: string;
  label: string;
  iconProvider: string;
  iconModel?: string;
  toneClass: string;
  options: ProviderModelOption[];
}

export const PROVIDER_MODEL_GROUPS: ProviderModelGroup[] = [
  {
    id: "claude",
    label: "Claude",
    iconProvider: "claude",
    toneClass: "text-blue-500",
    options: [
      { value: "claude/official", label: "Official", provider: "claude" },
      {
        value: "claude/fable",
        label: "Fable",
        provider: "claude",
        model: CLAUDE_FABLE_MODEL,
      },
      {
        value: "claude/minimax",
        label: "MiniMax 2.7",
        provider: "claude",
        model: MINIMAX_DEFAULT_MODEL,
      },
      {
        value: "claude/glm",
        label: "GLM 5.1",
        provider: "claude",
        model: GLM_DEFAULT_MODEL,
      },
      {
        value: "claude/deepseek",
        label: "DeepSeek",
        provider: "claude",
        model: DEEPSEEK_DEFAULT_MODEL,
      },
    ],
  },
  {
    id: "codex",
    label: "Codex",
    iconProvider: "codex",
    toneClass: "text-green-500",
    options: [{ value: "codex/official", label: "Official", provider: "codex" }],
  },
  {
    id: "opencode",
    label: "OpenCode",
    iconProvider: "opencode",
    toneClass: "text-orange-500",
    options: [
      { value: "opencode/official", label: "OpenCode", provider: "opencode" },
    ],
  },
  {
    id: "cursor-agent",
    label: "Cursor",
    iconProvider: "cursor-agent",
    toneClass: "text-sky-500",
    options: [
      {
        value: "cursor-agent/official",
        label: "Cursor",
        provider: "cursor-agent",
      },
    ],
  },
];

export const PROVIDER_MODEL_OPTIONS: ProviderModelOption[] =
  PROVIDER_MODEL_GROUPS.flatMap((group) =>
    group.options.map((option) => ({
      ...option,
      label:
        group.options.length === 1 && option.label === group.label
          ? group.label
          : `${group.label} / ${option.label}`,
    })),
  );

export const DEFAULT_PROVIDER_MODEL_OPTION = PROVIDER_MODEL_OPTIONS[0];

export function isMiniMaxModel(model?: string | null): boolean {
  if (typeof model !== "string") return false;
  const normalized = model.trim().toLowerCase();
  return normalized.includes("minimax") || normalized.startsWith("m2");
}

export function isGlmModel(model?: string | null): boolean {
  if (typeof model !== "string") return false;
  const normalized = model.trim().toLowerCase();
  return normalized.startsWith("glm") || normalized.includes("glm-");
}

export function isDeepseekModel(model?: string | null): boolean {
  if (typeof model !== "string") return false;
  const normalized = model.trim().toLowerCase();
  return normalized.includes("deepseek");
}

export function isFableModel(model?: string | null): boolean {
  if (typeof model !== "string") return false;
  const normalized = model.trim().toLowerCase();
  return normalized.includes("fable");
}

export function providerModelValue(
  provider?: string | null,
  model?: string | null,
): string {
  if (provider === "claude") {
    if (isFableModel(model)) return "claude/fable";
    if (isMiniMaxModel(model)) return "claude/minimax";
    if (isGlmModel(model)) return "claude/glm";
    if (isDeepseekModel(model)) return "claude/deepseek";
    return "claude/official";
  }
  if (provider === "codex") return "codex/official";
  if (provider === "opencode") return "opencode/official";
  if (provider === "cursor-agent") return "cursor-agent/official";
  if (provider === "minimax") return "claude/minimax";
  if (provider === "glm") return "claude/glm";
  if (provider === "deepseek") return "claude/deepseek";
  if (provider === "fable") return "claude/fable";
  return DEFAULT_PROVIDER_MODEL_OPTION.value;
}

export function findProviderModelOption(value: string): ProviderModelOption {
  return (
    PROVIDER_MODEL_OPTIONS.find((option) => option.value === value) ??
    DEFAULT_PROVIDER_MODEL_OPTION
  );
}
