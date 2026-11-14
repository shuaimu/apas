import { launchProfileKey, launchProfileKeyAliases } from "@/lib/store";
import type { PaneKind, Provider } from "@/lib/store";
import { PROVIDER_MODEL_GROUPS, PROVIDER_MODEL_OPTIONS, providerModelValue } from "@/lib/providerOptions";

/**
 * Readable names for launch-profile keys such as
 * `terminal:claude:deepseek:deepseek-flash`.
 *
 * Derived from the provider catalog, not listed here: a profile the catalog
 * gains or loses changes its name here with it. A key the catalog does not
 * know (a provider this build no longer offers, or one a newer server added)
 * keeps its raw key as its name, so nothing a policy allows is ever hidden.
 */
export interface LaunchProfileLabel {
  key: string;
  label: string;
  /** False when the catalog has no name for this key and `label` is the key. */
  known: boolean;
}

// New tabs are terminal tabs. A structured agent profile is only ever a
// leftover in an older policy, so it is the one that carries a qualifier.
const KINDS: Array<{ kind: PaneKind; suffix: string }> = [
  { kind: "terminal", suffix: "" },
  { kind: "agent", suffix: " (structured agent)" },
];

function catalogLabels(): Map<string, string> {
  const labels = new Map<string, string>();
  for (const { kind, suffix } of KINDS) {
    for (const option of PROVIDER_MODEL_OPTIONS) {
      if (option.supported === false) continue;
      const key = launchProfileKey(kind, option.provider as Provider, option.model);
      // The same aliases the policy check accepts, so a policy written before
      // a model was renamed still reads as that model.
      for (const alias of launchProfileKeyAliases(key)) {
        if (!labels.has(alias)) labels.set(alias, `${option.label}${suffix}`);
      }
    }
  }
  return labels;
}

let cached: Map<string, string> | undefined;

export function launchProfileLabel(key: string): LaunchProfileLabel {
  cached ??= catalogLabels();
  const label = cached.get(key.trim().toLowerCase());
  return label ? { key, label, known: true } : { key, label: key, known: false };
}

// Providers that usage, limits and old rosters still name although the catalog
// has no group for them. DeepSeek is offered as a Claude model and reports
// under its own name. Pi was retired, but a tab of it can stay in a roster as
// read-only history, and its name must not degrade to the wire value "pi".
const UNLISTED_PROVIDER_NAMES: Record<string, string> = {
  deepseek: "DeepSeek",
  pi: "Pi",
};

/** A provider's name as the catalog spells it: "Claude", "OMP". A provider
 *  neither the catalog nor the list above knows keeps the name it came with. */
export function providerDisplayName(provider: string): string {
  return PROVIDER_MODEL_GROUPS.find((group) => group.iconProvider === provider)?.label
    ?? UNLISTED_PROVIDER_NAMES[provider.trim().toLowerCase()]
    ?? provider;
}

/**
 * What a tab runs, for a label: "Claude", "Claude / DeepSeek 4.1 Flash",
 * "OMP". The provider's own name stands for its ordinary model. Null for a
 * combination the catalog does not offer.
 */
export function providerModelDisplayName(provider?: string | null, model?: string | null): string | null {
  const value = providerModelValue(provider, model);
  for (const group of PROVIDER_MODEL_GROUPS) {
    const option = group.options.find((candidate) => candidate.value === value);
    if (!option) continue;
    return option.model ? `${group.label} / ${option.label}` : group.label;
  }
  return null;
}
