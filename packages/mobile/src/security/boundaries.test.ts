import { readFileSync } from "node:fs";
import { resolve } from "node:path";

describe("mobile security boundaries", () => {
  it("keeps credentials out of AsyncStorage", () => {
    const source = readFileSync(resolve(__dirname, "credentials.ts"), "utf8");
    expect(source).toContain("expo-secure-store");
    expect(source).not.toContain("AsyncStorage");
  });

  it("does not place credentials in terminal route payloads", () => {
    const source = readFileSync(
      resolve(__dirname, "../../app/(code)/session/[sessionId]/terminal.tsx"),
      "utf8",
    );
    expect(source).not.toMatch(/accessToken|refreshToken|access_token|refresh_token|Authorization/);
  });
});
