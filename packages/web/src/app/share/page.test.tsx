import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SharePage from "./page";

const navigation = vi.hoisted(() => ({
  code: null as string | null,
  push: vi.fn(),
  searchParams: {
    get: vi.fn((key: string) => (key === "code" ? navigation.code : null)),
  },
}));

vi.mock("next/navigation", () => ({
  useRouter: () => ({ push: navigation.push }),
  useSearchParams: () => navigation.searchParams,
}));

const originalFetch = globalThis.fetch;
const fetchMock = vi.fn();

function codeInput(): HTMLInputElement {
  return screen.getByLabelText("Invitation Code") as HTMLInputElement;
}

function submitButton(): HTMLButtonElement {
  return screen.getByRole("button", { name: "Redeem Code" }) as HTMLButtonElement;
}

function mockRedeemResponse(body: unknown, ok = true) {
  fetchMock.mockResolvedValue({
    ok,
    json: vi.fn().mockResolvedValue(body),
  });
}

describe("SharePage", () => {
  beforeEach(() => {
    navigation.code = null;
    navigation.push.mockReset();
    navigation.searchParams.get.mockClear();
    fetchMock.mockReset();
    localStorage.clear();
    globalThis.fetch = fetchMock as unknown as typeof fetch;
    vi.useRealTimers();
  });

  afterEach(() => {
    vi.restoreAllMocks();
    vi.useRealTimers();
    globalThis.fetch = originalFetch;
    document.body.innerHTML = "";
    localStorage.clear();
  });

  it("auto-redeems URL codes for authenticated users and redirects after success", async () => {
    const originalSetTimeout = globalThis.setTimeout;
    const setTimeoutSpy = vi
      .spyOn(globalThis, "setTimeout")
      .mockImplementation((callback, timeout, ...args) => {
        if (timeout === 2000) {
          callback(...args);
          return 0 as unknown as ReturnType<typeof setTimeout>;
        }
        return originalSetTimeout(callback, timeout, ...args);
      });
    navigation.code = "INVITE42";
    localStorage.setItem("apas_token", "token-123");
    mockRedeemResponse({ success: true, session_id: "session-1" });

    render(<SharePage />);

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledTimes(1);
    });
    expect(fetchMock).toHaveBeenCalledWith("http://apas.mpaxos.com/share/redeem", {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: "Bearer token-123",
      },
      body: JSON.stringify({ code: "INVITE42" }),
    });
    expect(await screen.findByText("Session Shared!")).toBeTruthy();
    expect(setTimeoutSpy.mock.calls.some(([, timeout]) => timeout === 2000)).toBe(true);
    expect(navigation.push).toHaveBeenCalledWith("/");
  });

  it("redirects unauthenticated URL-code users to login with the return URL preserved", async () => {
    navigation.code = "GUEST123";

    render(<SharePage />);

    await waitFor(() => {
      expect(navigation.push).toHaveBeenCalledWith(
        "/login?redirect=%2Fshare%3Fcode%3DGUEST123",
      );
    });

    // The name of this test says "preserved", so assert that rather than the
    // literal string: the previous unencoded form ("/login?redirect=/share?
    // code=GUEST123") parsed as redirect=/share plus a stray top-level
    // code=GUEST123, so the invite code never reached the login page and the
    // recipient was stranded on the dashboard.
    const pushed = (navigation.push as unknown as { mock: { calls: string[][] } })
      .mock.calls[0][0];
    const url = new URL(pushed, "http://x.test");
    expect(url.searchParams.get("redirect")).toBe("/share?code=GUEST123");
    expect(url.searchParams.get("code")).toBeNull();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("uppercases manual entries and keeps submit disabled until the code is valid", () => {
    render(<SharePage />);

    expect(submitButton().disabled).toBe(true);

    fireEvent.change(codeInput(), { target: { value: "ab12cd3" } });
    expect(codeInput().value).toBe("AB12CD3");
    expect(submitButton().disabled).toBe(true);

    fireEvent.change(codeInput(), { target: { value: "ab12cd34" } });
    expect(codeInput().value).toBe("AB12CD34");
    expect(submitButton().disabled).toBe(false);
  });

  it("renders server errors from manual redemption attempts", async () => {
    localStorage.setItem("apas_token", "token-123");
    mockRedeemResponse({ message: "Invitation expired" }, false);

    render(<SharePage />);
    fireEvent.change(codeInput(), { target: { value: "error123" } });
    fireEvent.click(submitButton());

    await screen.findByText("Invitation expired");
    expect(fetchMock).toHaveBeenCalledWith(
      "http://apas.mpaxos.com/share/redeem",
      expect.objectContaining({
        body: JSON.stringify({ code: "ERROR123" }),
      }),
    );
  });

  it("renders fallback copy when redemption fails with a non-Error rejection", async () => {
    localStorage.setItem("apas_token", "token-123");
    fetchMock.mockRejectedValue("offline");

    render(<SharePage />);
    fireEvent.change(codeInput(), { target: { value: "offline1" } });
    fireEvent.click(submitButton());

    expect(await screen.findByText("Failed to redeem code")).toBeTruthy();
  });
});
