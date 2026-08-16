/**
 * Deciding which machines are behind, in one place.
 *
 * The two machine surfaces must never disagree about the same machine, and both
 * must agree with the daemon itself: the CLI parses `YY.MM.COMMIT` into a
 * numeric triple and refuses to act on anything it cannot read, rather than
 * guessing and risking a downgrade. This mirrors that ordering and that refusal.
 */

export interface ReleaseVersion {
  year: number;
  month: number;
  build: number;
}

/**
 * Parse a `YY.MM.COMMIT` release version, or return null.
 *
 * Null is the honest answer for anything else, and callers treat it as unknown
 * rather than as old — an unreadable version is not evidence of being behind.
 */
export function parseReleaseVersion(
  value: string | null | undefined,
): ReleaseVersion | null {
  if (typeof value !== "string") return null;
  const parts = value.trim().split(".");
  if (parts.length !== 3) return null;
  const numbers = parts.map((part) =>
    // Reject anything that is not purely digits. `Number("26 ")` and
    // `Number("2e1")` both parse, and neither is a release version.
    /^\d+$/.test(part) ? Number(part) : Number.NaN,
  );
  if (numbers.some((n) => !Number.isSafeInteger(n))) return null;
  const [year, month, build] = numbers;
  return { year, month, build };
}

/** Order by release, not by text: `26.08.9` precedes `26.08.10`. */
export function compareReleaseVersions(
  a: ReleaseVersion,
  b: ReleaseVersion,
): number {
  return a.year - b.year || a.month - b.month || a.build - b.build;
}

/**
 * The newest version the client can already see.
 *
 * Both sources are needed. The server's version alone misses a rollout in
 * progress, since a CLI installed from source is routinely newer than the
 * deployed server — which is the very window this is for. The machines alone
 * miss a fleet that is uniformly behind a newer deployment, because nothing any
 * of them can see is newer.
 *
 * Unreadable versions are dropped rather than parsed leniently, so one garbled
 * report cannot push the maximum up and mark every other machine behind.
 */
export function latestSeenVersion(
  candidates: Array<string | null | undefined>,
): ReleaseVersion | null {
  let latest: ReleaseVersion | null = null;
  for (const candidate of candidates) {
    const parsed = parseReleaseVersion(candidate);
    if (!parsed) continue;
    if (!latest || compareReleaseVersions(parsed, latest) > 0) latest = parsed;
  }
  return latest;
}

/**
 * Whether restarting this machine is *known* to also update it.
 *
 * Only ever a statement about what is known. A restart applies whatever update
 * it finds, so false does not promise the machine will come back unchanged — it
 * says only that nothing the client can see is newer.
 */
export function isMachineBehind(
  machineVersion: string | null | undefined,
  latest: ReleaseVersion | null,
): boolean {
  const current = parseReleaseVersion(machineVersion);
  if (!current || !latest) return false;
  return compareReleaseVersions(current, latest) < 0;
}

/**
 * The label for a machine's restart control.
 *
 * The plain case deliberately does not say "no update available": a restart
 * still tries, so that would be a claim the client cannot support.
 */
export function rebootLabelFor(behind: boolean): string {
  return behind ? "Reboot to update" : "Reboot";
}

/**
 * The same decision worded for a screen reader or a confirmation title, where
 * the machine has to be named. Shared so the two surfaces cannot describe one
 * machine differently.
 */
export function rebootActionLabelFor(behind: boolean, hostname: string): string {
  return behind
    ? `Reboot and update the daemon on ${hostname}`
    : `Reboot the daemon on ${hostname}`;
}

/** How a machine's version reads in its row, including when it reports none. */
export function daemonVersionLabel(value: string | null | undefined): string {
  const trimmed = typeof value === "string" ? value.trim() : "";
  return trimmed.length > 0 ? trimmed : "version unknown";
}
