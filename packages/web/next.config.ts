import type { NextConfig } from "next";
import { execSync } from "node:child_process";

function resolveWebVersion(): string {
  // Highest priority: explicit environment override.
  const envVersion = process.env.NEXT_PUBLIC_WEB_UI_VERSION?.trim();
  if (envVersion) return envVersion;

  // Compute build-time version from git using YY.MM.<commits-this-month>.
  try {
    const now = new Date();
    const yy = String(now.getFullYear()).slice(-2);
    const mm = String(now.getMonth() + 1).padStart(2, "0");
    const monthStart = `${now.getFullYear()}-${mm}-01 00:00:00`;
    const commitCount = execSync(`git rev-list --count --since="${monthStart}" HEAD`, {
      encoding: "utf8",
    }).trim();
    const date = `${yy}.${mm}`;
    if (date && commitCount) return `${date}.${commitCount}`;
  } catch {
    // Ignore and fall through.
  }

  return "00.00.0";
}

const nextConfig: NextConfig = {
  env: {
    NEXT_PUBLIC_WEB_UI_VERSION: resolveWebVersion(),
  },
};

export default nextConfig;
