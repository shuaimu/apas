import type { NextConfig } from "next";
import { execSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

function resolveWebVersion(): string {
  // Highest priority: explicit environment override.
  const envVersion = process.env.NEXT_PUBLIC_WEB_UI_VERSION?.trim();
  if (envVersion) return envVersion;

  // Use version generated during Rust build (YY.MM.commit_count).
  const generatedPath = join(process.cwd(), ".apas-version");
  if (existsSync(generatedPath)) {
    const generated = readFileSync(generatedPath, "utf8").trim();
    if (generated) return generated;
  }

  // Fallback for local dev in repo root where git metadata is available.
  try {
    const date = execSync("date +%y.%m", { encoding: "utf8" }).trim();
    const commitCount = execSync("git rev-list --count HEAD", { encoding: "utf8" }).trim();
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
