/**
 * Validate a `?redirect=` target before navigating to it.
 *
 * A redirect parameter passed through unchecked is an open redirect:
 * `/login?redirect=https://evil.example` would bounce a user who just typed
 * their password onto an attacker's page, from a link that was genuinely ours.
 * So only same-origin *relative* paths are accepted.
 *
 * Rejected, and why each one matters:
 *  - `https://evil.example` — absolute, different origin.
 *  - `//evil.example`       — protocol-relative; browsers read this as
 *                             absolute even though it starts with a slash.
 *  - `/\evil.example`       — some browsers fold `\` to `/`, making this
 *                             protocol-relative too.
 *  - `%2F%2Fevil.example`   — decodes to a protocol-relative URL, so the check
 *                             runs against the decoded form.
 *  - anything not starting with `/` — relative to the current path, which is
 *                             never what a redirect param means.
 *
 * Returns the path when safe, otherwise `null` so the caller falls back to its
 * own default.
 */
export function safeRedirect(raw: string | null | undefined): string | null {
  if (!raw) return null;
  const value = raw.trim();

  // Check the decoded form too: a caller may hand us a still-encoded value,
  // and `%2F%2Fevil.example` is `//evil.example` by the time a browser acts on
  // it. Malformed escapes mean we cannot know what it becomes — reject.
  let decoded = value;
  try {
    decoded = decodeURIComponent(value);
  } catch {
    return null;
  }

  for (const candidate of [value, decoded]) {
    if (!candidate.startsWith("/")) return null;
    if (candidate.startsWith("//")) return null;
    if (candidate.includes("\\")) return null;
    // Control characters — a raw newline or NUL especially — can truncate the
    // URL in some parsers, smuggling a different target past this check.
    if (/[\u0000-\u001f\u007f]/.test(candidate)) return null;
  }

  return value;
}

/**
 * Build a `?redirect=` query fragment for a target path.
 *
 * Encoding is the whole point: `/share?code=ABC` contains its own `?`, so
 * interpolating it raw produced `/login?redirect=/share?code=ABC` — parsed as
 * `redirect=/share` plus a stray top-level `code=ABC`, silently dropping the
 * invite code and stranding the recipient on an empty form.
 */
export function redirectParam(target: string): string {
  return `redirect=${encodeURIComponent(target)}`;
}
