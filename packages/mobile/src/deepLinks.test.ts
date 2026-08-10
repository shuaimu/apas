import { resolveAuthorizedDeepLink } from "./deepLinks";

const session = "58dbd62d-c40f-4b5b-a1d4-96aca52ea595";

describe("authorized mobile deep links", () => {
  it("accepts app and associated-domain code routes", () => {
    expect(resolveAuthorizedDeepLink("apas://code", new Set())).toEqual({ kind: "home" });
    expect(resolveAuthorizedDeepLink(`https://apas.mpaxos.com/code/session/${session}`, new Set([session]))).toEqual({ kind: "session", sessionId: session });
  });

  it("rejects inaccessible, malformed, and spoofed targets", () => {
    expect(resolveAuthorizedDeepLink(`apas://code/session/${session}`, new Set())).toBeNull();
    expect(resolveAuthorizedDeepLink("https://evil.example/code", new Set())).toBeNull();
    expect(resolveAuthorizedDeepLink("apas://code/session/not-a-uuid", new Set())).toBeNull();
  });

  it("treats a new-task instruction only as bounded review prefill", () => {
    expect(resolveAuthorizedDeepLink("apas://code/new?instruction=Run%20tests", new Set())).toEqual({ kind: "new-task", instruction: "Run tests" });
    expect(resolveAuthorizedDeepLink(`apas://code/new?instruction=${"x".repeat(4001)}`, new Set())).toEqual({ kind: "new-task", instruction: undefined });
  });
});
