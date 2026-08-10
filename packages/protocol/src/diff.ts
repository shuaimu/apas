export interface UnifiedDiffFile {
  path: string;
  content: string;
}

export interface UnifiedDiffResult {
  files: UnifiedDiffFile[];
  truncated: boolean;
  error: string | null;
}

const MAX_DIFF_CHARACTERS = 200_000;

export function splitUnifiedDiff(value: unknown, maximum = MAX_DIFF_CHARACTERS): UnifiedDiffResult {
  if (typeof value !== "string") return { files: [], truncated: false, error: "Diff content is unavailable." };
  const truncated = value.length > maximum;
  const content = truncated ? value.slice(0, maximum) : value;
  const lines = content.split("\n");
  const files: UnifiedDiffFile[] = [];
  let current: string[] = [];
  let path = "Patch";
  const flush = () => {
    if (current.length) files.push({ path, content: current.join("\n") });
    current = [];
  };
  for (const line of lines) {
    if (line.startsWith("diff --git ")) {
      flush();
      const match = line.match(/^diff --git a\/(.+) b\/(.+)$/);
      path = match?.[2] ?? line.slice("diff --git ".length);
    }
    current.push(line);
  }
  flush();
  return {
    files,
    truncated,
    error: files.length ? null : "The server returned an empty diff.",
  };
}
